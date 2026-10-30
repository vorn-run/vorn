//! Extensions, hosted here: the programs that speak the extension API, which
//! the bridge's [`Router`] calls, and the extensions that add to a
//! card, one process per extension and project; installing, linking and
//! removing them; the `extension:*` calls clients make about them, the bands
//! and panes they show, and the bridge a card extension speaks back on
//! ([`routes`]). Every process runs under the one lifecycle vornd holds.
//!
//! The rules are [`vorn_extensions`]'s. This module runs them against what
//! vornd has around it ([`Around`]): the session records, the session host,
//! its own endpoint and the clients it serves.

pub mod builtin;
pub mod hosted;
pub mod moved;
pub mod routes;
pub mod world;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, oneshot, Notify};
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
use vorn_extensions::reach::Given;
use vorn_extensions::router::{Hook, Router};
use vorn_extensions::{points, token};
use vorn_git::repo::Git;
use vorn_sessiond_wire::{Io, SpawnSpec};

use super::Answer;
use builtin::Builtins;

/// Every call clients make that is answered here.
pub const METHODS: &[&str] = &[
    "extension:list",
    "extension:activation",
    "extension:openPane",
    "extension:closePane",
    "extension:installed",
    "extension:inspect",
    "extension:install",
    "extension:link",
    "extension:remove",
    "extension:call",
    "extension:rows",
    "extension:mcpTools",
    "extension:mcpCall",
    "extension:setSandbox",
    "extension:contributions",
    "extension:builtins",
    "extension:setBuiltin",
    "extension:preferences",
    "extension:setPreference",
];

/// Told to every client when what extensions contribute changed: a package
/// came or went, a built-in was switched, or a linked package's manifest was saved.
pub const CONTRIBUTIONS_CHANGED: &str = "extension:contributionsChanged";

/// The longest tool name MCP clients take.
const MCP_NAME_MAX: usize = 64;

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

const FOOTER_ITEMS: &str = "extension:footerItems";
const ACTIVATION: &str = "extension:activation";

/// How often the installed packs are looked at for a change.
const PACK_CHECK: Duration = Duration::from_secs(2);
/// How often a linked package's manifest is looked at: a saved one redraws within 100 ms.
const LINKED_CHECK: Duration = Duration::from_millis(40);
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
    /// The running terminals; `None` while the records are not yet read.
    fn live(&self) -> Option<Vec<Live>>;
    /// Told whenever the records change.
    fn notes(&self) -> Option<broadcast::Receiver<Value>>;
    /// Names a card as an extension names it: a person may rename it after.
    fn rename(&self, id: &str, name: &str) -> Result<(), String>;
    /// Tells the clients drawing session `scope`.
    fn broadcast(&self, method: &str, params: Value, scope: &str);
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
    /// The built-ins vornd started before its extension host, to join the registry.
    fn started_builtins(&self) -> Vec<Arc<dyn builtin::Builtin>> {
        Vec::new()
    }
    /// The agents vornd runs, which installed adapters join.
    fn adapters(&self) -> Option<Arc<crate::adapters::Adapters>> {
        None
    }

    /// What extension `id` contributes changed (installed, removed or switched),
    /// or, with no id, every extension was read at start.
    fn contributions_changed(&self, _id: &str) {}

    fn pack_changed(&self, _id: &str) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
    /// A package is about to be removed: its connections' programs stop,
    /// and nothing else changes yet.
    fn pack_stopping(&self, _id: &str) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
    /// A package was removed: the connections made with it, their secrets
    /// and its tables go with it. A failure fails the removal, which another
    /// removal finishes.
    fn pack_removed(&self, _id: &str) -> BoxFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// One extension's MCP tool, as listed and as routed.
struct McpListed {
    name: String,
    extension: String,
    tool: String,
    listed: Value,
}

#[derive(Default)]
struct Bands {
    footers: Footers,
    pollers: HashMap<Key, JoinHandle<()>>,
}

/// An extension answering a person's click, counted until it drops.
struct Acting<'a> {
    given: &'a Mutex<Given>,
    id: String,
}

impl Drop for Acting<'_> {
    fn drop(&mut self) {
        guard(self.given).end(&self.id);
    }
}

/// The extension host: its processes, panes and bands.
pub struct Extensions {
    hosts: Arc<Hosts>,
    router: Arc<Router>,
    builtins: Builtins,
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
    /// What a person handed each extension, the sessions each started, and who answers a click now.
    given: Mutex<Given>,
    bands: Mutex<Bands>,
    /// Each open pane keeps its extension's host running: a pane's program holds that host's token.
    pane_hosts: Mutex<HashMap<String, Lease<Host>>>,
    /// The newest sync asked for each session; reading git can finish them out of order.
    syncs: Mutex<HashMap<String, u64>>,
    next_sync: AtomicU64,
    /// A package was linked or installed: the manifest watcher lists the linked ones again.
    relist: AtomicBool,
    /// A link, install or removal changed a pack: the pack watcher settles every session again.
    resettle: AtomicBool,
    /// A package was linked, installed or removed: its sessions are settled now, not on the next pack check.
    settle_now: Arc<Notify>,
    /// Each pack's version as last handled; a link, install or removal holds it, so the pack watcher acts only on changes made elsewhere.
    packs: tokio::sync::Mutex<Fingerprint>,
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

/// A path typed as `~/…` on this machine's home; anything else as it is.
fn home_path(typed: &str) -> String {
    let typed = typed.trim();
    match typed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", super::shell::home_dir())
        }
        _ => typed.to_owned(),
    }
}

/// A `ui.render` answer the client can draw, or every reason it cannot.
fn tree_checked(id: &str, answer: Value) -> Result<Value, String> {
    match vorn_tree::read(&answer) {
        Ok(_) => Ok(answer),
        Err(errors) => Err(format!(
            "{id} answered ui.render with a tree that cannot be drawn: {}",
            errors.join("; ")
        )),
    }
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
        let kept = router.store().root().parent().map_or_else(
            || PathBuf::from("builtins.json"),
            |d| d.join("builtins.json"),
        );
        let builtins = Builtins::new(Arc::clone(&router), kept, around.started_builtins());
        Arc::new_cyclic(|me| Extensions {
            hosts,
            builtins,
            router,
            installs,
            around,
            bridge_origin,
            page_origin: OnceLock::new(),
            ancestors,
            home,
            grants: Mutex::default(),
            given: Mutex::default(),
            bands: Mutex::default(),
            pane_hosts: Mutex::default(),
            syncs: Mutex::default(),
            next_sync: AtomicU64::new(0),
            relist: AtomicBool::new(false),
            resettle: AtomicBool::new(false),
            settle_now: Arc::default(),
            packs: tokio::sync::Mutex::default(),
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

    /// The extensions whose service runs now, as programs or components.
    pub fn services(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .router
            .lifecycle()
            .running()
            .into_iter()
            .map(|k| k.extension)
            .collect();
        ids.sort();
        ids.dedup();
        ids
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
        self.sync_adapters();
        self.builtins.start();
        let (router, installs) = (Arc::clone(&self.router), Arc::clone(&self.installs));
        tokio::spawn(async move {
            // A linked folder that gained a link while vornd was stopped does not start.
            let _ = tokio::task::spawn_blocking(move || installs.recheck_linked()).await;
            router.start_installed().await;
        });
        self.around.contributions_changed("");
        let notes = self.around.notes();
        tokio::spawn(watch(Arc::downgrade(self), notes));
        tokio::spawn(watch_linked(Arc::downgrade(self)));
    }

    /// The extensions Vorn ships with.
    pub fn builtins(&self) -> &Builtins {
        &self.builtins
    }

    /// Tells the agents' registry which installed packages adapt an agent.
    fn sync_adapters(&self) {
        if let Some(adapters) = self.around.adapters() {
            adapters.set_installed(crate::adapters::read_installed(self.router.store()));
        }
    }

    /// Every contribution a client draws or lists, of the installed packages
    /// and the built-ins that are on.
    fn contributions(&self) -> Vec<Value> {
        let store = self.router.store();
        let mut packs: Vec<_> = (store.ids().iter())
            .filter_map(|id| store.inspect_unmeasured(id).ok())
            .collect();
        // In the order they were installed: a top-bar button keeps its place.
        packs.sort_by_key(|p| p.installed_at.as_u64().unwrap_or(u64::MAX));
        let mut out = Vec::new();
        for pack in packs {
            let settings = self.router.preferences(&pack.manifest);
            for mut c in points::listed(&pack.manifest, &settings) {
                c["extensionName"] = json!(pack.manifest.name);
                c["builtin"] = json!(false);
                out.push(c);
            }
        }
        out.extend(self.builtins.contributions());
        out
    }

    /// The installed extensions that declare workflow node types, trigger
    /// types or workflows: each one's manifest and where its package is.
    pub fn workflow_providers(&self) -> Vec<(vorn_extensions::manifest::Manifest, PathBuf)> {
        let store = self.router.store();
        let mut out = Vec::new();
        for id in store.ids() {
            let Ok(pack) = store.inspect_unmeasured(&id) else {
                continue;
            };
            let declares = pack.manifest.contributes.as_ref().is_some_and(|c| {
                !c.nodes().is_empty() || !c.triggers().is_empty() || !c.workflows().is_empty()
            });
            if declares {
                out.push((pack.manifest.clone(), pack.path.clone()));
            }
        }
        out
    }

    /// The layouts installed extensions offer: `(extension id, layout id,
    /// title, the extension's folder, the file in it)`.
    pub fn layout_files(&self) -> Vec<(String, String, String, PathBuf, String)> {
        let store = self.router.store();
        let mut out = Vec::new();
        for id in store.ids() {
            let Ok(pack) = store.inspect_unmeasured(&id) else {
                continue;
            };
            let Some(contributes) = pack.manifest.contributes.as_ref() else {
                continue;
            };
            for l in contributes.layouts() {
                out.push((
                    id.clone(),
                    l.base.id.clone(),
                    l.base.title.clone(),
                    pack.path.clone(),
                    l.file.clone(),
                ));
            }
        }
        out
    }

    fn contributions_changed(&self, extension: &str) {
        self.around.contributions_changed(extension);
        self.around.broadcast(
            CONTRIBUTIONS_CHANGED,
            json!({ "extensionId": extension }),
            "",
        );
    }

    /// Stops every process and terminal the extensions started.
    pub async fn stop(&self) {
        let closed = guard(&self.grants).close_where(|_| true);
        self.kill_terminals(&closed);
        self.hosts.stop_all().await;
        self.builtins.stop_all();
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
                let (url, branch) = (home_path(text(params, "url")), params.get("branch"));
                let branch = branch.and_then(Value::as_str).map(str::to_owned);
                let link = params.get("link") == Some(&Value::Bool(true));
                let installs = Arc::clone(&self.installs);
                blocking(move || {
                    if link {
                        installs.inspect_link(Path::new(&url))
                    } else {
                        installs.inspect_git(&url, branch.as_deref())
                    }
                })
                .await
                .map(|sheet| {
                    let closed = self.router.confiner().closed_projects();
                    with_sandbox(json!(sheet), self.router.sandbox_kind(), &closed)
                })
            }
            "extension:install" => {
                let (token, typed) = (
                    text(params, "token").to_owned(),
                    text(params, "confirm").to_owned(),
                );
                let id = self.installs.staged_manifest(&token).map(|m| m.id);
                let packs = self.packs.lock().await;
                let finished = self.finish_removal(id.clone()).await;
                let installed = match finished {
                    Ok(()) => {
                        let installs = Arc::clone(&self.installs);
                        blocking(move || installs.confirm(&token, &typed)).await
                    }
                    Err(err) => Err(err),
                };
                if let Ok(pack) = &installed {
                    self.package_changed(&pack.id).await;
                }
                self.handled(
                    packs,
                    id.as_deref()
                        .into_iter()
                        .chain(installed.iter().map(|p| p.id.as_str())),
                );
                installed.map(|pack| json!(pack))
            }
            "extension:link" => {
                let path = PathBuf::from(home_path(text(params, "path")));
                // The manifest the sheet read, which is the one the person confirmed.
                let read = params
                    .get("manifest")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned);
                let (installs, folder) = (Arc::clone(&self.installs), path.clone());
                let id = blocking(move || Ok::<_, String>(installs.link_id(&folder)))
                    .await
                    .ok()
                    .flatten();
                let packs = self.packs.lock().await;
                let linked = match self.finish_removal(id.clone()).await {
                    Ok(()) => {
                        let installs = Arc::clone(&self.installs);
                        blocking(move || installs.link_as_read(&path, read.as_deref())).await
                    }
                    Err(err) => Err(err),
                };
                if let Ok(pack) = &linked {
                    self.package_changed(&pack.id).await;
                }
                self.handled(
                    packs,
                    id.as_deref()
                        .into_iter()
                        .chain(linked.iter().map(|p| p.id.as_str())),
                );
                linked.map(|pack| json!(pack))
            }
            "extension:remove" => {
                let id = text(params, "id");
                let packs = self.packs.lock().await;
                let removed = self.remove(id).await;
                self.handled(packs, [id]);
                removed
            }
            "extension:call" => {
                let ctx = CallCtx {
                    caller: "client".to_owned(),
                    project: params
                        .get("project")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                let args = params.get("args").cloned().unwrap_or(Value::Null);
                let (id, action) = (text(params, "id"), text(params, "action"));
                if let Some(picked) = params.get("picked").and_then(Value::as_str) {
                    self.hand(id, picked).await;
                }
                // A tree drawn for a session (a pane, a shell's log) attaches its extension to it.
                let scope = ["surface", "event"]
                    .iter()
                    .find_map(|k| args.get(k)?.get("scope")?.as_str());
                if let Some(session) = scope.filter(|s| self.session(s).is_some()) {
                    guard(&self.given).show(id, session);
                }
                let _acting = (action != vorn_tree::RENDER).then(|| self.acting(id));
                let answer = match self.builtins.call(id, action, args.clone()).await {
                    Some(answered) => answered.map_err(|e| e.message),
                    None => self
                        .router
                        .call(id, action, args, ctx)
                        .await
                        .map_err(|e| e.message),
                };
                match answer {
                    Ok(tree) if action == vorn_tree::RENDER => tree_checked(id, tree),
                    other => other,
                }
            }
            "extension:contributions" => match self.me.upgrade() {
                Some(me) => blocking(move || Ok(Value::Array(me.contributions()))).await,
                None => Err("vornd is stopping".to_owned()),
            },
            "extension:builtins" => Ok(self.builtins.list()),
            "extension:setBuiltin" => {
                let (id, on) = (
                    text(params, "id"),
                    params.get("on").and_then(Value::as_bool),
                );
                match on {
                    Some(on) => self.builtins.set(id, on).map(|()| {
                        self.contributions_changed(id);
                        json!({ "id": id, "on": on })
                    }),
                    None => Err("extension:setBuiltin takes on: true or false".to_owned()),
                }
            }
            "extension:preferences" => self
                .manifest_of(text(params, "id"))
                .map(|m| Value::Array(self.router.listed_preferences(&m))),
            "extension:setPreference" => {
                let (id, key) = (text(params, "id"), text(params, "key"));
                let value = params.get("value").cloned().unwrap_or(Value::Null);
                match self.manifest_of(id) {
                    Ok(m) => match self.router.set_preference(&m, key, value.clone()) {
                        Ok(()) => {
                            self.contributions_changed(id);
                            self.resync_sessions();
                            Ok(json!({ "id": id, "key": key, "value": value }))
                        }
                        Err(err) => Err(err.message),
                    },
                    Err(err) => Err(err),
                }
            }
            "extension:mcpTools" => self.listed_tools().await.map(|tools| {
                let mut listed = self.builtins.mcp_tools();
                listed.extend(hosted::mcp_tools(|id| self.router.program(id).is_some()));
                listed.extend(tools.into_iter().map(|t| t.listed));
                Value::Array(listed)
            }),
            "extension:mcpCall" => self.mcp_call(params).await,
            "extension:setSandbox" => {
                let (id, on) = (
                    text(params, "id"),
                    params.get("on").and_then(Value::as_bool),
                );
                match on {
                    Some(on) => self
                        .router
                        .set_sandboxed(id, on)
                        .await
                        .map(|()| json!({ "id": id, "sandboxed": on })),
                    None => Err("extension:setSandbox takes on: true or false".to_owned()),
                }
            }
            "extension:rows" => Ok(Value::Array(
                self.router
                    .rows(text(params, "id"), text(params, "collection")),
            )),
            "extension:activation" => match session() {
                Ok(s) => Ok(Value::Array(self.states(&s).await)),
                Err(err) => Err(err),
            },
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
            _ => return Answer::Unanswered,
        };
        answered.map_or_else(Answer::Error, Answer::Result)
    }

    /// The MCP tools installed extensions add, each named `<id>__<tool>`
    /// with the id's `.` and `-` as `_`; a name another took first, or one
    /// too long for MCP, is left out with a warning.
    fn mcp_tools(&self) -> Vec<McpListed> {
        let mut out: Vec<McpListed> = Vec::new();
        for id in self.router.store().ids() {
            let Some(pack) = self.router.program(&id) else {
                continue;
            };
            for tool in &pack.manifest.mcp {
                let name = format!("{}__{}", id.replace(['.', '-'], "_"), tool.name);
                if name.len() > MCP_NAME_MAX || out.iter().any(|t| t.name == name) {
                    warn!("[extensions] {id} offers the MCP tool {name}, which is too long or taken; it is not listed");
                    continue;
                }
                let schema = tool
                    .input
                    .as_ref()
                    .and_then(|p| vorn_extensions::reach::read_in(&pack.path, p).ok())
                    .and_then(|t| serde_json::from_slice::<Value>(&t).ok())
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({ "type": "object" }));
                out.push(McpListed {
                    listed: json!({
                        "name": name,
                        "description": format!("{} (from the extension {})", tool.description, pack.name()),
                        "inputSchema": schema,
                    }),
                    name,
                    extension: id.clone(),
                    tool: tool.name.clone(),
                });
            }
        }
        out
    }

    /// [`Extensions::mcp_tools`], off the runtime: it reads the packages' schema files.
    async fn listed_tools(&self) -> Result<Vec<McpListed>, String> {
        let me = self.me.upgrade().ok_or("vornd is stopping")?;
        blocking(move || Ok(me.mcp_tools())).await
    }

    /// An agent called an extension's MCP tool: `guest.handle("mcp.<tool>")` on its service.
    async fn mcp_call(&self, params: &Value) -> Result<Value, String> {
        let name = text(params, "name");
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(answered) = self.builtins.mcp_call(name, arguments.clone()).await {
            return Ok(answered);
        }
        if hosted::tasks::offers(name) && self.router.program(hosted::tasks::ID).is_some() {
            return Ok(hosted::tasks::mcp_call(self, name, arguments).await);
        }
        let found = self
            .listed_tools()
            .await?
            .into_iter()
            .find(|t| t.name == name);
        let Some(tool) = found else {
            return Err(format!("No extension offers the MCP tool {name}"));
        };
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let ctx = CallCtx {
            caller: "mcp".to_owned(),
            project: None,
        };
        let answer = self
            .router
            .call(&tool.extension, &format!("mcp.{}", tool.tool), args, ctx)
            .await;
        Ok(match answer {
            Ok(Value::String(text)) => json!({ "content": [{ "type": "text", "text": text }] }),
            Ok(other) => {
                let text = serde_json::to_string_pretty(&other).unwrap_or_default();
                json!({ "content": [{ "type": "text", "text": text }] })
            }
            Err(err) => {
                json!({ "content": [{ "type": "text", "text": err.message }], "isError": true })
            }
        })
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
                value["sandboxed"] = json!(self.router.sandboxed(&pack.id));
                // One rule for the switch and the listing: only a program the profile holds.
                let runs = vorn_extensions::sandbox::sandboxable(&pack.manifest);
                value["sandboxAvailable"] = json!(self.router.can_sandbox() && runs);
                // How its own program runs; a package without one says nothing.
                value["sandbox"] = match (
                    runs,
                    self.router.sandbox_kind(),
                    self.router.sandboxed(&pack.id),
                ) {
                    (false, _, _) => Value::Null,
                    (true, None, _) => json!("unsandboxed"),
                    (true, Some(kind), true) => json!(kind.name()),
                    (true, Some(_), false) => json!("off"),
                };
                if runs && self.router.sandboxed(&pack.id) {
                    value["closedProjects"] = json!(self.router.confiner().closed_projects());
                }
                value["preferences"] = json!(self.router.listed_preferences(&pack.manifest));
                if let Some(svg) = pack.logo_svg(false) {
                    value["logo"] = json!(svg);
                }
                if let Some(svg) = pack.logo_svg(true) {
                    value["logoDark"] = json!(svg);
                }
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
            && !self.installs.left_behind(id)
        {
            return Err(format!("No extension \"{id}\" is installed"));
        }
        let closed = guard(&self.grants).close_where(|g| g.extension_id == id);
        guard(&self.given).forget(id);
        self.kill_terminals(&closed);
        self.router.call_hook(id, Hook::Stop).await;
        self.hosts.stop_extension(id).await;
        self.router.forget(id).await;
        self.router.forget_preferences(id);
        // Its connector programs too, before its data goes, so none writes after.
        self.around.pack_stopping(id).await;
        // Its data before its package: while either is left it is still installed, so
        // removing it again finishes the job and nothing a new install of it gets is old.
        let (router, owned) = (Arc::clone(&self.router), id.to_owned());
        if let Err(err) =
            blocking(move || router.forget_files(&owned).map_err(|e| e.to_string())).await
        {
            warn!("[extensions] could not remove the files of {id}: {err}");
            return Err(format!("Could not remove the files {id} kept: {err}"));
        }
        let (installs, owned) = (Arc::clone(&self.installs), id.to_owned());
        blocking(move || installs.retire(&owned)).await?;
        // Out of the installed set: its connections and tables go now, and none
        // comes back with it; its folder goes last, so a failure here is retried.
        let finished = match self.around.pack_removed(id).await {
            Ok(()) => {
                let (installs, owned) = (Arc::clone(&self.installs), id.to_owned());
                blocking(move || installs.purge(&owned)).await
            }
            Err(err) => Err(format!("Could not remove what {id} left: {err}")),
        };
        self.sync_adapters();
        self.contributions_changed(id);
        self.settle_now.notify_one();
        if let Err(err) = &finished {
            warn!("[extensions] {err}");
        }
        finished.map(|()| json!({ "removed": id }))
    }

    /// Finishes a removal of `id` that stopped part way, before `id` is
    /// installed again, so the new one inherits none of it; refused, with
    /// why, when it still cannot finish.
    async fn finish_removal(&self, id: Option<String>) -> Result<(), String> {
        let Some(id) = id.filter(|id| self.installs.left_behind(id)) else {
            return Ok(());
        };
        info!("[extensions] finishing the removal of {id} before installing it");
        self.remove(&id).await.map(drop).map_err(|err| {
            format!("{id} was not fully removed and still cannot be ({err}); nothing was installed")
        })
    }

    /// After an install or a link, nothing runs its old files and its connector, if any, appears.
    async fn package_changed(&self, id: &str) {
        self.pack_changed(id).await;
        self.sync_adapters();
        self.router.lifecycle().stop_extension(id).await;
        // A component is compiled now, by the vorn command, so its first use waits for nothing.
        if let Err(err) = self.router.prepare_installed(id).await {
            warn!("[extensions] {err}");
        }
        self.router.call_hook(id, Hook::Start).await;
        self.around.pack_changed(id).await;
        self.relist.store(true, Ordering::Relaxed);
        self.contributions_changed(id);
    }

    /// Handles each pack changed since it was last handled, here or by a
    /// link, install or removal; whether any changed either way, so every session settles again.
    /// Skipped while a link, install or removal is under way, which wakes the watcher when done.
    async fn check_packs(&self) -> bool {
        let Ok(mut packs) = self.packs.try_lock() else {
            return false;
        };
        let now = fingerprint(self.hosts.store().root());
        let moved = changed(&packs, &now);
        *packs = now;
        for id in &moved {
            debug!("[extensions] pack {id} changed");
            self.pack_changed(id).await;
            self.start_if_stopped(id);
        }
        // An adapter installed from the catalog reaches vornd only as a changed pack.
        if !moved.is_empty() {
            self.sync_adapters();
        }
        let handled = self.resettle.swap(false, Ordering::Relaxed);
        !moved.is_empty() || handled
    }

    /// Records the packs `ids` as they are now, as handled here, and wakes the pack watcher to settle every session.
    fn handled<'a>(
        &self,
        mut packs: tokio::sync::MutexGuard<'_, Fingerprint>,
        ids: impl IntoIterator<Item = &'a str>,
    ) {
        let root = self.hosts.store().root();
        for id in ids {
            match stamp(root, id) {
                Some(now) => packs.insert(id.to_owned(), now),
                None => packs.remove(id),
            };
        }
        self.resettle.store(true, Ordering::Relaxed);
        drop(packs);
        self.settle_now.notify_one();
    }

    /// Whether `method` is `<id>:<action>` for an installed program extension.
    pub fn owns(&self, method: &str) -> bool {
        let Some((id, action)) = method.split_once(':') else {
            return false;
        };
        !RESERVED.contains(&id)
            && !action.is_empty()
            && (self.builtins.has(id) || self.router.program(id).is_some())
    }

    /// Extension `id`'s `action`, built in or installed.
    pub async fn call(
        &self,
        id: &str,
        action: &str,
        args: Value,
        ctx: CallCtx,
    ) -> Result<Value, ExtError> {
        if self.builtins.has(id) {
            let answered = self.builtins.call(id, action, args).await;
            return answered.unwrap_or_else(|| Err(builtin::unknown(id, action)));
        }
        self.router.call(id, action, args, ctx).await
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
        let _acting = self.acting(id);
        match self.call(id, action, params, ctx).await {
            Ok(value) => Answer::Result(value),
            Err(ExtError { message, .. }) => Answer::Error(message),
        }
    }

    /// A person chose `path` for extension `id`, in a file dialog or a form:
    /// it may read that file from now on.
    pub async fn hand(&self, id: &str, path: &str) {
        let path = PathBuf::from(home_path(path));
        let Ok(Ok(real)) =
            tokio::task::spawn_blocking(move || vorn_extensions::reach::resolve(&path)).await
        else {
            return;
        };
        guard(&self.given).hand(id, real);
    }

    /// Counts extension `id` as answering a person's click until the guard drops.
    fn acting(&self, id: &str) -> Acting<'_> {
        guard(&self.given).begin(id);
        Acting {
            given: &self.given,
            id: id.to_owned(),
        }
    }

    /// What a person handed each extension and the sessions each started.
    pub(crate) fn given(&self) -> MutexGuard<'_, Given> {
        guard(&self.given)
    }

    /// Whether extension `id` has a pane open on session `session`, or a
    /// client draws a tree of it there.
    pub(crate) fn paned(&self, id: &str, session: &str) -> bool {
        let open = guard(&self.grants).open_on(id, session, now_ms());
        open || guard(&self.given).attached(id, session)
    }

    /// `terminal.*` on `session` for `caller`: a session it started, or one it
    /// has a pane on with `process.run`.
    pub(crate) fn admit_terminal(
        &self,
        caller: &vorn_extensions::gate::Caller,
        method: &str,
        session: &str,
    ) -> Result<(), ExtError> {
        let started = guard(&self.given).starter_of(&caller.extension, session);
        let paned = self.paned(&caller.extension, session);
        vorn_extensions::gate::admit_terminal(caller, method, started, paned)
    }

    /// The manifest of built-in or installed extension `id`.
    pub(crate) fn manifest_of(
        &self,
        id: &str,
    ) -> Result<Arc<vorn_extensions::manifest::Manifest>, String> {
        if let Some(m) = self.builtins.manifest(id) {
            return Ok(m);
        }
        self.router
            .store()
            .inspect_unmeasured(id)
            .map(|p| Arc::new(p.manifest))
            .map_err(|_| format!("No extension \"{id}\" is installed"))
    }

    /// Syncs every live session again, so what a changed preference shows on a card follows it.
    fn resync_sessions(&self) {
        let (Some(me), Some(live)) = (self.me.upgrade(), self.around.live()) else {
            return;
        };
        for session in live {
            let me = Arc::clone(&me);
            tokio::spawn(async move { me.sync(session).await });
        }
    }

    /// The running session `id`, or the error for a missing one.
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
        packs
            .iter()
            .map(|p| state(p, &subject, &self.router.preferences(&p.manifest)))
            .collect()
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
            let settings = self.router.preferences(&pack.manifest);
            let shown = activation::shown_with(pack, &subject, &settings);
            states.push(state(pack, &subject, &settings));
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
        guard(&self.given).ended(&session.id);
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
        self.network_changed(extension_id).await;
    }

    /// Takes back the way out extension `id`'s programs shared and, when
    /// they had one, stops them, so each starts again under the hosts its
    /// manifest declares now.
    async fn network_changed(&self, id: &str) {
        if self.router.confiner().revoke(id) {
            self.router.lifecycle().stop_extension(id).await;
        }
        self.hosts.stop_extension(id).await;
    }

    /// Runs the start hook of `id` when it is installed and its service is not
    /// running: an install from the catalog reaches vornd only as a changed pack.
    fn start_if_stopped(&self, id: &str) {
        if self.router.program(id).is_none()
            || self
                .router
                .lifecycle()
                .running()
                .iter()
                .any(|k| k.extension == id)
        {
            return;
        }
        let (router, id) = (Arc::clone(&self.router), id.to_owned());
        tokio::spawn(async move { router.call_hook(&id, Hook::Start).await });
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
            PaneDraws::Tree => {
                return Err(format!(
                    "The pane \"{pane_id}\" draws a tree, which the client draws"
                ));
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
        use super::sessions::{set, set_terminal_env, INITIAL_COLS, INITIAL_ROWS};
        let id = uuid::Uuid::new_v4().to_string();
        let mut env = self.around.base_env();
        if let Some(token) = self.hosts.token_for(key) {
            set(&mut env, "VORN_EXTENSION_TOKEN", token);
        }
        let host = format!("{}/extensions/{ext}/bridge", self.bridge_origin);
        set(&mut env, "VORN_EXTENSION_HOST", host);
        set(&mut env, "VORN_SESSION_ID", id.clone());
        set_terminal_env(&mut env);
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
fn state(pack: &InstalledPack, subject: &Subject<'_>, settings: &Map<String, Value>) -> Value {
    let mut state = Map::new();
    state.insert("extensionId".into(), json!(pack.id));
    state.insert("extensionName".into(), json!(pack.name()));
    if let Ok(Value::Object(shown)) =
        serde_json::to_value(activation::shown_with(pack, subject, settings))
    {
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
            let id = entry.file_name().into_string().ok()?;
            let now = stamp(root, &id)?;
            Some((id, now))
        })
        .collect()
}

/// Pack `id`'s current version, if it is installed.
fn stamp(root: &Path, id: &str) -> Option<(Option<SystemTime>, u64)> {
    let meta = std::fs::metadata(root.join(id).join("current.json")).ok()?;
    Some((meta.modified().ok(), meta.len()))
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
    let Some(extensions) = me.upgrade() else {
        return;
    };
    *extensions.packs.lock().await = fingerprint(extensions.hosts.store().root());
    let settle_now = Arc::clone(&extensions.settle_now);
    drop(extensions);
    let mut tick = tokio::time::interval(PACK_CHECK);
    loop {
        tokio::select! {
            () = next_note(&mut notes) => {}
            () = settle_now.notified() => {}
            _ = tick.tick() => {}
        }
        let Some(extensions) = me.upgrade() else {
            return;
        };
        let moved = extensions.check_packs().await;
        reconcile(&extensions, &mut known, moved);
    }
}

/// When each linked package's manifest was last written.
fn manifest_stamps(linked: &[(String, PathBuf)]) -> HashMap<PathBuf, (Option<SystemTime>, u64)> {
    let mut out = HashMap::new();
    for (_, dir) in linked {
        let p = dir.join(vorn_extensions::manifest::file::FILE);
        if let Ok(meta) = std::fs::metadata(&p) {
            out.insert(p, (meta.modified().ok(), meta.len()));
        }
    }
    out
}

/// Tells clients when a linked package's manifest is saved, so what it
/// contributes shows as it is written, and checks each linked folder for a
/// link again as it is relisted and when its manifest changes: one that
/// gained a link is marked not loaded, which [`watch`] then sees as a
/// changed pack and stops.
async fn watch_linked(me: Weak<Extensions>) {
    let mut linked: Vec<(String, PathBuf)> = Vec::new();
    let mut stamps = HashMap::new();
    let mut since_list = PACK_CHECK;
    let mut tick = tokio::time::interval(LINKED_CHECK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(extensions) = me.upgrade() else {
            return;
        };
        since_list += LINKED_CHECK;
        let relist = since_list >= PACK_CHECK || extensions.relist.swap(false, Ordering::Relaxed);
        if relist {
            since_list = Duration::ZERO;
            recheck_linked(&extensions).await;
            let store = extensions.router.store().clone();
            linked = store
                .ids()
                .into_iter()
                .filter_map(|id| store.inspect_unmeasured(&id).ok())
                .filter(|p| p.linked)
                .map(|p| (p.id, p.path))
                .collect();
        }
        if linked.is_empty() {
            stamps.clear();
            continue;
        }
        let now = manifest_stamps(&linked);
        let moved: Vec<&String> = linked
            .iter()
            .filter(|(_, dir)| {
                let mine = |m: &HashMap<PathBuf, _>| -> Vec<_> {
                    let mut v: Vec<_> = m.iter().filter(|(p, _)| p.starts_with(dir)).collect();
                    v.sort_by(|a, b| a.0.cmp(b.0));
                    v.into_iter().map(|(p, s)| (p.clone(), *s)).collect()
                };
                !stamps.is_empty() && mine(&stamps) != mine(&now)
            })
            .map(|(id, _)| id)
            .collect();
        if !moved.is_empty() && !relist {
            recheck_linked(&extensions).await;
        }
        for id in moved {
            debug!("[extensions] {id}'s manifest changed");
            extensions.network_changed(id).await;
            extensions.contributions_changed(id);
        }
        stamps = now;
    }
}

/// Runs [`Installs::recheck_linked`] off the runtime.
async fn recheck_linked(extensions: &Extensions) {
    let installs = Arc::clone(&extensions.installs);
    let _ = tokio::task::spawn_blocking(move || installs.recheck_linked()).await;
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

/// [`Around`] in vornd: the registry, the session host and its own endpoint.
pub struct Wired {
    pub native: Weak<super::Native>,
    pub loopback: Arc<crate::mcp::Loopback>,
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

    fn started_builtins(&self) -> Vec<Arc<dyn builtin::Builtin>> {
        let Some(native) = self.native.upgrade() else {
            return Vec::new();
        };
        builtin::started(&native).into_iter().collect()
    }

    fn adapters(&self) -> Option<Arc<crate::adapters::Adapters>> {
        self.native.upgrade().map(|n| Arc::clone(n.adapters()))
    }

    fn contributions_changed(&self, _id: &str) {
        if let Some(native) = self.native.upgrade() {
            if let Some(work) = native.work() {
                work.sync_types();
            }
        }
    }

    fn pack_changed(&self, id: &str) -> BoxFuture<'static, ()> {
        let connectors = self.native.upgrade().and_then(|n| n.connectors().cloned());
        let id = id.to_owned();
        Box::pin(async move {
            if let Some(connectors) = connectors {
                connectors.inner().package_changed(&id).await;
            }
        })
    }

    fn pack_stopping(&self, id: &str) -> BoxFuture<'static, ()> {
        let connectors = self.native.upgrade().and_then(|n| n.connectors().cloned());
        let id = id.to_owned();
        Box::pin(async move {
            if let Some(connectors) = connectors {
                connectors.inner().package_stopping(&id).await;
            }
        })
    }

    fn pack_removed(&self, id: &str) -> BoxFuture<'static, Result<(), String>> {
        let native = self.native.upgrade();
        let connectors = native.as_ref().and_then(|n| n.connectors().cloned());
        if let Some(n) = &native {
            n.ext_tables().forget(id);
        }
        let db = native.and_then(|n| n.database().map(Path::to_path_buf));
        let id = id.to_owned();
        Box::pin(async move {
            if let Some(connectors) = connectors {
                connectors.inner().package_removed(&id).await?;
            }
            let Some(db) = db else { return Ok(()) };
            let owned = id.clone();
            let dropped = tokio::task::spawn_blocking(move || {
                vorn_store::extension::ExtensionTables::forget(&db, &owned)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
            .map_err(|e| format!("could not drop its tables: {e}"))?;
            if dropped > 0 {
                info!("[extensions] dropped {dropped} tables of {id}");
            }
            Ok(())
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

/// An install sheet with how its program will run: `sandbox` is
/// `sandboxed` or `unsandboxed`, and the closing words say what that means.
/// A package that runs no program of its own (a Wasm component) is left as it is.
fn with_sandbox(
    mut sheet: Value,
    can: Option<vorn_extensions::sandbox::Kind>,
    closed: &[PathBuf],
) -> Value {
    let runs = sheet["commands"].as_array().is_some_and(|commands| {
        commands.iter().any(|c| {
            c["when"]
                .as_str()
                .is_some_and(|w| w == "service" || w.starts_with("action"))
        })
    });
    if !runs {
        return sheet;
    }
    let (word, said) = match can {
        Some(kind) if kind.limits_network() => (
            kind.name(),
            "Its program runs sandboxed: it reads its own folder, writes only its data folder and your projects, and reaches only the hosts above. You can turn the sandbox off for it in Extensions.",
        ),
        Some(kind) => (
            kind.name(),
            "Its program runs sandboxed: it reads its own folder and writes only its data folder and your projects. This machine cannot limit its network, so it can reach any host. You can turn the sandbox off for it in Extensions.",
        ),
        None => (
            "unsandboxed",
            "Its program runs unsandboxed: this machine has no sandbox for programs, so it can reach whatever you can.",
        ),
    };
    sheet["sandbox"] = json!(word);
    let mut said = said.to_owned();
    if can.is_some() {
        for project in closed {
            said.push(' ');
            said.push_str(&closed_line(project));
        }
        sheet["closedProjects"] = json!(closed);
    }
    let before = sheet["runsAsYou"].as_str().unwrap_or_default().to_owned();
    sheet["runsAsYou"] = json!(format!("{before} {said}").trim().to_owned());
    sheet
}

/// What a sandboxed program cannot open, said plainly.
fn closed_line(project: &Path) -> String {
    format!(
        "{} holds Vorn's data, so this extension's programs can't open it.",
        project.display()
    )
}

#[cfg(test)]
mod sandbox_sheet {
    use super::*;

    #[test]
    fn a_sheet_says_how_its_program_runs() {
        let sheet = json!({ "runsAsYou": "X runs as you.", "commands": [{ "when": "service", "argv": ["node"] }] });
        use vorn_extensions::sandbox::Kind;
        let here = with_sandbox(sheet.clone(), Some(Kind::Seatbelt), &[]);
        assert_eq!(here["sandbox"], "sandboxed");
        assert!(here["runsAsYou"]
            .as_str()
            .unwrap()
            .contains("reaches only the hosts above"));
        // Landlock holds files only: the sheet never claims hosts it cannot keep to.
        let files = with_sandbox(sheet.clone(), Some(Kind::Landlock), &[]);
        assert_eq!(files["sandbox"], "files-only");
        let said = files["runsAsYou"].as_str().unwrap();
        assert!(said.contains("cannot limit its network") && !said.contains("only the hosts"));
        let there = with_sandbox(sheet.clone(), None, &[]);
        assert_eq!(there["sandbox"], "unsandboxed");
        let component =
            json!({ "runsAsYou": "C.", "commands": [{ "when": "build", "argv": ["cargo"] }] });
        assert_eq!(with_sandbox(component.clone(), None, &[]), component);
        // A project holding Vorn's data is named, and never given.
        let home = PathBuf::from("/Users/me");
        let named = with_sandbox(sheet, Some(Kind::Seatbelt), &[home]);
        assert!(named["runsAsYou"]
            .as_str()
            .unwrap()
            .ends_with("/Users/me holds Vorn's data, so this extension's programs can't open it."));
        assert_eq!(named["closedProjects"], json!(["/Users/me"]));
    }
}
