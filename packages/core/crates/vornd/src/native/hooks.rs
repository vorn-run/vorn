//! The endpoint agents' hooks post to, and what follows from each event
//! (`hook-server`, `hookStatusMapper`).
//!
//! It listens on loopback, on a fixed port when it is free so the agents'
//! settings stay the same across restarts, and answers only requests that
//! carry its token. One running Vorn registers it ([`vorn_hooks::owner`]):
//! that one writes the port and token beside the owner record in `~/.vorn`
//! and its entries in Claude Code's settings; another waits for the
//! registration to come free. An event sets the status of the terminal it
//! is about, from then on taken from hooks rather than the screen, and links
//! the terminal to the agent's conversation. A permission request is held
//! open until a client answers it (`permission:resolve`), the agent moves
//! on, or the agent stops waiting.
//!
//! Each new terminal is also followed from the registry's notes: a Copilot
//! terminal gets the hooks file Copilot reads, and an agent that cannot be
//! told its conversation has it read from the agent's own database a few
//! times over its first forty seconds ([`vorn_hooks::capture`]).

use std::collections::HashSet;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use vorn_hooks::mapper::{normalize, Status};
use vorn_hooks::owner::{may_claim, may_release, Owner};
use vorn_hooks::{claude, copilot, permission, Event, Mapper, Terminal};

use super::{Answer, Native};
use crate::endpoint::{full, Body};
use crate::registry::{HookStatus, Patch, TerminalSession};

/// Every call this module answers.
pub const METHODS: &[&str] = &["permission:resolve", "permission:resolve-top"];

/// Tried first, so the settings entry naming it stays the same across restarts.
const PREFERRED_PORT: u16 = 56432;
const MAX_BODY: usize = 1024 * 1024;
/// How often a Vorn that found the registration held looks again.
const OWNER_POLL: Duration = Duration::from_secs(1);
/// When an agent's own database is read for its conversation, after the last read.
const CAPTURE_LADDER: [u64; 4] = [5, 5, 10, 20];

/// A permission request held open.
#[derive(Debug)]
struct Pending {
    id: String,
    session: String,
    reply: oneshot::Sender<String>,
}

/// The user's directories the endpoint's files go in.
#[derive(Clone, Debug)]
pub struct Homes {
    /// `~/.vorn`: the port, token and owner record.
    pub vorn: PathBuf,
    /// `~/.claude/settings.json`.
    pub claude_settings: PathBuf,
    /// Copilot's hooks file.
    pub copilot_hooks: PathBuf,
    /// The user's home, for the agents' databases.
    pub home: PathBuf,
}

impl Homes {
    /// From `HOME` (`USERPROFILE` on Windows) and `COPILOT_HOME`.
    pub fn from_env() -> Option<Homes> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)?;
        let copilot_home = std::env::var("COPILOT_HOME").ok();
        Some(Homes {
            vorn: home.join(".vorn"),
            claude_settings: home.join(".claude").join("settings.json"),
            copilot_hooks: copilot::hooks_file(copilot_home.as_deref(), &home),
            home,
        })
    }
}

/// The hook endpoint and everything it keeps.
#[derive(Debug)]
pub struct Hooks {
    native: Weak<Native>,
    homes: Homes,
    token: String,
    port: u16,
    /// Whether this vornd holds the registration.
    owner: AtomicBool,
    /// Whether it wrote Copilot's hooks file.
    copilot: AtomicBool,
    /// Set once vornd stops: a registration that comes free then is not taken.
    stopped: AtomicBool,
    pending: Mutex<Vec<Pending>>,
    mapper: Mutex<Mapper>,
    last_activity: Mutex<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Hooks {
    /// Listens, claims the registration when it is free (else waits for it),
    /// and follows new terminals.
    pub async fn start(native: &Arc<Native>, homes: Homes) -> std::io::Result<Arc<Hooks>> {
        Hooks::start_on(native, homes, PREFERRED_PORT).await
    }

    pub async fn start_on(
        native: &Arc<Native>,
        homes: Homes,
        port: u16,
    ) -> std::io::Result<Arc<Hooks>> {
        let loopback = std::net::Ipv4Addr::LOCALHOST;
        let listener = match TcpListener::bind((loopback, port)).await {
            Ok(l) => l,
            Err(err) => {
                info!(port, %err, "the hook port is taken; listening on another");
                TcpListener::bind((loopback, 0)).await?
            }
        };
        let hooks = Arc::new(Hooks {
            native: Arc::downgrade(native),
            homes,
            token: uuid::Uuid::new_v4().to_string(),
            port: listener.local_addr()?.port(),
            owner: AtomicBool::new(false),
            copilot: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            pending: Mutex::default(),
            mapper: Mutex::default(),
            last_activity: Mutex::new(Instant::now()),
        });
        tokio::spawn(serve(Arc::downgrade(&hooks), listener));
        if !hooks.try_claim() {
            let owner = Owner::read(&hooks.owner_file());
            info!(
                ?owner,
                port = hooks.port,
                "another Vorn holds the hook registration; listening without it"
            );
            let weak = Arc::downgrade(&hooks);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(OWNER_POLL).await;
                    let Some(hooks) = weak.upgrade() else { return };
                    if hooks.stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    if hooks.try_claim() {
                        info!(
                            port = hooks.port,
                            "the hook registration came free; claimed it"
                        );
                        return;
                    }
                }
            });
        }
        if let Some(registry) = native.registry.get() {
            tokio::spawn(follow(Arc::downgrade(&hooks), registry.subscribe()));
        }
        Ok(hooks)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn owns_registration(&self) -> bool {
        self.owner.load(Ordering::SeqCst)
    }

    fn owner_file(&self) -> PathBuf {
        self.homes.vorn.join("hook-owner")
    }

    /// Claims the registration when nobody live holds it; whether this vornd holds it.
    fn try_claim(&self) -> bool {
        if self.owns_registration() {
            return true;
        }
        if self.stopped.load(Ordering::SeqCst) {
            return false;
        }
        let me = std::process::id();
        if !may_claim(
            Owner::read(&self.owner_file()),
            me,
            vorn_sessiond::launch::alive,
        ) {
            return false;
        }
        let written = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.homes.vorn)?;
            std::fs::write(
                self.owner_file(),
                Owner {
                    port: self.port,
                    pid: me,
                }
                .to_json(),
            )?;
            std::fs::write(self.homes.vorn.join("port"), self.port.to_string())?;
            write_private(&self.homes.vorn.join("token"), &self.token)?;
            claude::install_file(&self.homes.claude_settings, self.port, &self.token)
        })();
        if let Err(err) = written {
            warn!(%err, "could not register the hook endpoint");
            return false;
        }
        self.owner.store(true, Ordering::SeqCst);
        true
    }

    /// Denies what is pending and gives the registration up, when it is still this vornd's.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        for p in lock(&self.pending).drain(..) {
            let _ = p.reply.send(permission::decision(false, None, None));
        }
        if self.copilot.swap(false, Ordering::SeqCst) {
            if let Err(err) = copilot::uninstall(&self.homes.copilot_hooks) {
                debug!(%err, "could not remove Copilot's hooks file");
            }
        }
        if !self.owner.swap(false, Ordering::SeqCst) {
            return;
        }
        let owner = Owner::read(&self.owner_file());
        if !may_release(owner, std::process::id(), true) {
            info!(
                ?owner,
                "the hook registration moved on; left the shared files alone"
            );
            return;
        }
        for name in ["port", "token", "hook-owner"] {
            let _ = std::fs::remove_file(self.homes.vorn.join(name));
        }
        if let Err(err) = claude::uninstall_file(&self.homes.claude_settings) {
            debug!(%err, "could not remove the hooks from Claude's settings");
        }
    }

    /// How long since a hook posted, and how many requests are open.
    pub fn activity(&self) -> Value {
        json!({
            "port": self.port,
            "owner": self.owns_registration(),
            "msSinceActivity": lock(&self.last_activity).elapsed().as_millis() as u64,
            "pendingPermissions": lock(&self.pending).len(),
        })
    }

    fn terminals(native: &Native) -> Vec<Terminal> {
        let Some(registry) = native.registry.get() else {
            return Vec::new();
        };
        registry
            .read(|r| r.terminals().into_iter().map(terminal_view).collect())
            .unwrap_or_default()
    }

    /// An event other than a permission request: answered at once, then acted on.
    fn heard(self: &Arc<Self>, event: Event) {
        let Some(native) = self.native.upgrade() else {
            return;
        };
        info!(
            event = event.name(),
            session = event.session(),
            terminal = event.terminal(),
            "a hook event"
        );
        let terminals = Hooks::terminals(&native);
        let mapped = lock(&self.mapper).map(&event, &terminals);
        if let Some((resolved, status)) = mapped {
            if let Some(link) = &resolved.link {
                patch(
                    &native,
                    &resolved.terminal,
                    json!({ "hookSessionId": link }),
                );
            }
            hook_status(&native, &resolved.terminal, Some(status), true);
            if event.name() == "SessionStart" {
                let (n, terminal, session) = (
                    Arc::clone(&native),
                    resolved.terminal.clone(),
                    event.session().to_owned(),
                );
                tokio::spawn(async move { remember_on_task(&n, &terminal, &session).await });
            }
        }
        if event.dismisses_permissions() {
            self.cancel_session(&native, event.session());
        }
    }

    /// A permission request: its id while it is open, and the receiver of the hook's answer.
    fn asked(self: &Arc<Self>, event: Event) -> (Option<String>, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        let Some(native) = self.native.upgrade() else {
            let _ = tx.send("{}".to_owned());
            return (None, rx);
        };
        let terminals = Hooks::terminals(&native);
        let resolved = lock(&self.mapper).resolve(&event, &terminals);
        let Some(resolved) = resolved else {
            info!(
                session = event.session(),
                "a permission request names no terminal; the agent decides"
            );
            let _ = tx.send("{}".to_owned());
            return (None, rx);
        };
        if let Some(link) = &resolved.link {
            patch(
                &native,
                &resolved.terminal,
                json!({ "hookSessionId": link }),
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        lock(&self.pending).push(Pending {
            id: id.clone(),
            session: event.session().to_owned(),
            reply: tx,
        });
        hook_status(&native, &resolved.terminal, None, true);
        let about = about(&native, &resolved.terminal);
        info!(request = %id, tool = event.tool_name(), terminal = %resolved.terminal, "a permission request");
        native.broadcast(
            "widget:permission-request",
            permission::info(&id, &event, &about),
        );
        hook_status(&native, &resolved.terminal, Some(Status::Waiting), false);
        (Some(id), rx)
    }

    /// A request whose agent stopped waiting: clients are told it is gone.
    fn abandoned(&self, id: &str) {
        let mut pending = lock(&self.pending);
        let Some(at) = pending.iter().position(|p| p.id == id) else {
            return;
        };
        pending.remove(at);
        drop(pending);
        if let Some(native) = self.native.upgrade() {
            native.broadcast("widget:permission-cancelled", json!(id));
        }
    }

    fn cancel_session(&self, native: &Native, session: &str) {
        let gone: Vec<Pending> = {
            let mut pending = lock(&self.pending);
            let (gone, kept) = pending.drain(..).partition(|p| p.session == session);
            *pending = kept;
            gone
        };
        for p in gone {
            info!(request = %p.id, session, "a permission request the agent moved past");
            let _ = p.reply.send("{}".to_owned());
            native.broadcast("widget:permission-cancelled", json!(p.id));
        }
    }

    /// `permission:resolve`, or the oldest request with `top`.
    fn resolve(&self, params: &Value, top: bool) {
        let allow = params
            .get("allow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut pending = lock(&self.pending);
        let at = if top {
            (!pending.is_empty()).then_some(0)
        } else {
            let id = params
                .get("requestId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            pending.iter().position(|p| p.id == id)
        };
        let Some(at) = at else {
            info!("a permission answer for a request no longer open");
            return;
        };
        let p = pending.remove(at);
        drop(pending);
        let body = if top {
            permission::decision(allow, None, None)
        } else {
            permission::decision(
                allow,
                params.get("updatedPermissions"),
                params.get("updatedInput"),
            )
        };
        let _ = p.reply.send(body);
    }

    /// Writes Copilot's hooks file for a Copilot terminal, and links the terminal to the
    /// conversation id its hooks post.
    fn copilot_terminal(&self, native: &Native, id: &str) {
        let file = &self.homes.copilot_hooks;
        match copilot::install(file, &self.homes.vorn) {
            Ok(copilot::Installed::NotOurs) => {
                warn!(file = %file.display(), "Copilot's hooks file is not Vorn's; left it alone");
            }
            Ok(_) => self.copilot.store(true, Ordering::SeqCst),
            Err(err) => warn!(%err, "could not write Copilot's hooks file"),
        }
        let session = copilot::session_id(id);
        lock(&self.mapper).force_link(&session, id);
        patch(native, id, json!({ "hookSessionId": session }));
    }
}

/// Answers `permission:resolve` and `permission:resolve-top`.
pub fn answer(native: &Native, method: &str, params: &Value) -> Answer {
    let Some(hooks) = native.hooks.get() else {
        return Answer::Error("vornd has no hook endpoint".to_owned());
    };
    hooks.resolve(params, method == "permission:resolve-top");
    Answer::Void
}

fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(file)?;
        f.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(file, text)
}

fn terminal_view(t: &TerminalSession) -> Terminal {
    let dir = t.worktree_path.as_deref().unwrap_or(&t.project_path);
    Terminal {
        id: t.id.clone(),
        agent_session: t.agent_session_id.clone(),
        hook_session: t.hook_session_id.clone(),
        created_at: t.created_at as f64,
        path: normalize(dir),
    }
}

fn about(native: &Native, id: &str) -> permission::About {
    let record = native
        .registry
        .get()
        .and_then(|r| {
            r.read(|r| {
                r.terminal(id)
                    .map(|(t, _)| (t.agent_type.clone(), t.project_name.clone()))
            })
        })
        .flatten();
    permission::About {
        terminal: id.to_owned(),
        agent_type: record.as_ref().map(|r| r.0.clone()),
        project_name: record.map(|r| r.1),
    }
}

fn hook_status(native: &Native, id: &str, status: Option<Status>, promote: bool) {
    let Some(registry) = native.registry.get() else {
        return;
    };
    let call = json!({ "id": id, "status": status.map(Status::as_str), "promote": promote });
    let head = native.host.get().and_then(|h| h.head_stamp(id));
    let done = HookStatus::try_from(&call)
        .and_then(|c| registry.hook_status(&c, head, tokio::time::Instant::now()));
    if let Err(err) = done {
        debug!(%err, "a hook's status did not apply");
    }
}

fn patch(native: &Native, id: &str, fields: Value) {
    let Some(registry) = native.registry.get() else {
        return;
    };
    let done =
        Patch::try_from(&json!({ "id": id, "fields": fields })).and_then(|p| registry.patch(&p));
    if let Err(err) = done {
        debug!(%err, "a hook's link did not apply");
    }
}

/// The conversation a terminal started, on the in-progress task it was assigned, when the task has none yet.
async fn remember_on_task(native: &Arc<Native>, terminal: &str, session: &str) {
    if native.database().is_none() {
        return;
    }
    let (n, t, s) = (Arc::clone(native), terminal.to_owned(), session.to_owned());
    let changed = super::config::blocking("hooks", move || {
        super::config::with_store(&n, |store| {
            let tasks = store.call("dbListTasks", json!([null, "in_progress"]))?;
            let task = tasks.as_array().into_iter().flatten().find(|task| {
                task.get("assignedSessionId").and_then(Value::as_str) == Some(t.as_str())
                    && task.get("agentSessionId").is_none_or(Value::is_null)
            });
            let Some(id) = task.and_then(|t| t.get("id")).and_then(Value::as_str) else {
                return Ok(false);
            };
            let now = vorn_work::js::iso_now();
            store.call(
                "dbUpdateTask",
                json!([id, { "agentSessionId": s, "updatedAt": now }, ["agentSessionId", "updatedAt"]]),
            )?;
            Ok(true)
        })
    })
    .await;
    match changed {
        Ok(true) => {
            info!(
                terminal,
                session, "the conversation is kept on the terminal's task"
            );
            super::config::announce(native).await;
        }
        Ok(false) => {}
        Err(err) => warn!(%err, "could not keep the conversation on the task"),
    }
}

/// Serves the endpoint until the hooks go.
async fn serve(hooks: Weak<Hooks>, listener: TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(err) => {
                warn!(%err, "a hook connection was not accepted");
                continue;
            }
        };
        if hooks.strong_count() == 0 {
            return;
        }
        let hooks = hooks.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req| handle(hooks.clone(), req));
            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            if let Err(err) = conn.await {
                debug!(%peer, %err, "a hook connection ended with an error");
            }
        });
    }
}

fn reply(status: StatusCode, body: impl Into<Bytes>) -> Response<Body> {
    let mut res = Response::new(full(body.into()));
    *res.status_mut() = status;
    if status != StatusCode::NOT_FOUND {
        res.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
    }
    res
}

/// Removes a request whose connection went before it was answered.
struct OpenRequest {
    hooks: Arc<Hooks>,
    id: Option<String>,
}

impl Drop for OpenRequest {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.hooks.abandoned(&id);
        }
    }
}

async fn handle(hooks: Weak<Hooks>, req: Request<Incoming>) -> Result<Response<Body>, Infallible> {
    let Some(hooks) = hooks.upgrade() else {
        return Ok(reply(StatusCode::NOT_FOUND, Bytes::new()));
    };
    if req.method() != Method::POST {
        return Ok(reply(StatusCode::NOT_FOUND, Bytes::new()));
    }
    let bearer = format!("Bearer {}", hooks.token);
    let authorized = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(bearer.as_str());
    if !authorized {
        return Ok(reply(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"Unauthorized"}"#,
        ));
    }
    // Past the token only: an agent outside Vorn posts here too, and nothing else may hold vornd up.
    *lock(&hooks.last_activity) = Instant::now();
    let header = req
        .headers()
        .get(vorn_hooks::event::TERMINAL_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => {
            return Ok(reply(
                StatusCode::PAYLOAD_TOO_LARGE,
                r#"{"error":"Request body too large"}"#,
            ))
        }
    };
    let event = match Event::parse(&body, header.as_deref()) {
        Ok(e) => e,
        Err(why) => {
            warn!(%why, "ignored a malformed hook event");
            return Ok(reply(
                StatusCode::BAD_REQUEST,
                r#"{"error":"Malformed hook event"}"#,
            ));
        }
    };
    if event.name() != "PermissionRequest" {
        let h = Arc::clone(&hooks);
        tokio::spawn(async move { h.heard(event) });
        return Ok(reply(StatusCode::OK, "{}"));
    }
    let (id, answer) = hooks.asked(event);
    let mut open = OpenRequest {
        hooks: Arc::clone(&hooks),
        id,
    };
    let body = answer.await.unwrap_or_else(|_| "{}".to_owned());
    open.id = None;
    Ok(reply(StatusCode::OK, body))
}

/// Acts on each terminal new to the registry: a Copilot terminal's hooks,
/// and the conversation of an agent that cannot be told one.
async fn follow(hooks: Weak<Hooks>, mut notes: tokio::sync::broadcast::Receiver<Value>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        let note = match notes.recv().await {
            Ok(note) => note,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let Some(hooks) = hooks.upgrade() else { return };
        let Some(native) = hooks.native.upgrade() else {
            return;
        };
        let records: Vec<&Value> = match note.get("op").and_then(Value::as_str) {
            Some("upsert") if note.get("kind").and_then(Value::as_str) == Some("terminal") => {
                note.get("record").into_iter().collect()
            }
            Some("snapshot") => {
                // Records that were there before this vornd listened are not new.
                for t in note
                    .get("terminals")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = t.get("id").and_then(Value::as_str) {
                        seen.insert(id.to_owned());
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        for record in records {
            let text = |k: &str| {
                record
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
            };
            let Some(id) = text("id") else { continue };
            let agent = text("agentType").unwrap_or_default();
            if agent == "copilot" && text("hookSessionId").is_none() {
                hooks.copilot_terminal(&native, id);
            }
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let wants_capture = vorn_agents::Agent::from_id(agent)
                .is_some_and(|a| a.resumes_exactly() && !a.pins_session_ids())
                && text("agentSessionId").is_none()
                && text("remoteHostId").is_none();
            if let (true, Some(db_agent)) =
                (wants_capture, vorn_hooks::capture::Agent::from_id(agent))
            {
                tokio::spawn(capture(
                    Arc::downgrade(&native),
                    hooks.homes.home.clone(),
                    db_agent,
                    id.to_owned(),
                ));
            }
        }
    }
}

/// Reads the conversation `id`'s agent took, a few times over its first
/// forty seconds, and keeps it on the record.
async fn capture(
    native: Weak<Native>,
    home: PathBuf,
    agent: vorn_hooks::capture::Agent,
    id: String,
) {
    // Where OpenCode keeps its data: LOCALAPPDATA on Windows, XDG_DATA_HOME elsewhere.
    let data_home = if cfg!(windows) {
        Some(
            std::env::var_os("LOCALAPPDATA")
                .map_or_else(|| home.join("AppData").join("Local"), PathBuf::from),
        )
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
    };
    let db = agent.database(&home, data_home.as_deref());
    for secs in CAPTURE_LADDER {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let Some(native) = native.upgrade() else {
            return;
        };
        let Some(registry) = native.registry.get() else {
            return;
        };
        let record = registry
            .read(|r| {
                r.terminal(&id).map(|(t, _)| {
                    (
                        t.agent_session_id.clone(),
                        t.worktree_path
                            .clone()
                            .unwrap_or_else(|| t.project_path.clone()),
                    )
                })
            })
            .flatten();
        let release = || {
            if let Some(link) = native.link.get() {
                link.claims().release_for(&id);
            }
        };
        let Some((taken, cwd)) = record else {
            release();
            return;
        };
        if taken.is_some() {
            return;
        }
        let db2 = db.clone();
        let found =
            tokio::task::spawn_blocking(move || vorn_hooks::capture::capture(agent, &cwd, &db2))
                .await
                .ok()
                .flatten();
        if let Some(conversation) = found {
            patch(&native, &id, json!({ "agentSessionId": conversation }));
            release();
            info!(%id, %conversation, "captured the agent's conversation");
            return;
        }
    }
}
