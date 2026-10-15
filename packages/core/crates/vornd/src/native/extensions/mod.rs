//! Extensions, hosted here: the programs that speak the extension API, which
//! the bridge's [`Router`] calls, and the extensions of 0.8 that add to a
//! card, one process per extension and project; installing, linking and
//! removing them; the `extension:*` calls clients make about them, the bands
//! and panes they show, and the bridge a card extension speaks back on
//! ([`routes`]). Every process runs under the one lifecycle vornd holds.
//!
//! The rules are [`vorn_extensions`]'s. This module runs them against what
//! vornd has around it ([`Around`]): the session records, the session host,
//! its own endpoint and the clients the server holds.

pub mod routes;
pub mod world;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use vorn_extensions::activation::{self, Subject};
use vorn_extensions::api::types::CallCtx;
use vorn_extensions::api::ExtError;
use vorn_extensions::footer::{Declared, Footers, Key};
use vorn_extensions::grants::{Grant, Grants, Lookup, GRANT_IDLE_MS};
use vorn_extensions::host::{Asked, Host, HostKey, Hosts};
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::Lease;
use vorn_extensions::manifest::PaneDraws;
use vorn_extensions::pack::InstalledPack;
use vorn_extensions::router::Router;
use vorn_extensions::{links, token};
use vorn_git::repo::Git;
use vorn_sessiond_wire::{Io, SpawnSpec};

use super::Answer;

/// Every call clients make that is answered here.
pub const METHODS: &[&str] = &[
    "extension:list",
    "extension:activation",
    "extension:footerItems",
    "extension:openPane",
    "extension:closePane",
    "extension:runHandler",
    "extension:matchLinks",
    "extension:installed",
    "extension:inspect",
    "extension:install",
    "extension:link",
    "extension:remove",
    "extension:call",
    "extension:rows",
    "extension:running",
];

/// Core method groups, which no extension id may take: `<id>:<action>` routes to an extension.
pub const RESERVED: &[&str] = &[
    "agent",
    "bridge",
    "config",
    "connection",
    "connector",
    "credentials",
    "device",
    "browser",
    "extension",
    "file",
    "git",
    "headless",
    "hooks",
    "http",
    "ide",
    "pairing",
    "script",
    "server",
    "session",
    "sessions",
    "shell",
    "tailscale",
    "task",
    "terminal",
    "token",
    "vornd",
    "widget",
    "workflow",
    "worktree",
    "vorn",
];

/// The notification a window answers a selection request with.
pub const SELECTION_RESULT: &str = "extension:selectionResult";

const FOOTER_ITEMS: &str = "extension:footerItems";
const ACTIVATION: &str = "extension:activation";
const SELECTION_REQUEST: &str = "extension:selectionRequest";

/// Long enough for a window busy painting, short enough not to hold a band's turn.
const SELECTION_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the installed packs are looked at for a change.
const PACK_CHECK: Duration = Duration::from_secs(2);
/// How long a pane's program has to start.
const TERMINAL_START: Duration = Duration::from_secs(30);

/// A running terminal session, as the extensions see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    pub id: String,
    pub agent: String,
    pub project_path: String,
    pub worktree_path: Option<String>,
    pub agent_session_id: Option<String>,
    pub renamed_by_person: bool,
}

impl Live {
    fn worktree(&self) -> &str {
        self.worktree_path.as_deref().unwrap_or(&self.project_path)
    }

    fn asked(&self) -> Asked<'_> {
        Asked {
            session_id: &self.id,
            worktree_path: self.worktree(),
            agent: &self.agent,
        }
    }

    /// What decides what shows on it; a change to anything else changes nothing shown.
    fn shape(&self) -> (&str, &str, &str) {
        (&self.project_path, self.worktree(), &self.agent)
    }
}

/// What the extensions need from the rest of vornd, behind one seam so the
/// rules here are tested without a server, a holder or a socket.
pub trait Around: Send + Sync + 'static {
    /// The running terminals; `None` while vornd holds no copy of the records.
    fn live(&self) -> Option<Vec<Live>>;
    /// Told whenever the records change.
    fn notes(&self) -> Option<broadcast::Receiver<Value>>;
    /// Names a card as an extension names it: a person may rename it after.
    fn rename(&self, id: &str, name: &str) -> Result<(), String>;
    /// Tells the clients drawing session `scope`.
    fn broadcast(&self, method: &str, params: Value, scope: &str);
    /// How many clients are connected.
    fn clients(&self) -> u64;
    /// The environment a program is started with.
    fn base_env(&self) -> Vec<(String, String)>;
    /// Starts a terminal named `id`; answered with its pid once it runs.
    fn start_terminal(
        &self,
        id: String,
        spec: SpawnSpec,
    ) -> Result<oneshot::Receiver<Result<u32, String>>, String>;
    /// Stops terminal `id`.
    fn kill_terminal(&self, id: &str);
    /// The last `lines` of terminal `id`.
    fn read_output(
        &self,
        id: &str,
        lines: usize,
    ) -> BoxFuture<'static, Result<Vec<String>, String>>;
    /// Types `text` into terminal `id`.
    fn write(&self, id: &str, text: &str) -> BoxFuture<'static, Result<(), String>>;
    /// The git to read a worktree with.
    fn git(&self) -> Git;
    /// A package was installed or linked: the connector it brings comes with it.
    fn pack_changed(&self, _id: &str) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
    /// A package was removed: the connections made with it go with it.
    fn pack_removed(&self, _id: &str) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[derive(Default)]
struct Bands {
    footers: Footers,
    pollers: HashMap<Key, JoinHandle<()>>,
}

/// The extension host: its processes, panes, bands and pending selections.
pub struct Extensions {
    hosts: Arc<Hosts>,
    router: Arc<Router>,
    installs: Arc<Installs>,
    around: Arc<dyn Around>,
    /// Where an extension's process and programs reach the bridge.
    bridge_origin: String,
    /// The origin pane pages are served from, once its listener is up.
    page_origin: OnceLock<String>,
    /// The origins that may frame a pane.
    ancestors: Vec<String>,
    home: PathBuf,
    grants: Mutex<Grants>,
    bands: Mutex<Bands>,
    selections: Mutex<HashMap<u64, oneshot::Sender<String>>>,
    /// Each open pane keeps its extension's host running: a pane's program holds that host's token.
    pane_hosts: Mutex<HashMap<String, Lease<Host>>>,
    next_selection: AtomicU64,
    /// The newest sync asked for each session; reading git can finish them out of order.
    syncs: Mutex<HashMap<String, u64>>,
    next_sync: AtomicU64,
    me: Weak<Extensions>,
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extensions")
            .field("bridge_origin", &self.bridge_origin)
            .field("page_origin", &self.page_origin.get())
            .finish_non_exhaustive()
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Runs `work` off the runtime: git and builds block.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|e| Err(format!("the extension work failed in vornd: {e}")))
}

fn text<'a>(params: &'a Value, key: &str) -> &'a str {
    params.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl Extensions {
    pub fn new(
        hosts: Arc<Hosts>,
        router: Arc<Router>,
        installs: Arc<Installs>,
        around: Arc<dyn Around>,
        bridge_origin: String,
        ancestors: Vec<String>,
        home: PathBuf,
    ) -> Arc<Extensions> {
        Arc::new_cyclic(|me| Extensions {
            hosts,
            router,
            installs,
            around,
            bridge_origin,
            page_origin: OnceLock::new(),
            ancestors,
            home,
            grants: Mutex::default(),
            bands: Mutex::default(),
            selections: Mutex::default(),
            pane_hosts: Mutex::default(),
            next_selection: AtomicU64::new(0),
            syncs: Mutex::default(),
            next_sync: AtomicU64::new(0),
            me: me.clone(),
        })
    }

    /// The page listener is up at `origin`. Only the first one given is kept.
    pub fn set_page_origin(&self, origin: String) {
        let _ = self.page_origin.set(origin);
    }

    /// The extension processes running.
    pub fn hosts(&self) -> usize {
        self.hosts.running().len()
    }

    /// The router programs are called and woken through.
    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// The panes open.
    pub fn panes(&self) -> usize {
        guard(&self.grants).len()
    }

    /// Keeps every session's bands and processes in step with the records
    /// and the installed packs, until vornd stops.
    pub fn start(self: &Arc<Self>) {
        self.router.reindex();
        let notes = self.around.notes();
        tokio::spawn(watch(Arc::downgrade(self), notes));
    }

    /// Stops every process and terminal the extensions started.
    pub async fn stop(&self) {
        let closed = guard(&self.grants).close_where(|_| true);
        self.kill_terminals(&closed);
        self.hosts.stop_all().await;
        self.router.lifecycle().stop_all().await;
    }

    /// Answers one of [`METHODS`].
    pub async fn answer(&self, method: &str, params: &Value) -> Answer {
        let session = || self.live(text(params, "sessionId"));
        let answered = match method {
            "extension:list" => {
                Ok(serde_json::to_value(self.hosts.store().extensions()).unwrap_or_default())
            }
            "extension:installed" => Ok(self.installed()),
            "extension:inspect" => {
                let (url, branch) = (text(params, "url").to_owned(), params.get("branch"));
                let branch = branch.and_then(Value::as_str).map(str::to_owned);
                let installs = Arc::clone(&self.installs);
                blocking(move || installs.inspect_git(&url, branch.as_deref()))
                    .await
                    .map(|sheet| json!(sheet))
            }
            "extension:install" => {
                let (token, typed) = (
                    text(params, "token").to_owned(),
                    text(params, "confirm").to_owned(),
                );
                let installs = Arc::clone(&self.installs);
                match blocking(move || installs.confirm(&token, &typed)).await {
                    Ok(pack) => {
                        self.package_changed(&pack.id).await;
                        Ok(json!(pack))
                    }
                    Err(err) => Err(err),
                }
            }
            "extension:link" => {
                let path = PathBuf::from(text(params, "path"));
                let installs = Arc::clone(&self.installs);
                match blocking(move || installs.link(&path)).await {
                    Ok(pack) => {
                        self.package_changed(&pack.id).await;
                        Ok(json!(pack))
                    }
                    Err(err) => Err(err),
                }
            }
            "extension:remove" => self.remove(text(params, "id")).await,
            "extension:call" => {
                let ctx = CallCtx {
                    caller: "client".to_owned(),
                    project: params
                        .get("project")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                let args = params.get("args").cloned().unwrap_or(Value::Null);
                self.router
                    .call(text(params, "id"), text(params, "action"), args, ctx)
                    .await
                    .map_err(|e| e.message)
            }
            "extension:rows" => Ok(Value::Array(
                self.router
                    .rows(text(params, "id"), text(params, "collection")),
            )),
            "extension:running" => Ok(json!(self
                .router
                .lifecycle()
                .running()
                .into_iter()
                .map(|k| json!({ "extensionId": k.extension, "scope": k.scope }))
                .collect::<Vec<_>>())),
            "extension:activation" => match session() {
                Ok(s) => Ok(Value::Array(self.states(&s).await)),
                Err(err) => Err(err),
            },
            "extension:footerItems" => {
                let readings = guard(&self.bands)
                    .footers
                    .readings_of(text(params, "sessionId"));
                Ok(serde_json::to_value(readings).unwrap_or_default())
            }
            "extension:openPane" => match session() {
                Ok(s) => self
                    .open_pane(text(params, "extensionId"), text(params, "paneId"), &s)
                    .await
                    .map(|grant| serde_json::to_value(grant).unwrap_or_default()),
                Err(err) => Err(err),
            },
            "extension:closePane" => {
                Ok(json!({ "closed": self.close_pane(text(params, "nonce")) }))
            }
            "extension:runHandler" => match session() {
                Ok(s) => {
                    let (ext, handler) = (text(params, "extensionId"), text(params, "handlerId"));
                    self.run_handler(ext, handler, &s, text(params, "url"))
                        .await
                }
                Err(err) => Err(err),
            },
            "extension:matchLinks" => match session() {
                Ok(s) => Ok(self.match_links(&s, text(params, "text")).await),
                Err(err) => Err(err),
            },
            _ => return Answer::Unanswered,
        };
        answered.map_or_else(Answer::Error, Answer::Result)
    }

    /// Every installed package, each saying whether a program of it runs.
    fn installed(&self) -> Value {
        let running = self.router.lifecycle().running();
        let packs: Vec<Value> = self
            .hosts
            .store()
            .list()
            .into_iter()
            .map(|pack| {
                let mut value = json!(pack);
                value["running"] = json!(running.iter().any(|k| k.extension == pack.id));
                value
            })
            .collect();
        Value::Array(packs)
    }

    /// Stops every process of `id`, drops what it published, removes its
    /// files and the connection it came with: nothing of it is left.
    async fn remove(&self, id: &str) -> Result<Value, String> {
        if self.hosts.store().inspect(id).is_err()
            && !self.hosts.store().ids().iter().any(|i| i == id)
        {
            return Err(format!("No extension \"{id}\" is installed"));
        }
        let closed = guard(&self.grants).close_where(|g| g.extension_id == id);
        self.kill_terminals(&closed);
        self.hosts.stop_extension(id).await;
        self.router.forget(id).await;
        let (installs, owned) = (Arc::clone(&self.installs), id.to_owned());
        blocking(move || installs.remove(&owned)).await?;
        self.around.pack_removed(id).await;
        Ok(json!({ "removed": id }))
    }

    /// After an install or a link, nothing runs its old files and its connector, if any, appears.
    async fn package_changed(&self, id: &str) {
        self.pack_changed(id).await;
        self.router.lifecycle().stop_extension(id).await;
        self.around.pack_changed(id).await;
    }

    /// Whether `method` is `<id>:<action>` for an installed program extension.
    pub fn owns(&self, method: &str) -> bool {
        let Some((id, action)) = method.split_once(':') else {
            return false;
        };
        !RESERVED.contains(&id) && !action.is_empty() && self.router.program(id).is_some()
    }

    /// A client's call to `<id>:<action>`, routed to the extension.
    pub async fn call_action(&self, method: &str, params: Value) -> Answer {
        let Some((id, action)) = method.split_once(':') else {
            return Answer::Unanswered;
        };
        let ctx = CallCtx {
            caller: "client".to_owned(),
            project: params
                .get("project")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        match self.router.call(id, action, params, ctx).await {
            Ok(value) => Answer::Result(value),
            Err(ExtError { message, .. }) => Answer::Error(message),
        }
    }

    /// The running session `id`, worded as the server words a missing one.
    fn live(&self, id: &str) -> Result<Live, String> {
        self.session(id)
            .ok_or_else(|| format!("Session not found: {id}"))
    }

    fn session(&self, id: &str) -> Option<Live> {
        self.around.live()?.into_iter().find(|s| s.id == id)
    }

    /// The remote host a session's repository names, read only when a pack looks at it.
    async fn remote_host(&self, session: &Live, packs: &[InstalledPack]) -> Option<String> {
        if !activation::names_remote_host(packs) {
            return None;
        }
        let git = self.around.git();
        let worktree = PathBuf::from(session.worktree());
        tokio::task::spawn_blocking(move || git.origin_url(&worktree))
            .await
            .ok()
            .flatten()
            .and_then(|url| activation::remote_host_of(&url))
    }

    /// What every installed extension shows on `session`.
    async fn states(&self, session: &Live) -> Vec<Value> {
        let packs = self.hosts.store().extensions();
        let remote = self.remote_host(session, &packs).await;
        let subject = subject_of(session, remote.as_deref());
        packs.iter().map(|p| state(p, &subject)).collect()
    }

    /// Starts the bands `session` shows and stops the rest, then tells its windows what shows.
    async fn sync(&self, session: Live) {
        let turn = self.next_sync.fetch_add(1, Ordering::Relaxed) + 1;
        guard(&self.syncs).insert(session.id.clone(), turn);
        let packs = self.hosts.store().extensions();
        let remote = self.remote_host(&session, &packs).await;
        if guard(&self.syncs).get(&session.id) != Some(&turn) {
            return;
        }
        let subject = subject_of(&session, remote.as_deref());
        let mut wanted = Vec::new();
        let mut states = Vec::with_capacity(packs.len());
        for pack in &packs {
            let shown = activation::shown_on(pack, &subject);
            states.push(state(pack, &subject));
            let footers = pack
                .contributions()
                .map(|c| c.footers())
                .unwrap_or_default();
            for footer in footers
                .iter()
                .filter(|f| shown.footers.contains(&f.base.id))
            {
                let key = Key {
                    session: session.id.clone(),
                    extension: pack.id.clone(),
                    footer: footer.base.id.clone(),
                };
                let declared = Declared::new(
                    footer.every,
                    footer.base.title.clone(),
                    pack.version.clone(),
                );
                wanted.push((key, declared));
            }
        }
        self.plan_bands(&session.id, wanted);
        self.around.broadcast(
            ACTIVATION,
            json!({ "sessionId": session.id, "states": states }),
            &session.id,
        );
    }

    fn plan_bands(&self, session: &str, wanted: Vec<(Key, Declared)>) {
        let mut bands = guard(&self.bands);
        let plan = bands.footers.sync(session, wanted);
        for key in plan.stop {
            if let Some(poller) = bands.pollers.remove(&key) {
                poller.abort();
            }
        }
        for (key, every_ms) in plan.start {
            let poller = tokio::spawn(poll(self.me.clone(), key.clone(), every_ms));
            if let Some(old) = bands.pollers.insert(key, poller) {
                old.abort();
            }
        }
    }

    /// Reads band `key` once: `None` once it is no longer wanted, else a new interval if it changed.
    async fn read_band(&self, key: &Key) -> Option<Option<u64>> {
        if !guard(&self.bands).footers.has(key) {
            return None;
        }
        let (Some(session), Some(pack)) = (
            self.session(&key.session),
            self.hosts.store().describe(&key.extension),
        ) else {
            return Some(None);
        };
        let host_key = HostKey {
            extension_id: key.extension.clone(),
            project_path: session.project_path.clone(),
        };
        let answer = match self.hosts.get_or_start(&host_key).await {
            Ok(host) => host.footer(&key.footer, session.asked()).await,
            Err(err) => Err(err),
        };
        if let Err(err) = &answer {
            let (ext, footer) = (&key.extension, &key.footer);
            warn!("[extensions] {ext} {footer} failed: {err}");
        }
        let outcome = guard(&self.bands)
            .footers
            .record(key, pack.name(), answer, iso_now());
        if let Some(readings) = outcome.publish {
            self.around.broadcast(
                FOOTER_ITEMS,
                json!({ "sessionId": key.session, "readings": readings }),
                &key.session,
            );
        }
        Some(outcome.retime)
    }

    /// Everything held for a session that ended, and its project's processes once none is left.
    async fn release(&self, session: &Live) {
        guard(&self.syncs).remove(&session.id);
        {
            let mut bands = guard(&self.bands);
            for key in bands.footers.stop_session(&session.id) {
                if let Some(poller) = bands.pollers.remove(&key) {
                    poller.abort();
                }
            }
        }
        let closed = guard(&self.grants).close_where(|g| g.session_id == session.id);
        self.kill_terminals(&closed);
        // A pane's own terminal that ended takes only its authority with it.
        let ended = guard(&self.grants)
            .close_where(|g| g.terminal_id.as_deref() == Some(session.id.as_str()));
        self.let_go(&ended);
        let still_open = self.around.live().is_some_and(|live| {
            live.iter()
                .any(|s| s.id != session.id && s.project_path == session.project_path)
        });
        if !still_open {
            self.hosts.stop_project(&session.project_path).await;
        }
    }

    /// After a pack changed, no process keeps running its old files.
    async fn pack_changed(&self, extension_id: &str) {
        self.router.reindex();
        let closed = guard(&self.grants).close_where(|g| g.extension_id == extension_id);
        self.kill_terminals(&closed);
        self.hosts.stop_extension(extension_id).await;
    }

    fn kill_terminals(&self, closed: &[Grant]) {
        for terminal in closed.iter().filter_map(|g| g.terminal_id.as_deref()) {
            self.around.kill_terminal(terminal);
        }
        self.let_go(closed);
    }

    /// Closed panes no longer keep their hosts from idling.
    fn let_go(&self, closed: &[Grant]) {
        let mut held = guard(&self.pane_hosts);
        for grant in closed {
            held.remove(&grant.nonce);
        }
    }

    async fn open_pane(&self, ext: &str, pane_id: &str, session: &Live) -> Result<Grant, String> {
        let pack = self
            .hosts
            .store()
            .describe(ext)
            .filter(InstalledPack::is_extension);
        let pane = pack
            .as_ref()
            .and_then(|p| p.contributions())
            .and_then(|c| c.panes().iter().find(|p| p.base.id == pane_id))
            .ok_or_else(|| format!("The extension \"{ext}\" contributes no pane \"{pane_id}\""))?;
        let key = HostKey {
            extension_id: ext.to_owned(),
            project_path: session.project_path.clone(),
        };
        // Started first, so a page's first read or a program's first line has a bridge to talk to.
        let host = self.hosts.get_or_start(&key).await?;
        let nonce = token::mint().map_err(|e| e.to_string())?;
        let mut grant = Grant {
            nonce,
            extension_id: ext.to_owned(),
            pane_id: pane_id.to_owned(),
            session_id: session.id.clone(),
            project_path: session.project_path.clone(),
            url: None,
            terminal_id: None,
            expires: now_ms() + GRANT_IDLE_MS,
        };
        match &pane.draws {
            PaneDraws::Command(argv) => {
                let terminal = self.start_program(ext, &key, argv, session).await?;
                grant.terminal_id = Some(terminal);
            }
            PaneDraws::Web(_) => {
                let origin = self.page_origin.get().ok_or_else(|| {
                    "the page server is not running, so no pane can be served".to_owned()
                })?;
                grant.url = Some(format!(
                    "{origin}/extensions/{ext}/pane/{pane_id}/{}/",
                    grant.nonce
                ));
            }
        }
        guard(&self.grants).insert(grant.clone());
        guard(&self.pane_hosts).insert(grant.nonce.clone(), host);
        Ok(grant)
    }

    /// A pane's program, in a terminal of its own holding the extension's token.
    async fn start_program(
        &self,
        ext: &str,
        key: &HostKey,
        argv: &[String],
        session: &Live,
    ) -> Result<String, String> {
        use super::sessions::{set, INITIAL_COLS, INITIAL_ROWS, PTY_TERM};
        let id = uuid::Uuid::new_v4().to_string();
        let mut env = self.around.base_env();
        if let Some(token) = self.hosts.token_for(key) {
            set(&mut env, "VORN_EXTENSION_TOKEN", token);
        }
        let host = format!("{}/extensions/{ext}/bridge", self.bridge_origin);
        set(&mut env, "VORN_EXTENSION_HOST", host);
        set(&mut env, "VORN_SESSION_ID", id.clone());
        if !cfg!(windows) {
            set(&mut env, "TERM", PTY_TERM.to_owned());
        }
        let spec = SpawnSpec {
            argv: argv.to_vec(),
            cwd: session.worktree().to_owned(),
            env,
            io: Io::Pty {
                cols: INITIAL_COLS,
                rows: INITIAL_ROWS,
            },
            ring_bytes: None,
        };
        let started = self.around.start_terminal(id.clone(), spec)?;
        match tokio::time::timeout(TERMINAL_START, started).await {
            Ok(Ok(Ok(_pid))) => Ok(id),
            Ok(Ok(Err(err))) => Err(err),
            _ => Err(format!("the pane's program did not start: {}", argv[0])),
        }
    }

    /// Closing takes the authority with it, whichever end asks.
    fn close_pane(&self, nonce: &str) -> bool {
        let Some(grant) = guard(&self.grants).close(nonce) else {
            return false;
        };
        self.kill_terminals(std::slice::from_ref(&grant));
        true
    }

    /// The grant `nonce` names right now, its idle time restarted.
    fn grant(&self, nonce: &str) -> Option<Grant> {
        let found = guard(&self.grants).touch(nonce, now_ms());
        match found {
            Lookup::Live(grant) => Some(grant),
            Lookup::Expired(grant) => {
                self.kill_terminals(std::slice::from_ref(&grant));
                None
            }
            Lookup::Unknown => None,
        }
    }

    async fn run_handler(
        &self,
        ext: &str,
        handler: &str,
        session: &Live,
        url: &str,
    ) -> Result<Value, String> {
        let key = HostKey {
            extension_id: ext.to_owned(),
            project_path: session.project_path.clone(),
        };
        let host = self.hosts.get_or_start(&key).await?;
        let Some(asked) = host.handler(handler, session.asked(), url).await? else {
            return Ok(json!({}));
        };
        info!("[extensions] {ext} {handler} opened {asked}");
        let grant = self.open_pane(ext, &asked, session).await?;
        Ok(json!({ "openedPane": grant }))
    }

    async fn match_links(&self, session: &Live, text: &str) -> Value {
        if text.is_empty() {
            return json!([]);
        }
        let packs = self.hosts.store().extensions();
        let remote = self.remote_host(session, &packs).await;
        let subject = subject_of(session, remote.as_deref());
        serde_json::to_value(links::match_links(&packs, &subject, text)).unwrap_or_default()
    }

    /// What is selected in `session`, which only a window drawing it knows.
    async fn selection(&self, session: &str) -> String {
        if self.around.clients() == 0 {
            return String::new();
        }
        let id = self.next_selection.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        guard(&self.selections).insert(id, tx);
        self.around.broadcast(
            SELECTION_REQUEST,
            json!({ "requestId": id, "sessionId": session }),
            session,
        );
        match tokio::time::timeout(SELECTION_TIMEOUT, rx).await {
            Ok(Ok(text)) => text,
            _ => {
                guard(&self.selections).remove(&id);
                String::new()
            }
        }
    }

    /// A window's answer to a selection request; the first one wins.
    pub fn resolve_selection(&self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(Value::as_u64) else {
            return;
        };
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(waiting) = guard(&self.selections).remove(&id) {
            let _ = waiting.send(text.to_owned());
        }
    }
}

fn subject_of<'a>(session: &'a Live, remote_host: Option<&'a str>) -> Subject<'a> {
    Subject {
        worktree: Path::new(session.worktree()),
        agent: &session.agent,
        platform: activation::platform(),
        remote_host,
    }
}

/// One extension's activation on a card, as clients read it.
fn state(pack: &InstalledPack, subject: &Subject<'_>) -> Value {
    let mut state = Map::new();
    state.insert("extensionId".into(), json!(pack.id));
    state.insert("extensionName".into(), json!(pack.name()));
    if let Ok(Value::Object(shown)) = serde_json::to_value(activation::shown_on(pack, subject)) {
        state.extend(shown);
    }
    Value::Object(state)
}

/// Reads band `key` now and then every interval, until it is no longer wanted.
async fn poll(me: Weak<Extensions>, key: Key, every_ms: u64) {
    let mut every = every_ms;
    loop {
        let Some(extensions) = me.upgrade() else {
            return;
        };
        match extensions.read_band(&key).await {
            None => return,
            Some(Some(retimed)) => every = retimed,
            Some(None) => {}
        }
        drop(extensions);
        tokio::time::sleep(Duration::from_millis(every)).await;
    }
}

type Fingerprint = HashMap<String, (Option<SystemTime>, u64)>;

/// Each installed pack's current version, by when it was last written.
fn fingerprint(root: &Path) -> Fingerprint {
    let Ok(entries) = std::fs::read_dir(root) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let meta = std::fs::metadata(entry.path().join("current.json")).ok()?;
            let id = entry.file_name().into_string().ok()?;
            Some((id, (meta.modified().ok(), meta.len())))
        })
        .collect()
}

/// The packs whose current version differs between `before` and `after`.
fn changed(before: &Fingerprint, after: &Fingerprint) -> Vec<String> {
    let mut ids: Vec<String> = before
        .keys()
        .chain(after.keys())
        .filter(|id| before.get(*id) != after.get(*id))
        .cloned()
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

async fn next_note(notes: &mut Option<broadcast::Receiver<Value>>) {
    match notes {
        Some(rx) => {
            if let Err(broadcast::error::RecvError::Closed) = rx.recv().await {
                *notes = None;
            }
        }
        None => std::future::pending().await,
    }
}

/// Settles each session that appeared or moved and releases each that ended,
/// and settles every session again after a pack changed.
async fn watch(me: Weak<Extensions>, mut notes: Option<broadcast::Receiver<Value>>) {
    let mut known: HashMap<String, Live> = HashMap::new();
    let Some(mut packs) = me.upgrade().map(|e| fingerprint(e.hosts.store().root())) else {
        return;
    };
    let mut tick = tokio::time::interval(PACK_CHECK);
    loop {
        tokio::select! {
            () = next_note(&mut notes) => {}
            _ = tick.tick() => {}
        }
        let Some(extensions) = me.upgrade() else {
            return;
        };
        let now = fingerprint(extensions.hosts.store().root());
        let moved = changed(&packs, &now);
        packs = now;
        for id in &moved {
            debug!("[extensions] pack {id} changed");
            extensions.pack_changed(id).await;
        }
        reconcile(&extensions, &mut known, !moved.is_empty());
    }
}

/// Syncs the sessions that are new or moved (every one when `all`), and releases the ones gone.
fn reconcile(extensions: &Arc<Extensions>, known: &mut HashMap<String, Live>, all: bool) {
    let Some(live) = extensions.around.live() else {
        return;
    };
    let now: HashMap<String, Live> = live.into_iter().map(|s| (s.id.clone(), s)).collect();
    for (id, session) in &now {
        if all || known.get(id).map(Live::shape) != Some(session.shape()) {
            let extensions = Arc::clone(extensions);
            let session = session.clone();
            tokio::spawn(async move { extensions.sync(session).await });
        }
    }
    for (id, session) in known.drain() {
        if !now.contains_key(&id) {
            let extensions = Arc::clone(extensions);
            tokio::spawn(async move { extensions.release(&session).await });
        }
    }
    *known = now;
}

/// [`Around`] in vornd: the registry, the session host, its own endpoint and the server's clients.
pub struct Wired {
    pub native: Weak<super::Native>,
    pub loopback: Arc<crate::mcp::Loopback>,
    pub clients: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl Around for Wired {
    fn live(&self) -> Option<Vec<Live>> {
        let native = self.native.upgrade()?;
        native.registry.get()?.read(|r| {
            r.live_terminals()
                .map(|t| Live {
                    id: t.id.clone(),
                    agent: t.agent_type.clone(),
                    project_path: t.project_path.clone(),
                    worktree_path: t.worktree_path.clone(),
                    agent_session_id: t.agent_session_id.clone(),
                    renamed_by_person: t.renamed_by_person.unwrap_or(false),
                })
                .collect()
        })
    }

    fn notes(&self) -> Option<broadcast::Receiver<Value>> {
        Some(self.native.upgrade()?.registry.get()?.subscribe())
    }

    fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        let registry = native
            .registry
            .get()
            .ok_or("vornd holds no session records")?;
        let mut fields = Map::new();
        fields.insert("displayName".into(), json!(name));
        fields.insert("renamedByPerson".into(), json!(false));
        registry
            .change(|r| match r.set_fields(id, fields) {
                Ok(note) => (Ok(()), note.into_iter().collect()),
                Err(err) => (Err(err.to_string()), Vec::new()),
            })
            .unwrap_or_else(|| Err("vornd does not hold the session records yet".to_owned()))
    }

    fn broadcast(&self, method: &str, params: Value, scope: &str) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast_to(method, params, Some(scope));
        }
    }

    fn clients(&self) -> u64 {
        (self.clients)()
    }

    fn base_env(&self) -> Vec<(String, String)> {
        self.native
            .upgrade()
            .map(|n| n.env.get())
            .unwrap_or_default()
    }

    fn start_terminal(
        &self,
        id: String,
        spec: SpawnSpec,
    ) -> Result<oneshot::Receiver<Result<u32, String>>, String> {
        let native = self.native.upgrade().ok_or("vornd is stopping")?;
        let host = native
            .host
            .get()
            .filter(|h| h.ready())
            .ok_or("vornd is not holding sessions, so no program pane can start")?;
        let (tx, rx) = oneshot::channel();
        let then: super::sessions::Then = Box::new(move |started| {
            let _ = tx.send(started.map(|s| s.pid));
        });
        host.start(spec, id, super::sessions::Input::None, then);
        Ok(rx)
    }

    fn kill_terminal(&self, id: &str) {
        use vorn_sessiond_wire::Sig;
        if let Some(host) = self.native.upgrade().and_then(|n| n.host.get().cloned()) {
            host.signal(id, Sig::Term);
            host.signal_after(id, Sig::Kill, super::headless::FORCE_KILL_DELAY);
        }
    }

    fn read_output(
        &self,
        id: &str,
        lines: usize,
    ) -> BoxFuture<'static, Result<Vec<String>, String>> {
        let loopback = Arc::clone(&self.loopback);
        let params = json!({ "id": id, "lines": lines });
        Box::pin(async move {
            use vorn_mcp::Rpc;
            let answer = loopback
                .call(
                    "terminal:readOutput",
                    Some(params),
                    vorn_mcp::rpc::DEFAULT_TIMEOUT,
                )
                .await
                .map_err(String::from)?;
            let lines = answer.as_array().map(Vec::as_slice).unwrap_or_default();
            Ok(lines
                .iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect())
        })
    }

    fn write(&self, id: &str, text: &str) -> BoxFuture<'static, Result<(), String>> {
        let loopback = Arc::clone(&self.loopback);
        let params = json!({ "id": id, "data": text });
        Box::pin(async move {
            use vorn_mcp::Rpc;
            loopback
                .call(
                    "terminal:write",
                    Some(params),
                    vorn_mcp::rpc::DEFAULT_TIMEOUT,
                )
                .await
                .map(|_| ())
                .map_err(String::from)
        })
    }

    fn pack_changed(&self, id: &str) -> BoxFuture<'static, ()> {
        let connectors = self.native.upgrade().and_then(|n| n.connectors().cloned());
        let id = id.to_owned();
        Box::pin(async move {
            if let Some(connectors) = connectors {
                connectors.package_changed(&id).await;
            }
        })
    }

    fn pack_removed(&self, id: &str) -> BoxFuture<'static, ()> {
        let connectors = self.native.upgrade().and_then(|n| n.connectors().cloned());
        let id = id.to_owned();
        Box::pin(async move {
            if let Some(connectors) = connectors {
                connectors.package_removed(&id).await;
            }
        })
    }

    fn git(&self) -> Git {
        match self.native.upgrade() {
            Some(native) => Git {
                bin: native.env.git_bin(),
                env: native.env.get(),
                ssh: None,
            },
            None => Git {
                bin: "git".to_owned(),
                env: Vec::new(),
                ssh: None,
            },
        }
    }
}

#[cfg(test)]
mod tests;
