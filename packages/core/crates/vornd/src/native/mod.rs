//! The calls vornd answers itself, in the groups it has taken over.
//!
//! [`Conn::offer`] sees each call a client sends and decides, by its group's
//! mode ([`crate::groups`]), whether vornd answers it, the server does, or
//! both do and the answers are compared:
//!
//! - **native**: the call is answered here, framed exactly as the server's
//!   `ws-handler` frames it (`{jsonrpc, id, result}`, no `result` at all for
//!   a call that returns nothing, `{code: -32000, message}` for an error), on
//!   the connection's one ordered outbox. Some calls are still the server's,
//!   and go to it: those in [`SERVER_ONLY`], which need what only the server
//!   holds; any call whose path is in a project on a remote host; and any
//!   whose params are not the shape the server's handler expects. A
//!   connection is answered here only once the server has admitted it: the
//!   desktop's from the start, any other after the server's `auth:ok` or its
//!   first answer, so a socket that has not authenticated never reaches git
//!   or the file system through vornd.
//! - **shadow**: the call goes to the server, whose answer the client gets,
//!   and calls that change nothing ([`Effect::Read`]) are also run here; the
//!   two answers are compared when both are in ([`Conn::on_server_text`]),
//!   and a difference is logged with the method and counted. The client
//!   never sees vornd's answer.
//!
//! The work runs on blocking threads, at most [`MAX_CONCURRENT`] at a time,
//! and the calls that change a repository take turns per repository
//! ([`Turns`]), as the server's do.

pub mod agent;
pub mod env;
pub mod file;
pub mod git;
pub mod ide;
pub mod reach;
pub mod shell;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use vorn_store::{Placement, ProjectHosts};

use crate::applink::AppLink;
use crate::groups::{Counted, Groups, Mode};
use crate::streams::Forwarder;

/// Native calls running at once; the rest wait their turn. A board
/// refreshing thirty diff panels would otherwise start thirty gits together.
pub const MAX_CONCURRENT: usize = 8;

/// Whether a call changes anything, which decides whether shadow mode may run
/// it a second time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Read,
    Change,
}

/// Every call vornd answers, and its effect. `git:listRemoteBranches`
/// fetches, `ide:open` starts an editor and `agent:listModels` starts the
/// agent's CLI, so none of them runs twice.
pub const METHODS: &[(&str, Effect)] = &[
    ("git:isGitRepo", Effect::Read),
    ("git:listBranches", Effect::Read),
    ("git:listRemoteBranches", Effect::Change),
    ("git:createWorktree", Effect::Change),
    ("git:removeWorktree", Effect::Change),
    ("git:getWorktreeBranch", Effect::Read),
    ("git:worktreeDirty", Effect::Read),
    ("git:listWorktrees", Effect::Read),
    ("git:deleteBranches", Effect::Change),
    ("git:getBranch", Effect::Read),
    ("git:diffStat", Effect::Read),
    ("git:diffFull", Effect::Read),
    ("git:commit", Effect::Change),
    ("git:push", Effect::Change),
    ("file:listDir", Effect::Read),
    ("file:readContent", Effect::Read),
    ("file:stamp", Effect::Read),
    ("file:writeContent", Effect::Change),
    ("ide:detect", Effect::Read),
    ("ide:open", Effect::Change),
    ("server:reachableUrls", Effect::Read),
    ("tailscale:status", Effect::Read),
    ("token:list", Effect::Read),
    ("token:create", Effect::Change),
    ("token:revoke", Effect::Change),
    // Pairing is held in one place, the server's or vornd's, so its calls
    // are never run on both sides to compare: listing prunes, too.
    ("pairing:start", Effect::Change),
    ("pairing:pending", Effect::Change),
    ("pairing:approve", Effect::Change),
    ("pairing:deny", Effect::Change),
    ("pairing:cancel", Effect::Change),
    ("agent:detectInstalled", Effect::Read),
    ("agent:listModels", Effect::Change),
    ("sessions:getRecent", Effect::Read),
    ("shell:listExecutables", Effect::Read),
    ("shell:listInstalled", Effect::Read),
];

/// Calls in a native group that the server keeps answering, and why.
pub const SERVER_ONLY: &[(&str, &str)] = &[
    (
        "git:checkoutBranch",
        "moves the server's sessions on that worktree to the new branch and tells clients",
    ),
    (
        "git:renameWorktreeBranch",
        "moves the server's sessions on that worktree to the new branch and tells clients",
    ),
    (
        "git:renameWorktree",
        "moves the server's sessions to the worktree's new path and tells clients",
    ),
    ("server:shutdown", "stops the server itself"),
    (
        "server:handoff",
        "hands the server's listener and sessions to the server taking over",
    ),
    (
        "server:vornd",
        "reports on the vornd the server keeps running",
    ),
    (
        "auth:authenticate",
        "admits the server's own socket; vornd checks the credential beside it",
    ),
    (
        "sessions:restored",
        "lists the sessions the server carried over from its last run",
    ),
    (
        "sessions:resume",
        "starts a session in the server's registry, under the id it had",
    ),
    (
        "sessions:clear",
        "declines the server's carried-over sessions and saves the registry",
    ),
    (
        "shell:create",
        "starts a shell session in the server's registry and tells clients",
    ),
];

/// The effect of a call vornd answers, or `None` for one it does not.
pub fn effect(method: &str) -> Option<Effect> {
    METHODS.iter().find(|(m, _)| *m == method).map(|(_, e)| *e)
}

/// What a call came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Result(Value),
    /// A call that returns nothing: the frame has no `result`.
    Void,
    /// The server's handler would have thrown this message.
    Error(String),
    /// The server's to answer.
    Forward,
}

impl Answer {
    /// The frame a client gets for request `id`; `None` to forward.
    pub fn frame(&self, id: &Value) -> Option<Value> {
        let mut frame = json!({ "jsonrpc": "2.0", "id": id });
        match self {
            Answer::Result(v) => frame["result"] = v.clone(),
            Answer::Void => {}
            Answer::Error(message) => {
                frame["error"] = json!({ "code": -32000, "message": message });
            }
            Answer::Forward => return None,
        }
        Some(frame)
    }
}

/// What two answers to one request are compared by: the result, or the
/// error's code and message.
fn comparable(frame: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(result) = frame.get("result") {
        out.insert("result".into(), result.clone());
    }
    if let Some(error) = frame.get("error") {
        out.insert(
            "error".into(),
            json!({ "code": error.get("code"), "message": error.get("message") }),
        );
    }
    Value::Object(out)
}

/// Where two answers first differ, as a JSON pointer, for the log. The
/// values themselves are not logged: they can be a file's contents.
pub fn first_difference(a: &Value, b: &Value) -> String {
    fn walk(a: &Value, b: &Value, at: &mut String) -> bool {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
                keys.sort();
                keys.dedup();
                for k in keys {
                    let len = at.len();
                    at.push('/');
                    at.push_str(k);
                    match (x.get(k), y.get(k)) {
                        (Some(p), Some(q)) if !walk(p, q, at) => return false,
                        (Some(_), Some(_)) => {}
                        _ => return false,
                    }
                    at.truncate(len);
                }
                true
            }
            (Value::Array(x), Value::Array(y)) => {
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    let len = at.len();
                    at.push_str(&format!("/{i}"));
                    if !walk(p, q, at) {
                        return false;
                    }
                    at.truncate(len);
                }
                if x.len() != y.len() {
                    at.push_str(&format!("/{}", x.len().min(y.len())));
                    return false;
                }
                true
            }
            _ => a == b,
        }
    }
    let mut at = String::new();
    if walk(a, b, &mut at) {
        return String::new();
    }
    if at.is_empty() {
        "/".to_owned()
    } else {
        at
    }
}

/// The calls that change a repository take turns per repository
/// ([`vorn_git::repo::repo_key`]), so a commit's `add` and `commit` never
/// have another change between them.
#[derive(Debug, Default)]
pub struct Turns {
    repos: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl Turns {
    /// Runs `f` with the turn of the repository `path` is in. One entry is
    /// kept per repository changed, for as long as vornd runs.
    pub fn take<T>(&self, path: &Path, f: impl FnOnce() -> T) -> T {
        let key = vorn_git::repo::repo_key(path);
        let turn = Arc::clone(
            self.repos
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key)
                .or_default(),
        );
        let _held = turn.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
}

/// What vornd needs to answer calls: the environment programs run with,
/// where the server's database is, and what it keeps between calls.
#[derive(Debug)]
pub struct Native {
    env: Arc<env::SafeEnv>,
    /// `vorn.db`, read to tell a project on this machine from one on a
    /// remote host, and for how the agents are configured. Without it every
    /// call that needs either goes to the server, which can tell.
    db: OnceLock<PathBuf>,
    slots: Semaphore,
    turns: Turns,
    ignored: file::IgnoreCache,
    ides: ide::Ides,
    reach: reach::Reach,
    /// The app's channel, for what only the server can do.
    link: OnceLock<Arc<AppLink>>,
    /// The desktop's launch credential, which is also the server's local one.
    desktop: OnceLock<Vec<u8>>,
    /// The agents' model lists, kept as the server keeps them.
    catalog: vorn_agents::models::Catalog,
    shells: shell::Shells,
}

impl Native {
    pub fn new() -> Arc<Native> {
        Arc::new(Native {
            env: env::SafeEnv::new(),
            db: OnceLock::new(),
            slots: Semaphore::new(MAX_CONCURRENT),
            turns: Turns::default(),
            ignored: file::IgnoreCache::default(),
            ides: ide::Ides::default(),
            reach: reach::Reach::default(),
            link: OnceLock::new(),
            desktop: OnceLock::new(),
            catalog: vorn_agents::models::Catalog::default(),
            shells: shell::Shells::default(),
        })
    }

    /// The server's port, which the addresses a browser uses name.
    pub fn set_server_port(&self, port: u16) {
        self.reach.set_server_port(port);
    }

    /// The app's channel. Only the first one given is kept.
    pub fn set_link(&self, link: Arc<AppLink>) {
        let _ = self.link.set(link);
    }

    /// The desktop's launch credential. Only the first one given is kept.
    pub fn set_desktop_token(&self, token: Vec<u8>) {
        let _ = self.desktop.set(token);
    }

    /// Reads again which names a browser may load the web client from.
    pub fn refresh_trusted(&self) {
        self.reach.refresh_trusted(&self.env);
    }

    /// The names a browser may load the web client from, beyond addresses
    /// and `localhost`.
    pub fn trusted(&self) -> vorn_reach::origin::TrustedHosts {
        self.reach.trusted()
    }

    /// Where the server's database is. Only the first one given is kept.
    pub fn set_database(&self, db: PathBuf) {
        let _ = self.db.set(db);
    }

    /// Asks the login shell for its environment, in the background.
    pub fn prepare(&self) {
        self.env.prime();
    }

    /// Answers `method` with `params`, blocking this thread meanwhile.
    pub fn call(&self, method: &str, params: &Value) -> Answer {
        match method.split_once(':').map(|(g, _)| g) {
            Some("git") => git::call(self, method, params),
            Some("file") => self.file(method, params),
            Some("ide") => self.ide(method, params),
            Some("server" | "tailscale" | "token" | "pairing") => self.reach_call(method, params),
            Some("agent" | "sessions") => agent::call(self, method, params),
            Some("shell") => match method {
                "shell:listExecutables" => Answer::Result(self.shells.executables(&self.env)),
                "shell:listInstalled" => Answer::Result(self.shells.installed()),
                _ => Answer::Forward,
            },
            _ => Answer::Forward,
        }
    }

    fn file(&self, method: &str, params: &Value) -> Answer {
        // A remote host's files are the server's to reach.
        match params.get("remoteHostId") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(_) => return Answer::Forward,
        }
        let path_of = |key| params.get(key).and_then(absolute_str);
        match method {
            "file:listDir" => match path_of("dirPath") {
                Some(dir) => Answer::Result(Value::Array(
                    file::list_dir(dir, &self.env, &self.ignored)
                        .iter()
                        .map(file::FileEntry::to_json)
                        .collect(),
                )),
                None => Answer::Forward,
            },
            "file:readContent" => {
                let max = match params.get("maxBytes") {
                    None => Some(file::MAX_READ_BYTES),
                    Some(v) => v.as_u64(),
                };
                match (path_of("filePath"), max) {
                    (Some(path), Some(max)) => Answer::Result(json!(file::read_content(path, max))),
                    _ => Answer::Forward,
                }
            }
            "file:stamp" => match path_of("filePath") {
                Some(path) => Answer::Result(file::stamp(path).unwrap_or(Value::Null)),
                None => Answer::Forward,
            },
            "file:writeContent" => {
                match (
                    path_of("filePath"),
                    params.get("content").and_then(Value::as_str),
                ) {
                    (Some(path), Some(content)) => {
                        Answer::Result(file::write_content(path, content))
                    }
                    _ => Answer::Forward,
                }
            }
            _ => Answer::Forward,
        }
    }

    fn ide(&self, method: &str, params: &Value) -> Answer {
        match method {
            "ide:detect" => Answer::Result(self.ides.detect_json(&self.env)),
            "ide:open" => {
                let id = params.get("ideId").and_then(Value::as_str);
                let project = params.get("projectPath").and_then(Value::as_str);
                match (id, project) {
                    (Some(id), Some(project)) => {
                        self.ides.open(id, project, &self.env);
                        Answer::Void
                    }
                    _ => Answer::Forward,
                }
            }
            _ => Answer::Forward,
        }
    }

    /// The projects and hosts, fresh from the database: the server may
    /// have changed them since the last call. `None` when vornd cannot tell.
    fn hosts(&self) -> Option<ProjectHosts> {
        let db = self.db.get()?;
        match ProjectHosts::read(db) {
            Ok(hosts) => Some(hosts.unwrap_or_default()),
            Err(err) => {
                debug!(%err, "could not read the projects; the server answers");
                None
            }
        }
    }

    /// Whether the project at `path` is known to be on this machine.
    fn local_project(&self, path: &str) -> bool {
        self.hosts()
            .is_some_and(|h| h.for_project(path) == Placement::Local)
    }

    /// Whether `path` is known to be in no project on a remote host.
    fn local_path(&self, path: &str) -> bool {
        self.hosts()
            .is_some_and(|h| h.for_path(path) == Placement::Local)
    }
}

/// A string param that is an absolute path. A relative one would resolve
/// against vornd's working directory rather than the server's.
fn absolute_str(v: &Value) -> Option<&str> {
    v.as_str().filter(|p| Path::new(p).is_absolute())
}

/// Requests out to the server and to vornd at once, in shadow mode.
#[derive(Debug, Default)]
pub struct Shadows {
    pending: Mutex<HashMap<String, Pending>>,
    open: AtomicUsize,
}

#[derive(Debug)]
struct Pending {
    method: String,
    native: Option<Value>,
    server: Option<Value>,
}

enum Side {
    Native,
    Server,
}

impl Shadows {
    fn begin(&self, key: String, method: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.insert(
            key,
            Pending {
                method: method.to_owned(),
                native: None,
                server: None,
            },
        );
        self.open.store(pending.len(), Ordering::Release);
    }

    fn cancel(&self, key: &str) -> Option<String> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let gone = pending.remove(key).map(|p| p.method);
        self.open.store(pending.len(), Ordering::Release);
        gone
    }

    /// Files one side's answer; compares and counts once both are in.
    fn settle(&self, key: &str, side: Side, answer: Value, groups: &Groups) {
        let done = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let Some(p) = pending.get_mut(key) else {
                return;
            };
            match side {
                Side::Native => p.native = Some(answer),
                Side::Server => p.server = Some(answer),
            }
            if p.native.is_none() || p.server.is_none() {
                return;
            }
            let done = pending.remove(key);
            self.open.store(pending.len(), Ordering::Release);
            done
        };
        let Some(Pending {
            method,
            native: Some(native),
            server: Some(server),
        }) = done
        else {
            return;
        };
        if native == server {
            groups.count(&method, Counted::ShadowMatched);
        } else {
            let at = first_difference(&native, &server);
            warn!(%method, differs_at = %at, "shadow answer differs from the server's");
            groups.count(&method, Counted::ShadowMismatched);
        }
    }

    fn waiting(&self) -> bool {
        self.open.load(Ordering::Acquire) > 0
    }
}

/// The group of the credential and Origin checks.
pub const AUTH_GROUP: &str = "auth";
/// What a credential check is counted as, wherever the credential came from.
pub const AUTH_METHOD: &str = "auth:authenticate";
/// What an Origin check is counted as.
pub const ORIGIN_METHOD: &str = "auth:origin";
/// The close code the server refuses a credential with.
pub const CLOSE_CREDENTIAL_REJECTED: u16 = 4002;
/// The shadow comparison of the credential check, which no request id can
/// name: ids are JSON, quoted or numbers.
const CREDENTIAL: &str = "credential";

/// What [`Conn::offer`] did with a frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Offer {
    /// vornd has it; it is not to be sent to the server.
    Taken,
    /// Send it to the server as it is.
    Pass,
}

/// One client connection's view of the native calls.
pub struct Conn {
    native: Arc<Native>,
    groups: Arc<Groups>,
    reply: Forwarder,
    /// Where a call vornd took after all goes. Weak, so a call still running
    /// never keeps the server's side of a closed connection open.
    upstream: mpsc::WeakSender<Message>,
    authed: AtomicBool,
    shadows: Arc<Shadows>,
}

impl Conn {
    /// `desktop` connections are admitted from the start: vornd checked
    /// their credential itself.
    pub fn new(
        native: Arc<Native>,
        groups: Arc<Groups>,
        reply: Forwarder,
        upstream: &mpsc::Sender<Message>,
        desktop: bool,
    ) -> Arc<Conn> {
        Arc::new(Conn {
            native,
            groups,
            reply,
            upstream: upstream.downgrade(),
            authed: AtomicBool::new(desktop),
            shadows: Arc::default(),
        })
    }

    fn admitted(&self) -> bool {
        self.authed.load(Ordering::Acquire)
    }

    /// Checks the credential a client presents, on its upgrade or in
    /// `auth:authenticate`, by the `auth` group's mode. The server checks it
    /// too, and closes the socket if it refuses, whatever vornd made of it.
    ///
    /// - **native**: a credential vornd admits admits the connection here
    ///   at once, without waiting for the server's word. One it refuses or
    ///   cannot read is the server's to judge.
    /// - **shadow**: vornd's verdict is compared with the server's, which is
    ///   an answer or `auth:ok` (admitted) or a close with
    ///   [`CLOSE_CREDENTIAL_REJECTED`] (refused).
    pub fn check_credential(self: &Arc<Self>, raw: String) {
        let mode = self.groups.mode(AUTH_GROUP);
        if mode == Mode::Forward || self.admitted() {
            self.groups.count(AUTH_METHOD, Counted::Forwarded);
            return;
        }
        if mode == Mode::Shadow {
            self.shadows.begin(CREDENTIAL.to_owned(), AUTH_METHOD);
        }
        let conn = Arc::clone(self);
        tokio::spawn(async move {
            let native = Arc::clone(&conn.native);
            let verdict = tokio::task::spawn_blocking(move || native.verify_credential(&raw))
                .await
                .unwrap_or(reach::Verdict::CannotTell);
            match (mode, verdict) {
                (Mode::Native, reach::Verdict::Admitted) => {
                    conn.authed.store(true, Ordering::Release);
                    conn.groups.count(AUTH_METHOD, Counted::Native);
                }
                (Mode::Native, _) => conn.groups.count(AUTH_METHOD, Counted::Forwarded),
                (_, reach::Verdict::CannotTell) => {
                    conn.groups.count(AUTH_METHOD, Counted::Forwarded);
                    if conn.shadows.cancel(CREDENTIAL).is_some() {
                        conn.groups.count(AUTH_METHOD, Counted::ShadowUnported);
                    }
                }
                (_, verdict) => {
                    conn.groups.count(AUTH_METHOD, Counted::Forwarded);
                    let admitted = verdict == reach::Verdict::Admitted;
                    conn.shadows
                        .settle(CREDENTIAL, Side::Native, json!(admitted), &conn.groups);
                }
            }
        });
    }

    /// The server closed the connection with `code`.
    pub fn on_server_close(&self, code: u16) {
        if code != CLOSE_CREDENTIAL_REJECTED {
            return;
        }
        if self.authed.swap(false, Ordering::AcqRel) {
            warn!("the server refused a credential vornd admitted");
        }
        self.shadows
            .settle(CREDENTIAL, Side::Server, json!(false), &self.groups);
    }

    /// Decides who answers a client's call to `method`, whose frame is `text`.
    pub fn offer(self: &Arc<Self>, method: &str, text: &str) -> Offer {
        if method == AUTH_METHOD {
            if let Some(token) = request_of(text)
                .and_then(|(_, params)| params.get("token")?.as_str().map(str::to_owned))
                .filter(|t| !t.is_empty())
            {
                self.check_credential(token);
            } else {
                self.groups.count(method, Counted::Forwarded);
            }
            return Offer::Pass;
        }
        let mode = self.groups.route(method);
        if mode == Mode::Forward {
            self.groups.count(method, Counted::Forwarded);
            return Offer::Pass;
        }
        let call = effect(method)
            .filter(|_| self.admitted())
            .and_then(|e| request_of(text).map(|(id, params)| (e, id, params)));
        match (mode, call) {
            (Mode::Native, Some((_, id, params))) => {
                self.answer(method.to_owned(), id, params, text.to_owned());
                Offer::Taken
            }
            (Mode::Shadow, Some((Effect::Read, id, params))) => {
                self.groups.count(method, Counted::Forwarded);
                self.shadow(method.to_owned(), id, params);
                Offer::Pass
            }
            (Mode::Shadow, _) => {
                self.groups.count(method, Counted::Forwarded);
                self.groups.count(method, Counted::ShadowUnported);
                Offer::Pass
            }
            _ => {
                self.groups.count(method, Counted::Forwarded);
                Offer::Pass
            }
        }
    }

    /// Runs the call off this task and answers it, or sends it on to the
    /// server when it turns out to be the server's.
    fn answer(self: &Arc<Self>, method: String, id: Value, params: Value, text: String) {
        let conn = Arc::clone(self);
        tokio::spawn(async move {
            let answer = conn.run(method.clone(), params).await;
            match answer.frame(&id) {
                Some(frame) => {
                    conn.groups.count(&method, Counted::Native);
                    conn.reply.send_now(&frame);
                }
                None => {
                    conn.groups.count(&method, Counted::Forwarded);
                    if let Some(tx) = conn.upstream.upgrade() {
                        let _ = tx.send(Message::text(text)).await;
                    }
                }
            }
        });
    }

    fn shadow(self: &Arc<Self>, method: String, id: Value, params: Value) {
        let key = id.to_string();
        self.shadows.begin(key.clone(), &method);
        let conn = Arc::clone(self);
        tokio::spawn(async move {
            let answer = conn.run(method.clone(), params).await;
            match answer.frame(&id) {
                Some(frame) => {
                    conn.shadows
                        .settle(&key, Side::Native, comparable(&frame), &conn.groups);
                }
                None => {
                    if conn.shadows.cancel(&key).is_some() {
                        conn.groups.count(&method, Counted::ShadowUnported);
                    }
                }
            }
        });
    }

    /// The call on a blocking thread, once a slot is free. A panic there is
    /// the server's call to answer, not a crash.
    async fn run(&self, method: String, params: Value) -> Answer {
        let Ok(_slot) = self.native.slots.acquire().await else {
            return Answer::Forward;
        };
        let native = Arc::clone(&self.native);
        let m = method.clone();
        match tokio::task::spawn_blocking(move || native.call(&m, &params)).await {
            Ok(answer) => answer,
            Err(err) => {
                warn!(%method, %err, "a native call failed; the server answers it");
                Answer::Forward
            }
        }
    }

    /// Reads a frame the server sent this client: whether it admits the
    /// connection, and whether it answers a shadowed call. The frame itself
    /// goes to the client unchanged whatever this finds.
    pub fn on_server_text(&self, text: &str) {
        let admitting =
            !self.admitted() && (text.contains("\"result\"") || text.contains("\"auth:ok\""));
        let shadowed = self.shadows.waiting() && text.contains("\"id\"");
        if !admitting && !shadowed {
            return;
        }
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let method = frame.get("method").and_then(Value::as_str);
        let id = frame.get("id").filter(|id| !id.is_null());
        if admitting {
            // The server sends a result, or `auth:ok`, only to a socket it
            // has admitted.
            let answer = method.is_none() && id.is_some() && frame.contains_key("result");
            if answer || (method == Some("auth:ok") && id.is_none()) {
                self.authed.store(true, Ordering::Release);
                self.shadows
                    .settle(CREDENTIAL, Side::Server, json!(true), &self.groups);
            }
        }
        if shadowed && method.is_none() {
            if let Some(id) = id {
                let frame = Value::Object(frame.clone());
                self.shadows.settle(
                    &id.to_string(),
                    Side::Server,
                    comparable(&frame),
                    &self.groups,
                );
            }
        }
    }
}

/// The id and params of a request frame; `None` for a notification or a
/// frame that does not parse.
fn request_of(text: &str) -> Option<(Value, Value)> {
    let Value::Object(mut frame) = serde_json::from_str::<Value>(text).ok()? else {
        return None;
    };
    let id = frame.remove("id").filter(|id| !id.is_null())?;
    let params = frame.remove("params").unwrap_or(Value::Null);
    Some((id, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_answers_as_the_server_does() {
        let id = json!(7);
        assert_eq!(
            Answer::Result(json!(null)).frame(&id),
            Some(json!({ "jsonrpc": "2.0", "id": 7, "result": null }))
        );
        // A call that returns nothing has no result at all, as
        // `JSON.stringify` drops an undefined one.
        assert_eq!(
            Answer::Void.frame(&id),
            Some(json!({ "jsonrpc": "2.0", "id": 7 }))
        );
        assert_eq!(
            Answer::Error("no".into()).frame(&json!("a")),
            Some(
                json!({ "jsonrpc": "2.0", "id": "a", "error": { "code": -32000, "message": "no" } })
            )
        );
        assert_eq!(Answer::Forward.frame(&id), None);
    }

    #[test]
    fn compares_results_and_errors_not_framing() {
        let a = json!({ "jsonrpc": "2.0", "id": 1, "result": [1, 2] });
        let b = json!({ "id": 1, "result": [1, 2], "jsonrpc": "2.0" });
        assert_eq!(comparable(&a), comparable(&b));
        let e = json!({ "id": 1, "error": { "code": -32000, "message": "x", "data": null } });
        assert_eq!(
            comparable(&e),
            json!({ "error": { "code": -32000, "message": "x" } })
        );
    }

    #[test]
    fn says_where_two_answers_differ_without_their_values() {
        let a = json!({ "result": { "files": [{ "diff": "secret a" }], "n": 1 } });
        let b = json!({ "result": { "files": [{ "diff": "secret b" }], "n": 1 } });
        assert_eq!(first_difference(&a, &b), "/result/files/0/diff");
        assert_eq!(first_difference(&a, &a), "");
        assert_eq!(
            first_difference(&json!({ "result": [1] }), &json!({ "result": [1, 2] })),
            "/result/1"
        );
        assert_eq!(first_difference(&json!(1), &json!(2)), "/");
    }

    #[test]
    fn reads_a_request_and_not_a_notification() {
        assert_eq!(
            request_of(r#"{"jsonrpc":"2.0","id":3,"method":"git:getBranch","params":"/r"}"#),
            Some((json!(3), json!("/r")))
        );
        assert_eq!(
            request_of(r#"{"jsonrpc":"2.0","method":"git:getBranch","params":"/r"}"#),
            None
        );
        assert_eq!(request_of(r#"{"id":null,"method":"x"}"#), None);
        assert_eq!(request_of("[1]"), None);
    }

    #[test]
    fn every_server_only_call_is_one_vornd_does_not_answer() {
        for (method, why) in SERVER_ONLY {
            assert_eq!(effect(method), None, "{method}");
            assert!(!why.is_empty());
            assert!(crate::groups::NATIVE_GROUPS.contains(&crate::groups::group_of(method)));
        }
        for (method, _) in METHODS {
            assert!(
                crate::groups::NATIVE_GROUPS.contains(&crate::groups::group_of(method)),
                "{method}"
            );
        }
    }

    #[test]
    fn a_call_without_a_database_goes_to_the_server() {
        let native = Native::new();
        assert_eq!(
            native.call("git:getBranch", &json!("/tmp")),
            Answer::Forward
        );
        assert_eq!(
            native.call("git:checkoutBranch", &json!({})),
            Answer::Forward
        );
        // Relative paths resolve against the server's directory, not vornd's.
        assert_eq!(native.call("git:isGitRepo", &json!("rel")), Answer::Forward);
        assert_eq!(
            native.call(
                "file:stamp",
                &json!({ "filePath": "/x", "remoteHostId": "h" })
            ),
            Answer::Forward
        );
        // An absolute path on this platform, which is answered here.
        let missing = std::env::temp_dir().join("vornd-no-such-file");
        assert_eq!(
            native.call("file:stamp", &json!({ "filePath": missing })),
            Answer::Result(Value::Null)
        );
    }

    #[test]
    fn turns_run_one_change_per_repository_at_a_time() {
        let turns = Arc::new(Turns::default());
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (turns, inside, most) = (turns.clone(), inside.clone(), most.clone());
                std::thread::spawn(move || {
                    turns.take(Path::new("/nowhere/repo"), || {
                        let n = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(n, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }
}
