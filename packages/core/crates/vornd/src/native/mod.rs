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
//!   never sees vornd's answer. A create (`terminal:create`,
//!   `shell:create`, `headless:create`) is not run twice either; what
//!   vornd would start for it is worked out instead ([`sessions::plan`])
//!   and compared with the spawn the server asks for and the record it
//!   answers.
//!
//! The work runs on blocking threads, at most [`MAX_CONCURRENT`] at a time,
//! and the calls that change a repository take turns per repository
//! ([`Turns`]), as the server's do. Calls on an MCP connection's child wait on
//! the child rather than a thread, and run on the runtime instead. The work
//! model's calls ([`work`]) are async too: they run workflows.
//!
//! Connections and connectors, their secrets included, are vornd's
//! ([`connectors`]).

pub mod agent;
pub mod config;
pub mod connectors;
pub mod env;
pub mod extensions;
pub mod file;
pub mod git;
pub mod headless;
pub mod ide;
pub mod mcp;
pub mod reach;
pub mod script;
pub mod secrets;
pub mod sessions;
pub mod shell;
pub mod ssh;
pub mod work;
pub mod worktree;
pub mod worktree_move;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};
use vorn_store::{Placement, ProjectHosts, Store};

use crate::applink::AppLink;
use crate::groups::{Counted, Groups, Mode};
use crate::registry::{Registry, SessionRegistry};
use crate::streams::Forwarder;

/// How long the comparison of a create's plan waits for the spawn the
/// server asks for after it answers: it asks once the session holder is up,
/// which a cold start can take seconds over.
const SPAWN_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

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
/// fetches, `ide:open` starts an editor, `agent:listModels` starts the
/// agent's CLI, and the two connection calls that start a child run its
/// tools, so none of them runs twice.
pub const METHODS: &[(&str, Effect)] = &[
    ("git:isGitRepo", Effect::Read),
    ("git:listBranches", Effect::Read),
    ("git:listRemoteBranches", Effect::Change),
    ("git:createWorktree", Effect::Change),
    ("git:getWorktreeBranch", Effect::Read),
    ("git:worktreeDirty", Effect::Read),
    ("git:listWorktrees", Effect::Read),
    ("git:deleteBranches", Effect::Change),
    ("git:getBranch", Effect::Read),
    ("git:diffStat", Effect::Read),
    ("git:diffFull", Effect::Read),
    ("git:commit", Effect::Change),
    ("git:push", Effect::Change),
    // Answered once vornd holds the session records ([`worktree_move`]).
    ("git:renameWorktreeBranch", Effect::Change),
    ("git:renameWorktree", Effect::Change),
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
    ("sessions:restored", Effect::Read),
    ("sessions:resume", Effect::Change),
    ("sessions:clear", Effect::Change),
    ("shell:listExecutables", Effect::Read),
    ("shell:listInstalled", Effect::Read),
    // Answered from the copy of the server's records ([`crate::registry`]),
    // which vornd changes itself for the calls that change a terminal
    // ([`sessions`]) or start and stop a headless agent ([`headless`]);
    // the worktree manager's from it and the repositories ([`worktree`]).
    ("terminal:listActive", Effect::Read),
    ("terminal:create", Effect::Change),
    ("terminal:kill", Effect::Change),
    ("terminal:rename", Effect::Change),
    ("terminal:setGroup", Effect::Change),
    ("terminal:reorder", Effect::Change),
    ("shell:create", Effect::Change),
    ("headless:list", Effect::Read),
    ("headless:create", Effect::Change),
    ("headless:kill", Effect::Change),
    ("worktree:activeSessions", Effect::Read),
    ("worktree:inventory", Effect::Read),
    ("worktree:removeMany", Effect::Change),
    ("worktree:reclaimArtifacts", Effect::Change),
    ("worktree:pruneOrphans", Effect::Change),
    ("git:removeWorktree", Effect::Change),
    // The connector inbox's leases are the work model's ([`work`]).
    ("connector:inboxComplete", Effect::Change),
    ("connector:inboxRenew", Effect::Change),
    ("config:load", Effect::Read),
    ("config:save", Effect::Change),
];

/// Calls in a native group that the server keeps answering, and why.
pub const SERVER_ONLY: &[(&str, &str)] = &[
    (
        "git:checkoutBranch",
        "moves the server's sessions on that worktree to the new branch and tells clients",
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
];

/// The effect of a call vornd answers, or `None` for one it does not. The
/// work model's calls are all answered here, never compared.
pub fn effect(method: &str) -> Option<Effect> {
    if work::METHODS.contains(&method) || extensions::METHODS.contains(&method) {
        return Some(Effect::Change);
    }
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
    secrets: secrets::Secrets,
    mcp: mcp::McpClients,
    /// The agents' model lists, kept as the server keeps them.
    catalog: vorn_agents::models::Catalog,
    shells: shell::Shells,
    /// The copy of the server's session records, when vornd holds sessions.
    registry: OnceLock<Arc<SessionRegistry>>,
    /// What starts the sessions vornd creates: the engine, when it runs one.
    host: OnceLock<Arc<dyn sessions::Host>>,
    sessions: Arc<sessions::Sessions>,
    /// The worktrees' sizes, measured by the inventory and kept for the
    /// actions that report what they freed.
    sizes: vorn_worktrees::Sizes,
    /// The work model, once vornd has a database and its own address.
    work: OnceLock<Arc<work::Work>>,
    /// Connections and connectors, once vornd has a database and its own address.
    connectors: OnceLock<Arc<connectors::Connectors>>,
    /// The extension host, once vornd has a database and its own address.
    extensions: OnceLock<Arc<extensions::Extensions>>,
}

/// How long a call about what runs waits for the copy to settle as vornd
/// starts: the server's own wait for the holder (`HOLDER_WAIT_MS`).
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The calls that say what runs and what is offered from the last run, which
/// wait for the copy to settle ([`SessionRegistry::settle`]): a window opening
/// as Vorn starts reads its board from them, and resumes what they offer.
fn reads_what_runs(method: &str) -> bool {
    matches!(
        method,
        "terminal:listActive" | "headless:list" | "sessions:restored" | "sessions:resume"
    )
}

impl Native {
    pub fn new() -> Arc<Native> {
        Native::with_secrets(secrets::Secrets::new())
    }

    fn with_secrets(secrets: secrets::Secrets) -> Arc<Native> {
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
            secrets,
            mcp: mcp::McpClients::default(),
            catalog: vorn_agents::models::Catalog::default(),
            shells: shell::Shells::default(),
            registry: OnceLock::new(),
            host: OnceLock::new(),
            sessions: Arc::default(),
            sizes: vorn_worktrees::Sizes::default(),
            work: OnceLock::new(),
            connectors: OnceLock::new(),
            extensions: OnceLock::new(),
        })
    }

    /// What starts the sessions vornd creates. Only the first one given is
    /// kept; without one, the server creates them.
    pub fn set_host(&self, host: Arc<dyn sessions::Host>) {
        let _ = self.host.set(host);
    }

    /// The copy of the server's session records to answer from, which this
    /// asks the server to feed. Only the first one given is kept.
    pub fn set_registry(&self, registry: Arc<SessionRegistry>) {
        registry.want();
        let _ = self.registry.set(registry);
    }

    /// The server's port, which the addresses a browser uses name.
    pub fn set_server_port(&self, port: u16) {
        self.reach.set_server_port(port);
    }

    /// The work model. Only the first one given is kept.
    pub fn set_work(&self, work: Arc<work::Work>) {
        let _ = self.work.set(work);
    }

    pub fn work(&self) -> Option<&Arc<work::Work>> {
        self.work.get()
    }

    /// Connections and connectors. Only the first one given is kept.
    pub fn set_connectors(&self, connectors: Arc<connectors::Connectors>) {
        let _ = self.connectors.set(connectors);
    }

    pub fn connectors(&self) -> Option<&Arc<connectors::Connectors>> {
        self.connectors.get()
    }

    /// The environment a script step's named connection gives it, from the vault.
    pub(crate) fn script_secrets(&self, connection_id: &str) -> Vec<(String, String)> {
        match self.secrets.lookup(connection_id) {
            secrets::Known::Fields(fields) => {
                let plain: Vec<(String, String)> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.expose().to_owned()))
                    .collect();
                vorn_connectors::connections::script_env(&plain)
            }
            _ => Vec::new(),
        }
    }

    /// The extension host. Only the first one given is kept.
    pub fn set_extensions(&self, extensions: Arc<extensions::Extensions>) {
        let _ = self.extensions.set(extensions);
    }

    pub fn extensions(&self) -> Option<&Arc<extensions::Extensions>> {
        self.extensions.get()
    }

    /// The environment vornd's children start from.
    pub(crate) fn child_env(&self) -> env::Env {
        self.env.get()
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
        if let Some(dir) = db.parent() {
            self.secrets.settle(dir);
        }
        let _ = self.db.set(db);
    }

    /// Asks the login shell for its environment, in the background.
    pub fn prepare(&self) {
        self.env.prime();
    }

    /// Answers `method` with `params`, blocking this thread meanwhile. The
    /// calls [`connection::is_async`] names are answered by [`Native::answer`]
    /// only.
    pub fn call(&self, method: &str, params: &Value) -> Answer {
        if reads_what_runs(method) {
            if let Some(registry) = self.registry.get() {
                registry.settle(SETTLE_LIMIT);
            }
        }
        match method.split_once(':').map(|(g, _)| g) {
            Some("git") if method == "git:removeWorktree" => worktree::call(self, method, params),
            Some("worktree") if method != "worktree:activeSessions" => {
                worktree::call(self, method, params)
            }
            Some("git") if worktree_move::foresees(method) => {
                worktree_move::call(self, method, params)
            }
            Some("git") => git::call(self, method, params),
            Some("file") => self.file(method, params),
            Some("ide") => self.ide(method, params),
            Some("server" | "tailscale" | "token" | "pairing") => self.reach_call(method, params),
            Some("sessions") if method != "sessions:getRecent" => {
                sessions::call(self, method, params)
            }
            Some("agent" | "sessions") => agent::call(self, method, params),
            Some("terminal") if method != "terminal:listActive" => {
                sessions::call(self, method, params)
            }
            Some("headless") if method != "headless:list" => headless::call(self, method, params),
            Some("terminal" | "headless" | "worktree") => self.sessions(method, params),
            Some("shell") => match method {
                "shell:listExecutables" => Answer::Result(self.shells.executables(&self.env)),
                "shell:listInstalled" => Answer::Result(self.shells.installed()),
                "shell:create" => sessions::call(self, method, params),
                _ => Answer::Forward,
            },
            _ => Answer::Forward,
        }
    }

    /// Answers `method` with `params`: on a blocking thread once a slot is
    /// free, or on the runtime for a call that waits on an MCP child. A
    /// panic is the server's call to answer, not a crash, unless the call
    /// may already have changed something.
    pub async fn answer(
        self: &Arc<Self>,
        method: String,
        params: Value,
        viewer: &config::Viewer,
    ) -> Answer {
        if config::METHODS.contains(&method.as_str()) {
            return config::answer(self, &method, params, viewer).await;
        }
        if extensions::METHODS.contains(&method.as_str()) {
            let Some(host) = self.extensions.get().cloned() else {
                return Answer::Error("vornd is not hosting extensions".to_owned());
            };
            let m = method.clone();
            let running = tokio::spawn(async move { host.answer(&m, &params).await });
            return running.await.unwrap_or_else(|err| {
                warn!(%method, %err, "an extension call failed");
                Answer::Error(format!("{method} failed in vornd"))
            });
        }
        if work::is_work(&method)
            || matches!(
                method.as_str(),
                "connector:inboxComplete" | "connector:inboxRenew"
            )
        {
            let Some(work) = self.work.get().cloned() else {
                return Answer::Forward;
            };
            let m = method.clone();
            let running = tokio::spawn(async move { work.answer(&m, &params).await });
            return running.await.unwrap_or_else(|err| {
                warn!(%method, %err, "a work call failed");
                Answer::Error(format!("{method} failed in vornd"))
            });
        }
        if connectors::METHODS.contains(&method.as_str()) {
            let Some(connectors) = self.connectors.get().cloned() else {
                return Answer::Forward;
            };
            let m = method.clone();
            let running = tokio::spawn(async move { connectors.answer(&m, params).await });
            return running.await.unwrap_or_else(|err| {
                warn!(%method, %err, "a connector call failed");
                Answer::Error(format!("{method} failed in vornd"))
            });
        }
        let Ok(_slot) = self.slots.acquire().await else {
            return Answer::Forward;
        };
        let native = Arc::clone(self);
        let m = method.clone();
        match tokio::task::spawn_blocking(move || native.call(&m, &params)).await {
            Ok(answer) => answer,
            Err(err) => {
                warn!(%method, %err, "a native call failed; the server answers it");
                Answer::Forward
            }
        }
    }

    /// The server's database, opened beside it for one call. `None` without
    /// one, and when it cannot be opened (the server answers then).
    fn store(&self) -> Option<Store> {
        let db = self.db.get()?;
        match Store::open_beside(db) {
            Ok(store) => store,
            Err(err) => {
                debug!(%err, "could not open the database; the server answers");
                None
            }
        }
    }

    /// `dbSignalChange`: tells the server, and any other process watching
    /// the data directory, that the configuration changed.
    fn signal_change(&self) {
        let Some(dir) = self.db.get().and_then(|db| db.parent()) else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        if let Err(err) = std::fs::write(dir.join(".db-signal"), now.to_string()) {
            debug!(%err, "could not signal a configuration change");
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

    /// The terminals and headless agents in the copy of the session
    /// records, as clients get them; `None` while there is no copy.
    pub(crate) fn session_records(&self) -> Option<(Vec<Value>, Vec<Value>)> {
        self.registry.get()?.read(|r| {
            (
                r.terminals().into_iter().map(json_of).collect(),
                r.headless().map(json_of).collect(),
            )
        })
    }

    /// The server's port, which the addresses clients open name.
    pub(crate) fn server_port(&self) -> Option<u16> {
        self.reach.server_port()
    }

    /// Tells every client `method`. Every broadcast vornd makes goes
    /// through here; for now the server, which holds the clients, sends it.
    pub(crate) fn broadcast(&self, method: &str, params: Value) {
        self.broadcast_to(method, params, None);
    }

    /// [`Native::broadcast`] to the clients of one session only, when `scope` names it.
    pub(crate) fn broadcast_to(&self, method: &str, params: Value, scope: Option<&str>) {
        if let Some(link) = self.link.get() {
            let mut note = json!({ "method": method, "params": params });
            if let Some(scope) = scope {
                note["scope"] = json!(scope);
            }
            link.tell("vornd:broadcast", note);
        }
    }

    /// The database, once given.
    pub(crate) fn database(&self) -> Option<&Path> {
        self.db.get().map(PathBuf::as_path)
    }

    /// The calls that read the server's session registry, from vornd's copy
    /// of it. Forwarded while there is no copy to trust.
    fn sessions(&self, method: &str, params: &Value) -> Answer {
        let Some(registry) = self.registry.get() else {
            return Answer::Forward;
        };
        let records = |list: Vec<Value>| Answer::Result(Value::Array(list));
        registry
            .read(|r| match method {
                "terminal:listActive" => records(r.terminals().into_iter().map(json_of).collect()),
                "headless:list" => records(r.headless().map(json_of).collect()),
                "worktree:activeSessions" => match params.as_str() {
                    Some(path) => {
                        let ids = r.active_in_worktree(path);
                        Answer::Result(json!({ "count": ids.len(), "sessionIds": ids }))
                    }
                    None => Answer::Forward,
                },
                _ => Answer::Forward,
            })
            .unwrap_or(Answer::Forward)
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

/// A record as the server's handler returns it. Serializing a plain struct
/// of strings and numbers does not fail.
fn json_of<T: serde::Serialize>(record: T) -> Value {
    serde_json::to_value(record).unwrap_or(Value::Null)
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
        let (native, server) = (
            worktree::compared(&method, native),
            worktree::compared(&method, server),
        );
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
/// The error code the server answers a call on a socket it has not admitted with.
const NOT_AUTHENTICATED: i64 = -32001;
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
    /// Who this connection is, as its credential says.
    viewer: Mutex<config::Viewer>,
    shadows: Arc<Shadows>,
    /// Shadowed creates whose plan is compared, by request id.
    plans: Mutex<HashMap<String, Planned>>,
}

/// A create shadowed, kept until the server answers it and asks for its
/// spawn: what vornd would start is worked out then ([`Conn::planned_answer`]).
#[derive(Debug)]
struct Planned {
    method: String,
    params: Value,
    /// How many shells there were when the call came, before the server's
    /// answer adds one: a new shell is numbered after them.
    shells: usize,
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
            viewer: Mutex::new(if desktop {
                config::Viewer::Desktop
            } else {
                config::Viewer::Local
            }),
            shadows: Arc::default(),
            plans: Mutex::default(),
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
    ///
    /// The check runs off this task; what it returns ends when it is done,
    /// which the upgrade's first frame waits for.
    pub fn check_credential(self: &Arc<Self>, raw: String) -> Option<tokio::task::JoinHandle<()>> {
        let mode = self.groups.mode(AUTH_GROUP);
        if mode == Mode::Forward || self.admitted() {
            self.groups.count(AUTH_METHOD, Counted::Forwarded);
            return None;
        }
        if mode == Mode::Shadow {
            self.shadows.begin(CREDENTIAL.to_owned(), AUTH_METHOD);
        }
        // Who it is counts only once admitted, by vornd or by the server, which may answer first.
        *self.viewer.lock().unwrap_or_else(|e| e.into_inner()) = self.native.viewer_of(&raw);
        let conn = Arc::clone(self);
        Some(tokio::spawn(async move {
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
        }))
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
                // The server answers `auth:ok`, which admits the connection here too.
                let _ = self.check_credential(token);
            } else {
                self.groups.count(method, Counted::Forwarded);
            }
            return Offer::Pass;
        }
        // Before a connection is admitted the server refuses it, whatever it asks.
        if !self.admitted() {
            self.groups.count(method, Counted::BeforeAuth);
            return Offer::Pass;
        }
        if crate::groups::unknown(method) {
            if let Some((id, _)) = request_of(text) {
                let frame = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Method not found: {method}") },
                });
                self.reply.send_now(&frame);
            }
            self.groups.count(method, Counted::Native);
            return Offer::Taken;
        }
        let mode = self.groups.route(method);
        if mode == Mode::Forward {
            self.groups.count(method, Counted::Forwarded);
            return Offer::Pass;
        }
        if method == extensions::SELECTION_RESULT && mode == Mode::Native && self.admitted() {
            if let Some(host) = self.native.extensions.get() {
                let params = serde_json::from_str::<Value>(text)
                    .ok()
                    .and_then(|mut frame| frame.get_mut("params").map(Value::take))
                    .unwrap_or_default();
                host.resolve_selection(&params);
                self.groups.count(method, Counted::Native);
                return Offer::Taken;
            }
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
            (Mode::Shadow, Some((Effect::Change, id, params))) if sessions::plans(method) => {
                self.groups.count(method, Counted::Forwarded);
                self.plan(method.to_owned(), id, params);
                Offer::Pass
            }
            (Mode::Shadow, Some((Effect::Change, id, params)))
                if sessions::foresees(method) || worktree::foresees(method) =>
            {
                self.groups.count(method, Counted::Forwarded);
                self.foresee(method, &id, &params);
                Offer::Pass
            }
            (Mode::Shadow, _) if script::compared_by_server(method) => {
                self.groups.count(method, Counted::Forwarded);
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

    async fn run(&self, method: String, params: Value) -> Answer {
        let viewer = self
            .viewer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.native.answer(method, params, &viewer).await
    }

    /// Keeps a create the server answers, to work out what vornd would have
    /// started for it once the server has ([`Conn::planned_answer`]). The
    /// shells there are now are read before the server's answer can add one.
    fn plan(&self, method: String, id: Value, params: Value) {
        let Some(shells) = self
            .native
            .registry
            .get()
            .and_then(|r| r.read(Registry::shells))
        else {
            return self.groups.count(&method, Counted::ShadowUnported);
        };
        let key = id.to_string();
        self.shadows.begin(key.clone(), &method);
        self.lock_plans().insert(
            key,
            Planned {
                method,
                params,
                shells,
            },
        );
    }

    /// Files what vornd would answer a call that changes a terminal, read
    /// without making the change, for the comparison with the server's
    /// answer when it comes.
    fn foresee(&self, method: &str, id: &Value, params: &Value) {
        let frame = sessions::foresee(&self.native, method, params)
            .or_else(|| worktree::foresee(&self.native, method, params))
            .and_then(|a| a.frame(id));
        let Some(frame) = frame else {
            return self.groups.count(method, Counted::ShadowUnported);
        };
        let key = id.to_string();
        self.shadows.begin(key.clone(), method);
        self.shadows
            .settle(&key, Side::Native, comparable(&frame), &self.groups);
    }

    /// A shadowed call with nothing native to compare: counted so.
    fn unported(&self, key: &str, method: &str) {
        self.lock_plans().remove(key);
        if self.shadows.cancel(key).is_some() {
            self.groups.count(method, Counted::ShadowUnported);
        }
    }

    fn lock_plans(&self) -> std::sync::MutexGuard<'_, HashMap<String, Planned>> {
        self.plans.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The server answered a create whose plan is compared: its side is the
    /// spawn it asked vornd for under the record's id, with the record.
    /// vornd's side is worked out only then, after the server's start, which
    /// writes what a launch reads (a shell's shims) as it goes.
    fn planned_answer(
        self: &Arc<Self>,
        key: String,
        planned: Planned,
        frame: &serde_json::Map<String, Value>,
    ) {
        // A resume answers `{ok, session}`; a create, the record itself.
        let record = frame
            .get("result")
            .map(|r| r.get("session").cloned().unwrap_or_else(|| r.clone()));
        let name = record
            .as_ref()
            .and_then(|r| r.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (Some(record), Some(name), Some(link)) = (record, name, self.native.link.get()) else {
            return self.unported(&key, &planned.method);
        };
        let (conn, link) = (Arc::clone(self), Arc::clone(link));
        tokio::spawn(async move {
            let Some(spawn) = link.spawned(&name, SPAWN_WAIT).await else {
                return conn.unported(&key, &planned.method);
            };
            let native = Arc::clone(&conn.native);
            let Planned {
                method,
                params,
                shells,
            } = planned;
            let m = method.clone();
            let ours =
                tokio::task::spawn_blocking(move || sessions::plan(&native, &m, &params, shells))
                    .await
                    .ok()
                    .flatten();
            let Some(ours) = ours else {
                return conn.unported(&key, &method);
            };
            conn.shadows.settle(&key, Side::Native, ours, &conn.groups);
            let theirs = sessions::spawn_plan(&spawn, &record);
            conn.shadows
                .settle(&key, Side::Server, theirs, &conn.groups);
        });
    }

    /// Reads a frame the server sent this client: whether it admits the
    /// connection, and whether it answers a shadowed call. The frame itself
    /// goes to the client unchanged whatever this finds.
    pub fn on_server_text(self: &Arc<Self>, text: &str) {
        let admitting = !self.admitted()
            && (text.contains("\"result\"")
                || text.contains("\"error\"")
                || text.contains("\"auth:ok\""));
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
            // The server answers, or sends `auth:ok`, only to a socket it has
            // admitted: any other gets its not-authenticated refusal.
            let refused = frame
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_i64)
                == Some(NOT_AUTHENTICATED);
            let answer = method.is_none()
                && id.is_some()
                && !refused
                && (frame.contains_key("result") || frame.contains_key("error"));
            if answer || (method == Some("auth:ok") && id.is_none()) {
                self.authed.store(true, Ordering::Release);
                self.shadows
                    .settle(CREDENTIAL, Side::Server, json!(true), &self.groups);
            }
        }
        if shadowed && method.is_none() {
            if let Some(key) = id.map(Value::to_string) {
                if let Some(planned) = self.lock_plans().remove(&key) {
                    self.planned_answer(key, planned, &frame);
                    return;
                }
            }
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
            let group = crate::groups::group_of(method);
            assert!(crate::groups::NATIVE_GROUPS.contains(&group), "{method}");
        }
        for method in work::METHODS {
            assert_eq!(effect(method), Some(Effect::Change), "{method}");
            assert!(
                crate::groups::NATIVE_GROUPS.contains(&crate::groups::group_of(method)),
                "{method}"
            );
        }
    }

    #[test]
    fn reads_the_session_registry_once_the_server_has_fed_it() {
        let native = Native::new();
        assert_eq!(
            native.call("terminal:listActive", &Value::Null),
            Answer::Forward
        );
        let registry = SessionRegistry::new();
        native.set_registry(Arc::clone(&registry));
        assert!(registry.wanted());
        // Not fed yet: vornd cannot tell what the server holds.
        assert_eq!(native.call("headless:list", &Value::Null), Answer::Forward);

        let terminal = json!({
            "id": "t", "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 5, "pid": 9, "worktreePath": "/w",
        });
        registry
            .feed(
                1,
                &json!({ "op": "snapshot", "terminals": [terminal.clone()], "headless": [] }),
            )
            .unwrap();
        assert_eq!(
            native.call("terminal:listActive", &Value::Null),
            Answer::Result(json!([terminal]))
        );
        assert_eq!(
            native.call("headless:list", &Value::Null),
            Answer::Result(json!([]))
        );
        assert_eq!(
            native.call("worktree:activeSessions", &json!("/w")),
            Answer::Result(json!({ "count": 1, "sessionIds": ["t"] }))
        );
        // Params the server's handler would not expect: the server says so.
        assert_eq!(
            native.call("worktree:activeSessions", &json!({ "path": "/w" })),
            Answer::Forward
        );
        assert_eq!(native.call("terminal:create", &json!({})), Answer::Forward);
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
