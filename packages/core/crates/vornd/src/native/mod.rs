//! The calls vornd answers ([`Conn::offer`]); one nothing here answers is refused as unknown.
//!
//! The work runs on blocking threads, at most [`MAX_CONCURRENT`] at a time,
//! and the calls that change a repository take turns per repository
//! ([`Turns`]). Calls on an MCP connection's child wait on the child rather
//! than a thread, and run on the runtime instead. The work model's calls
//! ([`work`]) are async too: they run workflows.
//!
//! Connections and connectors, their secrets included, are vornd's
//! ([`connectors`]).

pub mod about;
pub mod agent;
pub mod config;
pub mod connectors;
pub mod credential;
pub mod desktop;
pub mod env;
pub mod extensions;
pub mod file;
pub mod git;
pub mod headless;
pub mod hooks;
pub mod ide;
pub mod mcp;
pub mod reach;
pub mod remote;
pub mod script;
pub mod secrets;
#[cfg(feature = "engine")]
pub mod session_events;
pub mod sessions;
pub mod shell;
pub mod ssh;
pub mod tasks;
pub mod widget;
pub mod work;
pub mod worktree;
pub mod worktree_move;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};
use vorn_store::{ProjectHosts, Store};

use crate::applink::AppLink;
use crate::registry::SessionRegistry;
use crate::streams::Forwarder;

/// Native calls running at once; the rest wait their turn. A board
/// refreshing thirty diff panels would otherwise start thirty gits together.
pub const MAX_CONCURRENT: usize = 8;

/// The calls vornd answers beside those its modules list.
pub const METHODS: &[&str] = &[
    "git:isGitRepo",
    "git:listBranches",
    "git:listRemoteBranches",
    "git:createWorktree",
    "git:getWorktreeBranch",
    "git:worktreeDirty",
    "git:listWorktrees",
    "git:deleteBranches",
    "git:getBranch",
    "git:diffStat",
    "git:diffFull",
    "git:commit",
    "git:push",
    // Answered once vornd holds the session records ([`worktree_move`]).
    "git:renameWorktreeBranch",
    "git:renameWorktree",
    "git:checkoutBranch",
    "file:listDir",
    "file:readContent",
    "file:stamp",
    "file:writeContent",
    "ide:detect",
    "ide:open",
    "server:reachableUrls",
    "tailscale:status",
    "token:list",
    "token:create",
    "token:revoke",
    "pairing:start",
    "pairing:pending",
    "pairing:approve",
    "pairing:deny",
    "pairing:cancel",
    "agent:detectInstalled",
    "agent:listModels",
    "sessions:getRecent",
    "sessions:restored",
    "sessions:resume",
    "sessions:clear",
    "shell:listExecutables",
    "shell:listInstalled",
    // Answered from the copy of the server's records ([`crate::registry`]),
    // which vornd changes itself for the calls that change a terminal
    // ([`sessions`]) or start and stop a headless agent ([`headless`]);
    // the worktree manager's from it and the repositories ([`worktree`]).
    "terminal:listActive",
    // Answered by `crate::terminal` for the session they name.
    "terminal:attach",
    "terminal:readOutput",
    "terminal:readScrollback",
    "terminal:lockSize",
    "terminal:create",
    "terminal:kill",
    "terminal:rename",
    "terminal:setGroup",
    "terminal:reorder",
    "shell:create",
    "headless:list",
    "headless:create",
    "headless:kill",
    "worktree:activeSessions",
    "worktree:inventory",
    "worktree:removeMany",
    "worktree:reclaimArtifacts",
    "worktree:pruneOrphans",
    "git:removeWorktree",
    // The connector inbox's leases are the work model's ([`work`]).
    "connector:inboxComplete",
    "connector:inboxRenew",
    "config:load",
    "config:save",
];

/// The desktop's main process claiming its connection ([`desktop`]).
pub const IDENTIFY: &str = "bridge:identify";

/// Whether vornd answers `method`.
pub fn answers(method: &str) -> bool {
    work::METHODS.contains(&method)
        || extensions::METHODS.contains(&method)
        || connectors::METHODS.contains(&method)
        || desktop::answers(method)
        || method == IDENTIFY
        || tasks::METHODS.contains(&method)
        || widget::METHODS.contains(&method)
        || about::METHODS.contains(&method)
        || method == script::METHOD
        || credential::METHODS.contains(&method)
        || hooks::METHODS.contains(&method)
        || METHODS.contains(&method)
}

/// The group a method belongs to: everything before the first colon.
pub fn group_of(method: &str) -> &str {
    method.split_once(':').map_or(method, |(group, _)| group)
}

/// What a call came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Result(Value),
    /// A call that returns nothing: the frame has no `result`.
    Void,
    /// The call failed with this message.
    Error(String),
    /// Nothing here answers it: refused as unknown.
    Unanswered,
}

impl Answer {
    /// The frame a client gets for request `id`; `None` when unanswered.
    pub fn frame(&self, id: &Value) -> Option<Value> {
        let mut frame = json!({ "jsonrpc": "2.0", "id": id });
        match self {
            Answer::Result(v) => frame["result"] = v.clone(),
            Answer::Void => {}
            Answer::Error(message) => {
                frame["error"] = json!({ "code": -32000, "message": message });
            }
            Answer::Unanswered => return None,
        }
        Some(frame)
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
    /// remote host, and for how the agents are configured.
    db: OnceLock<PathBuf>,
    slots: Semaphore,
    turns: Turns,
    ignored: file::IgnoreCache,
    ides: ide::Ides,
    reach: reach::Reach,
    /// What the calls share beyond one connection ([`AppLink`]).
    link: OnceLock<Arc<AppLink>>,
    /// The clients vornd serves itself, as the server: notifications go to them.
    clients: OnceLock<Arc<crate::serve::clients::Clients>>,
    /// Raised when vornd itself changed the configuration.
    config_changed: tokio::sync::Notify,
    /// The desktop's launch credential, which is also the server's local one.
    desktop: OnceLock<Vec<u8>>,
    secrets: secrets::Secrets,
    mcp: mcp::McpClients,
    /// The agents' model lists, kept as the server keeps them.
    catalog: vorn_agents::models::Catalog,
    shells: shell::Shells,
    /// The copy of the server's session records, when vornd holds sessions.
    registry: OnceLock<Arc<SessionRegistry>>,
    /// The endpoint agents' hooks post to, once it listens ([`hooks`]).
    hooks: OnceLock<Arc<hooks::Hooks>>,
    /// Woken when a client asks for the widget's list ([`widget`]).
    widget: tokio::sync::Notify,
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
    /// The desktop's main process, which answers the browser and device calls.
    main: Arc<desktop::Desktop>,
}

/// How long a call about what runs waits for the copy to settle as vornd
/// starts: the server's own wait for the holder (`HOLDER_WAIT_MS`).
const SETTLE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a session or script waits for the session holder to connect before it fails.
const HOLDER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

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
            clients: OnceLock::new(),
            config_changed: tokio::sync::Notify::new(),
            desktop: OnceLock::new(),
            secrets,
            mcp: mcp::McpClients::default(),
            catalog: vorn_agents::models::Catalog::default(),
            shells: shell::Shells::default(),
            registry: OnceLock::new(),
            widget: tokio::sync::Notify::new(),
            hooks: OnceLock::new(),
            host: OnceLock::new(),
            sessions: Arc::default(),
            sizes: vorn_worktrees::Sizes::default(),
            work: OnceLock::new(),
            connectors: OnceLock::new(),
            extensions: OnceLock::new(),
            main: Arc::default(),
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

    /// Starts the endpoint agents' hooks post to, once.
    pub async fn start_hooks(self: &Arc<Self>) {
        let Some(homes) = hooks::Homes::from_env() else {
            warn!("no home directory; agents' hooks are not received");
            return;
        };
        match hooks::Hooks::start(self, homes).await {
            Ok(h) => {
                info!(
                    port = h.port(),
                    owner = h.owns_registration(),
                    "the hook endpoint listens"
                );
                let _ = self.hooks.set(h);
            }
            Err(err) => warn!(%err, "the hook endpoint could not listen"),
        }
    }

    /// Denies open permission requests and gives the hook registration up.
    pub fn stop_hooks(&self) {
        if let Some(h) = self.hooks.get() {
            h.stop();
        }
    }

    /// How the hook endpoint is doing, for the health report.
    pub fn hooks_activity(&self) -> Option<Value> {
        self.hooks.get().map(|h| h.activity())
    }

    /// Tells the status widget's list as the registry changes, once there is one.
    pub fn start_widget(self: &Arc<Self>) {
        if let Some(registry) = self.registry.get() {
            tokio::spawn(widget::follow(Arc::clone(self), Arc::clone(registry)));
        }
    }

    /// vornd changed the configuration: what depends on it reads it again.
    pub(crate) fn config_changed(&self) {
        self.config_changed.notify_one();
    }

    /// Resolves at the next [`Native::config_changed`].
    pub(crate) async fn config_change(&self) {
        self.config_changed.notified().await;
    }

    /// How many terminals and headless agents run, from the registry.
    pub fn registry_live(&self) -> Option<Value> {
        self.registry.get().map(|r| r.live())
    }

    /// The clients vornd serves as the server, which notifications go to.
    pub fn set_clients(&self, clients: Arc<crate::serve::clients::Clients>) {
        let _ = self.clients.set(clients);
    }

    pub(crate) fn clients(&self) -> Option<&Arc<crate::serve::clients::Clients>> {
        self.clients.get()
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

    /// The desktop's main process, as vornd reaches it.
    pub fn main_process(&self) -> &Arc<desktop::Desktop> {
        &self.main
    }

    /// What the calls share beyond one connection. Only the first one given is kept.
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
            Some("git") if worktree_move::answers(method) => {
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
                _ => not_answered(method),
            },
            _ => Answer::Unanswered,
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
        if desktop::answers(&method) {
            return match self.main.request(&method, params, desktop::TIMEOUT).await {
                Ok(Some(result)) => Answer::Result(result),
                Ok(None) => Answer::Void,
                Err(message) => Answer::Error(message),
            };
        }
        if config::METHODS.contains(&method.as_str()) {
            return config::answer(self, &method, params, viewer).await;
        }
        if tasks::METHODS.contains(&method.as_str()) {
            return tasks::answer(self, &method, params).await;
        }
        if widget::METHODS.contains(&method.as_str()) {
            return widget::answer(self);
        }
        if about::METHODS.contains(&method.as_str()) {
            return about::answer(self, &method, params).await;
        }
        if method == script::METHOD {
            return script::execute(self, params).await;
        }
        if credential::METHODS.contains(&method.as_str()) {
            return credential::answer(self, &method, params).await;
        }
        if hooks::METHODS.contains(&method.as_str()) {
            return hooks::answer(self, &method, &params);
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
                return Answer::Error("vornd does not run workflows".to_owned());
            };
            let m = method.clone();
            let running = tokio::spawn(async move { work.answer(&m, &params).await });
            return running.await.unwrap_or_else(|err| {
                warn!(%method, %err, "a work call failed");
                Answer::Error(format!("{method} failed in vornd"))
            });
        }
        if connectors::METHODS.contains(&method.as_str()) {
            // Only a vornd with no database starts none.
            let Some(connectors) = self.connectors.get().cloned() else {
                return Answer::Unanswered;
            };
            let m = method.clone();
            let running = tokio::spawn(async move { connectors.answer(&m, params).await });
            return running.await.unwrap_or_else(|err| {
                warn!(%method, %err, "a connector call failed");
                Answer::Error(format!("{method} failed in vornd"))
            });
        }
        if sessions::starts(&method) {
            self.await_holder().await;
        }
        let Ok(_slot) = self.slots.acquire().await else {
            return Answer::Error(format!("{method} failed in vornd: it is stopping"));
        };
        let native = Arc::clone(self);
        let m = method.clone();
        match tokio::task::spawn_blocking(move || native.call(&m, &params)).await {
            Ok(answer) => answer,
            Err(err) => {
                warn!(%method, %err, "a native call failed");
                Answer::Error(format!("{method} failed in vornd"))
            }
        }
    }

    /// Waits, at most [`HOLDER_WAIT`], for the session holder to connect: a
    /// session asked for as vornd starts is started once it is, not refused.
    pub(crate) async fn await_holder(&self) {
        let deadline = tokio::time::Instant::now() + HOLDER_WAIT;
        while !self.host.get().is_some_and(|h| h.ready()) && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// The database, opened for one call; `None` without one or when it cannot be opened.
    fn store(&self) -> Option<Store> {
        let db = self.db.get()?;
        match Store::open_beside(db) {
            Ok(store) => store,
            Err(err) => {
                debug!(%err, "could not open the database");
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
        let host = match params.get("remoteHostId") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(_) => return bad_params(method),
        };
        let place = host.map_or(remote::Place::Local, |id| self.host_place(id));
        // A remote host's paths are POSIX whatever this machine is.
        let path_of = |key: &str| {
            let p = params.get(key)?;
            match place.login() {
                Some(_) => p.as_str().filter(|p| p.starts_with('/')),
                None => absolute_str(p),
            }
        };
        let login = place.login();
        match method {
            "file:listDir" => match path_of("dirPath") {
                Some(dir) => {
                    let entries = match login {
                        Some(login) => file::remote::list_dir(login, dir),
                        None => file::list_dir(dir, &self.env, &self.ignored),
                    };
                    Answer::Result(Value::Array(
                        entries.iter().map(file::FileEntry::to_json).collect(),
                    ))
                }
                None => bad_params(method),
            },
            "file:readContent" => {
                let max = match params.get("maxBytes") {
                    None => Some(file::MAX_READ_BYTES),
                    Some(v) => v.as_u64(),
                };
                match (path_of("filePath"), max) {
                    (Some(path), Some(max)) => Answer::Result(json!(match login {
                        Some(login) => file::remote::read_content(login, path, max),
                        None => file::read_content(path, max),
                    })),
                    _ => bad_params(method),
                }
            }
            "file:stamp" => match path_of("filePath") {
                Some(path) => Answer::Result(
                    match login {
                        Some(login) => file::remote::stamp(login, path),
                        None => file::stamp(path),
                    }
                    .unwrap_or(Value::Null),
                ),
                None => bad_params(method),
            },
            "file:writeContent" => {
                match (
                    path_of("filePath"),
                    params.get("content").and_then(Value::as_str),
                ) {
                    (Some(path), Some(content)) => Answer::Result(match login {
                        Some(login) => file::remote::write_content(login, path, content),
                        None => file::write_content(path, content),
                    }),
                    _ => bad_params(method),
                }
            }
            _ => Answer::Unanswered,
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
    /// through here.
    pub(crate) fn broadcast(&self, method: &str, params: Value) {
        self.broadcast_to(method, params, None);
    }

    /// [`Native::broadcast`] to the clients of one session only, when `scope` names it.
    pub(crate) fn broadcast_to(&self, method: &str, params: Value, scope: Option<&str>) {
        if let Some(notifier) = self.notifier() {
            notifier.tell(method, params, scope);
        }
    }

    /// Where notifications for every client go, to keep beyond this call.
    pub(crate) fn notifier(&self) -> Option<Notifier> {
        self.clients.get().map(|c| Notifier(Arc::clone(c)))
    }

    /// The database, once given.
    pub(crate) fn database(&self) -> Option<&Path> {
        self.db.get().map(PathBuf::as_path)
    }

    /// The calls that read the session records.
    fn sessions(&self, method: &str, params: &Value) -> Answer {
        let Some(registry) = self.registry.get() else {
            return not_ready();
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
                    None => bad_params(method),
                },
                _ => not_answered(method),
            })
            .unwrap_or_else(not_ready)
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
                    _ => bad_params(method),
                }
            }
            _ => not_answered(method),
        }
    }

    /// The projects and hosts, fresh from the database: the server may
    /// have changed them since the last call. `None` when vornd cannot tell.
    fn hosts(&self) -> Option<ProjectHosts> {
        let db = self.db.get()?;
        match ProjectHosts::read(db) {
            Ok(hosts) => Some(hosts.unwrap_or_default()),
            Err(err) => {
                debug!(%err, "could not read the projects");
                None
            }
        }
    }
}

/// Where notifications for every client go: vornd's clients.
#[derive(Debug, Clone)]
pub(crate) struct Notifier(Arc<crate::serve::clients::Clients>);

impl Notifier {
    /// Tells `method` with `params`; `scope` is the session it is about.
    pub(crate) fn tell(&self, method: &str, params: Value, scope: Option<&str>) {
        self.0.broadcast(method, params, scope);
    }
}

/// A record as the server's handler returns it. Serializing a plain struct
/// of strings and numbers does not fail.
fn json_of<T: serde::Serialize>(record: T) -> Value {
    serde_json::to_value(record).unwrap_or(Value::Null)
}

/// The error while vornd has no database, which only a test's vornd lacks.
/// The error for a call in one of vornd's groups that it has no answer for.
pub(crate) fn not_answered(method: &str) -> Answer {
    Answer::Error(format!("vornd does not answer {method}"))
}

/// The error while vornd's copy of the session records is not fed yet.
pub(crate) fn not_ready() -> Answer {
    Answer::Error("vornd has not read the sessions yet".to_owned())
}

pub(crate) fn no_database() -> Answer {
    Answer::Error("vornd has no database".to_owned())
}

/// The error for params of a shape `method`'s handler cannot read.
pub(crate) fn bad_params(method: &str) -> Answer {
    Answer::Error(format!("{method} cannot read the params it was given"))
}

/// A string param that is an absolute path. A relative one would resolve
/// against vornd's working directory rather than the server's.
fn absolute_str(v: &Value) -> Option<&str> {
    // A POSIX path is absolute anywhere: a remote host's paths are POSIX.
    v.as_str()
        .filter(|p| p.starts_with('/') || Path::new(p).is_absolute())
}

/// What a credential check is called, wherever the credential came from.
pub const AUTH_METHOD: &str = "auth:authenticate";

/// What [`Conn::offer`] did with a frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Offer {
    /// vornd has it, and answers it.
    Taken,
    /// Nothing here takes it: refused as unknown.
    Pass,
}

/// One client connection's view of the calls.
pub struct Conn {
    /// The connection's id in [`crate::streams`].
    id: u64,
    native: Arc<Native>,
    reply: Forwarder,
    authed: AtomicBool,
    /// Who this connection is, as its credential says.
    viewer: Mutex<config::Viewer>,
}

impl Conn {
    /// `desktop` connections are admitted from the start: vornd checked
    /// their credential itself.
    pub fn new(id: u64, native: Arc<Native>, reply: Forwarder, desktop: bool) -> Arc<Conn> {
        Arc::new(Conn {
            id,
            native,
            reply,
            authed: AtomicBool::new(desktop),
            viewer: Mutex::new(if desktop {
                config::Viewer::Desktop
            } else {
                config::Viewer::Local
            }),
        })
    }

    fn admitted(&self) -> bool {
        self.authed.load(Ordering::Acquire)
    }

    /// Admits the connection, which presented `raw`, once vornd has checked it.
    pub fn admit(&self, raw: &str) {
        *self.viewer.lock().unwrap_or_else(|e| e.into_inner()) = self.native.viewer_of(raw);
        self.authed.store(true, Ordering::Release);
    }

    /// Takes a client's call to `method`, whose frame is `text`, when vornd answers it.
    pub fn offer(self: &Arc<Self>, method: &str, text: &str) -> Offer {
        if method == AUTH_METHOD || !self.admitted() {
            return Offer::Pass;
        }
        if method == extensions::SELECTION_RESULT {
            if let Some(host) = self.native.extensions.get() {
                let params = serde_json::from_str::<Value>(text)
                    .ok()
                    .and_then(|mut frame| frame.get_mut("params").map(Value::take))
                    .unwrap_or_default();
                host.resolve_selection(&params);
                return Offer::Taken;
            }
        }
        if method == IDENTIFY {
            self.identify(text);
            return Offer::Taken;
        }
        match request_of(text).filter(|_| answers(method)) {
            Some((id, params)) => {
                self.answer(method.to_owned(), id, params);
                Offer::Taken
            }
            None => Offer::Pass,
        }
    }

    /// Main claims this connection as its own and hears whether it holds it.
    fn identify(&self, text: &str) {
        let claimed = self.native.main.claim(self.id, &self.reply);
        if let Some((id, _)) = request_of(text) {
            self.reply
                .send_now(&json!({ "jsonrpc": "2.0", "id": id, "result": { "ok": claimed } }));
        }
    }

    /// Whether `text`, a frame without a method, answers a call vornd made
    /// of main; if so it is settled here.
    pub fn settle_desktop(&self, text: &str) -> bool {
        self.native.main.settle(self.id, text)
    }

    /// The client's side closed: main's calls fail now if this was main.
    pub fn closed(&self) {
        self.native.main.release(self.id);
    }

    /// Runs the call off this task and answers it.
    fn answer(self: &Arc<Self>, method: String, id: Value, params: Value) {
        let conn = Arc::clone(self);
        tokio::spawn(async move {
            let viewer = conn
                .viewer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let answer = conn.native.answer(method.clone(), params, &viewer).await;
            let frame = answer.frame(&id).unwrap_or_else(|| {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Method not found: {method}") },
                })
            });
            conn.reply.send_now(&frame);
        });
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
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn frames_answers_as_clients_expect() {
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
        assert_eq!(Answer::Unanswered.frame(&id), None);
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
    fn answers_the_calls_its_modules_list() {
        for method in METHODS.iter().chain(work::METHODS) {
            assert!(answers(method), "{method}");
        }
        for method in ["server:shutdown", "subscribe:set", "nope:never"] {
            assert!(!answers(method), "{method}");
        }
        assert_eq!(group_of("git:status"), "git");
        assert_eq!(group_of("ping"), "ping");
    }

    #[test]
    fn reads_the_session_registry_once_the_server_has_fed_it() {
        let native = Native::new();
        assert_eq!(
            native.call("terminal:listActive", &Value::Null),
            not_ready()
        );
        let registry = SessionRegistry::new();
        native.set_registry(Arc::clone(&registry));
        assert!(registry.wanted());
        // Not read yet: vornd cannot tell what runs.
        assert_eq!(native.call("headless:list", &Value::Null), not_ready());

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
        // Params the server's handler would not expect.
        assert_eq!(
            native.call("worktree:activeSessions", &json!({ "path": "/w" })),
            bad_params("worktree:activeSessions")
        );
        assert_eq!(
            native.call("terminal:create", &json!({})),
            bad_params("terminal:create")
        );
    }

    #[test]
    fn a_call_without_a_database_is_answered_as_on_this_machine() {
        let native = Native::new();
        let outside = std::env::temp_dir().join("vornd-no-such-repo");
        assert_eq!(
            native.call("git:getBranch", &json!(outside)),
            Answer::Result(Value::Null)
        );
        // A relative path is read from where vornd runs, as the server read it from where it ran.
        assert_eq!(
            native.call("git:isGitRepo", &json!("vornd-no-such-dir")),
            Answer::Result(json!(false))
        );
        assert_eq!(
            native.call("git:isGitRepo", &json!(3)),
            bad_params("git:isGitRepo")
        );
        // A host vornd cannot read is reached as this machine, as the server reaches it.
        assert_eq!(
            native.call(
                "file:stamp",
                &json!({ "filePath": "/vornd-no-such-file", "remoteHostId": "h" })
            ),
            Answer::Result(Value::Null)
        );
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
