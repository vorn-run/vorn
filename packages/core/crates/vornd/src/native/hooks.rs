//! The endpoint agents' hooks post to, and what follows from each event
//! (`hook-server`, `hookStatusMapper`).
//!
//! It listens on loopback, on a fixed port when it is free so the agents'
//! settings stay the same across restarts, and answers only requests that
//! carry its token. One running Vorn registers it ([`vorn_hooks::owner`]):
//! that one writes the port and token beside the owner record in `~/.vorn`;
//! another waits for the registration to come free. That one also merges hooks into the settings
//! of every agent whose adapter asks for them ([`vorn_hooks::settings`]),
//! and takes them out again when the agent is switched off or vornd stops.
//! An event sets the status of the terminal it
//! is about, from then on taken from hooks rather than the screen, and links
//! the terminal to the agent's conversation. A permission request is held
//! open until a client answers it (`permission:resolve`), the agent moves
//! on, or the agent stops waiting.
//!
//! Each new terminal is also followed from the registry's notes: a terminal
//! whose adapter keeps a hooks file gets it written ([`vorn_hooks::file`]),
//! and an agent that cannot be told its conversation has it read from the
//! agent's own database a few times over its first forty seconds
//! ([`vorn_hooks::capture`]).

use std::collections::{BTreeMap, HashSet};
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
use vorn_agents::spec::{Places, SettingsHooks};
use vorn_agents::Agent;
use vorn_extensions::events::{
    AGENT_CONVERSATION, AGENT_HOOK, AGENT_PERMISSION, AGENT_PERMISSION_CLOSED,
};
use vorn_hooks::mapper::{normalize, Status};
use vorn_hooks::owner::{may_claim, may_release, Owner};
use vorn_hooks::{file, permission, settings, Event, Mapper, Terminal};

use super::{Answer, Native};
use crate::endpoint::{full, Body};
use crate::registry::{HookStatus, Patch, TerminalSession};

/// Every call this module answers.
pub const METHODS: &[&str] = &[
    "permission:resolve",
    "permission:resolve-top",
    "permission:list",
];

/// Tried first, so the settings entry naming it stays the same across restarts.
const PREFERRED_PORT: u16 = 56432;
const MAX_BODY: usize = 1024 * 1024;
/// How often a Vorn that found the registration held looks again.
const OWNER_POLL: Duration = Duration::from_secs(1);
/// How long the version probe waits for the login shell's PATH.
const VERSION_PROBE_WAIT: Duration = Duration::from_secs(30);
/// When an agent's own database is read for its conversation, after the last read.
const CAPTURE_LADDER: [u64; 4] = [5, 5, 10, 20];

/// A permission request held open.
#[derive(Debug)]
struct Pending {
    id: String,
    session: String,
    terminal: String,
    /// The rules the agent offered to keep, which an answer may pick one of by index.
    suggestions: Vec<Value>,
    /// The tool's input, which answers to its questions are added to.
    input: Value,
    /// What clients are told of it: `permission:list` and `permission:requested`.
    info: Value,
    reply: oneshot::Sender<String>,
}

/// The user's directories the endpoint's files go in.
#[derive(Clone, Debug)]
pub struct Homes {
    /// `~/.vorn`: the port, token and owner record.
    pub vorn: PathBuf,
    /// Where the agents' adapters name their files from.
    pub places: Places,
}

impl Homes {
    /// From `HOME` (`USERPROFILE` on Windows).
    pub fn from_env() -> Option<Homes> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)?;
        let data = Places::from_env().map_or_else(|| home.join(".local").join("share"), |p| p.data);
        Some(Homes {
            vorn: home.join(".vorn"),
            places: Places {
                home: home.clone(),
                data,
            },
        })
    }
}

/// Settings hooks Vorn merged into an agent's settings, to take out again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Merged {
    file: PathBuf,
    /// Every event the adapter names, so each is cleaned.
    all: Vec<String>,
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
    /// The hooks files it wrote, to remove when it stops.
    files: Mutex<HashSet<PathBuf>>,
    /// The settings it merged hooks into, by agent.
    merged: Mutex<BTreeMap<String, Merged>>,
    /// The agents whose CLI on PATH reports its own status (OSC 7501): they
    /// get only the hooks they need answered.
    own_status: Mutex<HashSet<String>>,
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
            files: Mutex::default(),
            merged: Mutex::default(),
            own_status: Mutex::default(),
            stopped: AtomicBool::new(false),
            pending: Mutex::default(),
            mapper: Mutex::default(),
            last_activity: Mutex::new(Instant::now()),
        });
        tokio::spawn(serve(Arc::downgrade(&hooks), listener));
        hooks.probe_versions(native);
        tokio::spawn(follow_adapters(
            Arc::downgrade(&hooks),
            native.adapters().subscribe(),
        ));
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
            write_private(&self.homes.vorn.join("token"), &self.token)
        })();
        if let Err(err) = written {
            warn!(%err, "could not register the hook endpoint");
            return false;
        }
        self.owner.store(true, Ordering::SeqCst);
        self.sync_settings();
        true
    }

    /// Merges hooks into the settings of each agent that is on and asks for
    /// them, and takes them out of an agent's that was switched off. Only
    /// while this vornd holds the registration.
    fn sync_settings(&self) {
        if !self.owns_registration() || self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let Some(native) = self.native.upgrade() else {
            return;
        };
        let wanted: Vec<(Agent, SettingsHooks)> = native
            .agents()
            .iter()
            .filter_map(|a| {
                let spec = a.spec().hooks.as_ref()?.settings.clone()?;
                Some((a.clone(), spec))
            })
            .collect();
        // Held throughout: `stop` takes it after marking itself stopped, so
        // nothing is merged back in once it has taken the hooks out.
        let mut merged = lock(&self.merged);
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let gone: Vec<Merged> = {
            let keep: HashSet<&str> = wanted.iter().map(|(a, _)| a.id()).collect();
            let gone: Vec<String> = merged
                .keys()
                .filter(|id| !keep.contains(id.as_str()))
                .cloned()
                .collect();
            gone.iter().filter_map(|id| merged.remove(id)).collect()
        };
        for m in gone {
            let all: Vec<&str> = m.all.iter().map(String::as_str).collect();
            if let Err(err) = settings::uninstall_file(&m.file, &all) {
                debug!(%err, file = %m.file.display(), "could not take hooks out of an agent's settings");
            }
        }
        for (agent, spec) in wanted {
            let file = self.homes.places.path(&spec.file);
            let reports = lock(&self.own_status).contains(agent.id());
            let mut all: Vec<&str> = spec.events.iter().map(String::as_str).collect();
            let events: Vec<&str> = match (&spec.own_status, reports) {
                (Some(own), true) => {
                    for e in &own.events {
                        if !all.contains(&e.as_str()) {
                            all.push(e);
                        }
                    }
                    own.events.iter().map(String::as_str).collect()
                }
                _ => all.clone(),
            };
            match settings::install_file(&file, self.port, &self.token, &all, &events) {
                Ok(()) => {
                    let all = all.iter().map(|e| (*e).to_owned()).collect();
                    merged.insert(agent.id().to_owned(), Merged { file, all });
                }
                Err(err) => {
                    warn!(%err, agent = agent.id(), "could not merge hooks into an agent's settings")
                }
            }
        }
    }

    /// Asks each agent CLI whose adapter names a version it reports its own
    /// status from for its version, once the login shell's PATH is known;
    /// one that does gets only the hooks it needs answered.
    fn probe_versions(self: &Arc<Self>, native: &Arc<Native>) {
        let wanted: Vec<(String, String, String)> = native
            .adapters()
            .agents()
            .iter()
            .filter_map(|a| {
                let own = a
                    .spec()
                    .hooks
                    .as_ref()?
                    .settings
                    .as_ref()?
                    .own_status
                    .as_ref()?;
                Some((
                    a.id().to_owned(),
                    a.spec().launch.command.clone(),
                    own.since.clone(),
                ))
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        let (weak, env) = (Arc::downgrade(self), Arc::clone(&native.env));
        // A thread of its own, so a slow probe never holds vornd's shutdown.
        let spawned = std::thread::Builder::new()
            .name("version-probe".into())
            .spawn(move || {
                env.prime();
                let deadline = Instant::now() + VERSION_PROBE_WAIT;
                while !env.resolved() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(250));
                }
                let mut changed = false;
                for (id, command, since) in wanted {
                    let version = env
                        .find(&command)
                        .and_then(|bin| super::shell::version_of(&bin, Duration::from_secs(5)));
                    let Some(hooks) = weak.upgrade() else { return };
                    let reports = version
                        .as_deref()
                        .is_some_and(|v| settings::reports_status(v, &since));
                    info!(agent = %id, ?version, reports, "an agent CLI on PATH");
                    if reports {
                        changed |= lock(&hooks.own_status).insert(id);
                    }
                }
                if let (true, Some(hooks)) = (changed, weak.upgrade()) {
                    hooks.sync_settings();
                }
            });
        if let Err(err) = spawned {
            warn!(%err, "could not probe the agents' versions");
        }
    }

    /// Denies what is pending and gives the registration up, when it is still this vornd's.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        for p in lock(&self.pending).drain(..) {
            let _ = p.reply.send(permission::decision(false, None, None));
        }
        for written in lock(&self.files).drain() {
            if let Err(err) = file::uninstall(&written) {
                debug!(%err, file = %written.display(), "could not remove a hooks file");
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
        let merged = std::mem::take(&mut *lock(&self.merged));
        for m in merged.into_values() {
            let all: Vec<&str> = m.all.iter().map(String::as_str).collect();
            if let Err(err) = settings::uninstall_file(&m.file, &all) {
                debug!(%err, file = %m.file.display(), "could not remove the hooks from an agent's settings");
            }
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
        // One per agent tool call: debug, or a busy agent fills the log.
        debug!(
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
            let terminal = resolved.terminal.as_str();
            native.events().emit(AGENT_HOOK, || {
                json!({ "sessionId": terminal, "event": event.name(), "tool": event.tool_name() })
            });
            if event.name() == "SessionStart" {
                native.events().emit(
                    AGENT_CONVERSATION,
                    || json!({ "sessionId": terminal, "conversationId": event.session() }),
                );
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
        let info = permission::info(&id, &event, &about(&native, &resolved.terminal));
        lock(&self.pending).push(Pending {
            id: id.clone(),
            session: event.session().to_owned(),
            terminal: resolved.terminal.clone(),
            suggestions: (event
                .get("permission_suggestions")
                .and_then(Value::as_array))
            .cloned()
            .unwrap_or_default(),
            input: (event.get("tool_input").filter(|v| v.is_object()))
                .cloned()
                .unwrap_or_else(|| json!({})),
            info: info.clone(),
            reply: tx,
        });
        if let Some(registry) = native.registry.get() {
            registry.asking(&resolved.terminal, true);
        }
        hook_status(&native, &resolved.terminal, None, true);
        info!(request = %id, tool = event.tool_name(), terminal = %resolved.terminal, "a permission request");
        native.broadcast("permission:requested", info.clone());
        native.events().emit(AGENT_PERMISSION, || info);
        hook_status(&native, &resolved.terminal, Some(Status::Waiting), false);
        (Some(id), rx)
    }

    /// A request whose agent stopped waiting: clients are told it is gone.
    fn abandoned(&self, id: &str) {
        let mut pending = lock(&self.pending);
        let Some(at) = pending.iter().position(|p| p.id == id) else {
            return;
        };
        let p = pending.remove(at);
        drop(pending);
        if let Some(native) = self.native.upgrade() {
            closed(&native, id, &p.terminal);
            self.answered(&native, &p.terminal);
        }
    }

    /// A request of `terminal` closed: with none left open, it stops waiting on Vorn.
    fn answered(&self, native: &Native, terminal: &str) {
        if lock(&self.pending).iter().any(|p| p.terminal == terminal) {
            return;
        }
        if let Some(registry) = native.registry.get() {
            registry.asking(terminal, false);
        }
    }

    fn cancel_session(&self, native: &Native, session: &str) {
        self.cancel(native, |p| p.session == session);
    }

    /// The requests `which` picks, answered with no decision as their agent moved past them.
    fn cancel(&self, native: &Native, which: impl Fn(&Pending) -> bool) {
        let gone: Vec<Pending> = {
            let mut pending = lock(&self.pending);
            let (gone, kept) = pending.drain(..).partition(|p| which(p));
            *pending = kept;
            gone
        };
        for p in gone {
            info!(request = %p.id, session = %p.session, "a permission request the agent moved past");
            let _ = p.reply.send("{}".to_owned());
            closed(native, &p.id, &p.terminal);
            self.answered(native, &p.terminal);
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
        if let Some(native) = self.native.upgrade() {
            closed(&native, &p.id, &p.terminal);
            self.answered(&native, &p.terminal);
        }
        let body = if top {
            permission::decision(allow, None, None)
        } else {
            let (rules, input) = picked(&p, params);
            permission::decision(
                allow,
                rules.as_ref().or(params.get("updatedPermissions")),
                input.as_ref().or(params.get("updatedInput")),
            )
        };
        let _ = p.reply.send(body);
    }

    /// Writes the hooks file `agent`'s adapter keeps for its terminal `id`,
    /// and links the terminal to the conversation id its hooks post.
    fn file_terminal(&self, native: &Native, agent: &Agent, id: &str) {
        let Some(spec) = agent.spec().hooks.as_ref().and_then(|h| h.file.as_ref()) else {
            return;
        };
        let path = self.homes.places.path(&spec.path);
        let hooks = file::Spec {
            session: &spec.session,
            events: &spec.events,
        };
        match file::install(hooks, &path, &self.homes.vorn) {
            Ok(file::Installed::NotOurs) => {
                warn!(file = %path.display(), agent = agent.id(), "the agent's hooks file is not Vorn's; left it alone");
            }
            Ok(_) => {
                lock(&self.files).insert(path);
            }
            Err(err) => warn!(%err, agent = agent.id(), "could not write the agent's hooks file"),
        }
        let session = hooks.session_id(id);
        lock(&self.mapper).force_link(&session, id);
        patch(native, id, json!({ "hookSessionId": session }));
    }
}

/// Tells subscribers and clients request `id` of `terminal` is no longer open.
fn closed(native: &Native, id: &str, terminal: &str) {
    native.broadcast(
        "permission:closed",
        json!({ "requestId": id, "terminalId": terminal }),
    );
    native.events().emit(
        AGENT_PERMISSION_CLOSED,
        || json!({ "requestId": id, "terminalId": terminal }),
    );
}

/// Answers `permission:resolve`, `permission:resolve-top` and `permission:list`.
pub fn answer(native: &Native, method: &str, params: &Value) -> Answer {
    let Some(hooks) = native.hooks.get() else {
        return Answer::Error("vornd has no hook endpoint".to_owned());
    };
    if method == "permission:list" {
        let open = lock(&hooks.pending)
            .iter()
            .map(|p| p.info.clone())
            .collect();
        return Answer::Result(Value::Array(open));
    }
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

/// Whether a terminal record's program reports it waits for a permission.
fn asks_permission(record: &Value) -> bool {
    let Some(p) = record.get("programStatus").filter(|p| p["native"] == true) else {
        return false;
    };
    p["records"]
        .as_array()
        .into_iter()
        .flatten()
        .any(blocked_on_permission)
}

fn blocked_on_permission(r: &Value) -> bool {
    let kind = r.get("kind");
    r["state"] == "blocked" && kind.is_none_or(|k| k == "permission")
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

/// Acts on each terminal new to the registry: a hooks file its adapter keeps,
/// and the conversation of an agent that cannot be told one.
async fn follow(hooks: Weak<Hooks>, mut notes: tokio::sync::broadcast::Receiver<Value>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut seen: HashSet<String> = HashSet::new();
    // Terminals whose program reports it waits for permission.
    let mut blocked: HashSet<String> = HashSet::new();
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
            if asks_permission(record) {
                blocked.insert(id.to_owned());
            } else if blocked.remove(id) {
                hooks.cancel(&native, |p| p.terminal == id);
            }
            let Some(agent) = native.adapters().get(text("agentType").unwrap_or_default()) else {
                seen.insert(id.to_owned());
                continue;
            };
            let has_file = agent
                .spec()
                .hooks
                .as_ref()
                .is_some_and(|h| h.file.is_some());
            if has_file && text("hookSessionId").is_none() {
                hooks.file_terminal(&native, &agent, id);
            }
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let wants_capture = agent.resumes_exactly()
                && !agent.pins_session_ids()
                && text("agentSessionId").is_none()
                && text("remoteHostId").is_none();
            let capture_spec = agent.spec().resume.as_ref().and_then(|r| r.capture.clone());
            if let (true, Some(spec)) = (wants_capture, capture_spec) {
                tokio::spawn(capture(
                    Arc::downgrade(&native),
                    hooks.homes.places.path(&spec.db),
                    spec.query,
                    id.to_owned(),
                ));
            }
        }
    }
}

/// Merges or takes out settings hooks whenever which agents are on changes.
async fn follow_adapters(hooks: Weak<Hooks>, mut changed: tokio::sync::watch::Receiver<u64>) {
    while changed.changed().await.is_ok() {
        let Some(hooks) = hooks.upgrade() else { return };
        tokio::task::spawn_blocking(move || hooks.sync_settings());
    }
}

/// Reads the conversation `id`'s agent took from `db` with `query`, a few
/// times over its first forty seconds, and keeps it on the record.
async fn capture(native: Weak<Native>, db: PathBuf, query: String, id: String) {
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
        let (db2, query2) = (db.clone(), query.clone());
        let found =
            tokio::task::spawn_blocking(move || vorn_hooks::capture::capture(&query2, &cwd, &db2))
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

/// What an answer picked by name rather than spelt out: `suggestion`, the index of a
/// rule the agent offered, and `answers`, added to the tool's input under `answers`.
fn picked(p: &Pending, params: &Value) -> (Option<Value>, Option<Value>) {
    let rules = (params.get("suggestion").and_then(Value::as_u64))
        .and_then(|i| p.suggestions.get(usize::try_from(i).ok()?))
        .map(|sg| json!([sg]));
    let input = params.get("answers").filter(|a| a.is_object()).map(|a| {
        let mut input = p.input.clone();
        input["answers"] = a.clone();
        input
    });
    (rules, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_picks_an_offered_rule_and_adds_answers_to_the_input() {
        let (reply, _) = oneshot::channel();
        let p = Pending {
            id: "r".into(),
            session: String::new(),
            terminal: String::new(),
            suggestions: vec![json!({ "type": "setMode", "mode": "plan" })],
            input: json!({ "questions": [] }),
            info: Value::Null,
            reply,
        };
        let (rules, input) = picked(&p, &json!({ "suggestion": 0, "answers": { "0": "Dark" } }));
        assert_eq!(rules, Some(json!([{ "type": "setMode", "mode": "plan" }])));
        assert_eq!(
            input,
            Some(json!({ "questions": [], "answers": { "0": "Dark" } }))
        );
        assert_eq!(picked(&p, &json!({ "suggestion": 3 })), (None, None));
    }
}
