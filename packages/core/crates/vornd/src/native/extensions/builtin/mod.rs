//! The extensions Vorn ships with, written against the API a package speaks
//! (a manifest, trees, core calls held to the declared permissions,
//! published rows and subscribed events) and compiled in rather than started
//! as a program: as Rust in vornd, or as a Wasm component vornd carries
//! ([`wasm`]). Each has a switch; off, it subscribes to nothing, publishes
//! nothing and contributes nothing.

pub mod artifacts;
pub mod connectors;
pub mod tasks;
pub mod wasm;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::{Owner, Queue, QUEUE};
use vorn_extensions::gate::{self, Caller};
use vorn_extensions::manifest::{file, Manifest};
use vorn_extensions::points;
use vorn_extensions::router::Router;

use super::guard;

/// What a built-in reaches Vorn through: the gate a package's program goes through.
pub struct Api {
    manifest: Arc<Manifest>,
    caller: Caller,
    router: Arc<Router>,
}

impl Api {
    /// A core method, refused unless the manifest declares what it needs.
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, ExtError> {
        gate::admit_call(&self.manifest, method)?;
        self.router.world().call(&self.caller, method, args).await
    }

    pub fn publish(&self, collection: &str, ops: Vec<RowOp>) {
        self.router.publish(&self.caller, collection, ops);
    }

    /// The rows it published to `collection`, or its preferences for
    /// [`vorn_extensions::preferences::COLLECTION`].
    pub fn rows(&self, collection: &str) -> Vec<Value> {
        self.router.rows_of(&self.manifest, collection)
    }
}

/// One extension Vorn ships with.
pub trait Builtin: Send + Sync + 'static {
    /// Its `vorn-extension.toml`.
    fn manifest(&self) -> &'static str;
    /// The public events it subscribes to.
    fn events(&self) -> &'static [&'static str];
    /// Whether it is on before the person switches it.
    fn on_by_default(&self) -> bool;
    /// Once switched on: publishes what it shows.
    fn start<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()>;
    /// A client's call to one of its actions.
    fn handle<'a>(
        &'a self,
        api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>>;
    /// A batch of the events it subscribes to, in order.
    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()>;
    /// The moment its switch changes, before anything of it starts or stops:
    /// a call that arrives next already sees it.
    fn switched(&self, _on: bool) {}
    /// Once switched off: stops what it runs, keeping what it keeps.
    fn stop<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    /// The MCP tools it adds, as `tools/list` lists them, under their own names.
    fn mcp_tools(&self) -> Vec<Value> {
        Vec::new()
    }
    /// An agent called its tool `name`; `None` when it adds no such tool.
    fn mcp_call<'a>(&'a self, _name: &'a str, _arguments: Value) -> BoxFuture<'a, Option<Value>> {
        Box::pin(async { None })
    }
}

/// Compiles every component Vorn ships into the cache, one after another
/// and Tasks first, so the first task written or board opened after an
/// install or upgrade waits for nothing. A call that comes first compiles as
/// it always did. A test's vornd (`VORN_EXIT_WITH`) skips it: its start stays
/// cheap, and each component compiles at its first use.
pub fn precompile(router: &Arc<Router>) {
    if std::env::var_os(vorn_spawn::watch::EXIT_WITH).is_some() {
        return;
    }
    let shipped: Vec<(&'static str, &'static [u8])> = [
        (tasks::MANIFEST, tasks::WASM),
        (wasm::workflows().manifest, wasm::workflows().wasm),
    ]
    .into_iter()
    .collect();
    let router = Arc::clone(router);
    tokio::spawn(async move {
        for (manifest, wasm) in shipped {
            let Ok(manifest) = file::parse_builtin(manifest) else {
                continue;
            };
            let bundled = vorn_extensions::component::Bundled {
                manifest: Arc::new(manifest),
                wasm,
            };
            if let Err(err) = router.prepare_bundled(&bundled).await {
                warn!(
                    "[extensions] could not compile {} ahead: {err}",
                    bundled.manifest.name
                );
            }
        }
    });
}

/// Whether built-in `id` is switched on in `file`, `default` when the person never switched it.
pub fn switched_on(file: &std::path::Path, id: &str, default: bool) -> bool {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, bool>>(&b).ok())
        .and_then(|m| m.get(id).copied())
        .unwrap_or(default)
}

/// The id Workflows' view, menus and gates went by before they joined Workflows.
const OLD_WORKFLOW_EDITOR: &str = "vorn.workflow-editor";

/// Folds the old Workflow editor's switch into Workflows', once: the merged
/// one is off only if both were off. Says whether `on` changed.
fn merge_workflow_switches(on: &mut BTreeMap<String, bool>) -> bool {
    let Some(editor) = on.remove(OLD_WORKFLOW_EDITOR) else {
        return false;
    };
    let workflows = on.get(vorn_workflows::ID).copied().unwrap_or(true);
    on.insert(vorn_workflows::ID.to_owned(), workflows || editor);
    true
}

struct Entry {
    builtin: Arc<dyn Builtin>,
    manifest: Arc<Manifest>,
    api: Arc<Api>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Entry {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn owner(&self) -> Owner {
        Owner::Core(format!("builtin:{}", self.id()))
    }
}

/// Every built-in and its switch, kept in `builtins.json` beside the packages.
pub struct Builtins {
    entries: Vec<Entry>,
    on: Mutex<BTreeMap<String, bool>>,
    file: PathBuf,
    router: Arc<Router>,
}

impl std::fmt::Debug for Builtins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builtins")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

/// The built-ins this Vorn ships, in the order Settings lists them, with
/// those vornd starts before the extension host (Connectors) after them.
fn shipped(started: Vec<Arc<dyn Builtin>>) -> Vec<Arc<dyn Builtin>> {
    let mut all: Vec<Arc<dyn Builtin>> = vec![
        Arc::new(artifacts::Artifacts),
    ];
    all.extend(started);
    all
}

/// A built-in vornd starts before its extension host, which joins the
/// registry as that host starts if vornd started it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    Connectors,
    Tasks,
    Workflows,
}

impl Started {
    /// Every one, in Settings' order: Workflows' view where it always was, after Tasks.
    pub const ALL: [Started; 3] = [Started::Connectors, Started::Tasks, Started::Workflows];

    /// The manifest its entry is held to, whether or not vornd started it.
    pub fn manifest(self) -> &'static str {
        match self {
            Started::Connectors => connectors::MANIFEST,
            Started::Tasks => tasks::MANIFEST,
            Started::Workflows => vorn_workflows::MANIFEST,
        }
    }

    /// Its registry entry, if `native` started it.
    pub fn entry(self, native: &crate::native::Native) -> Option<Arc<dyn Builtin>> {
        match self {
            Started::Connectors => native.connectors().map(connectors::entry),
            Started::Tasks => native.tasks().map(tasks::entry),
            Started::Workflows => Some(Arc::new(wasm::workflows())),
        }
    }
}

/// The ids of every extension compiled into vornd, carried or started, which
/// share one namespace with installs (services, storage, switches), so no
/// install may take one. A manifest that does not read is an error, not a gap.
pub fn bundled_ids() -> Result<std::collections::BTreeSet<String>, file::FileError> {
    shipped(Vec::new())
        .iter()
        .map(|b| b.manifest())
        .chain(Started::ALL.map(Started::manifest))
        .map(|m| file::parse_builtin(m).map(|m| m.id))
        .collect()
}

impl Builtins {
    pub fn new(router: Arc<Router>, file: PathBuf, started: Vec<Arc<dyn Builtin>>) -> Builtins {
        Builtins::with(router, file, shipped(started))
    }

    pub(crate) fn with(
        router: Arc<Router>,
        file: PathBuf,
        shipped: Vec<Arc<dyn Builtin>>,
    ) -> Builtins {
        let entries = shipped
            .into_iter()
            .filter_map(|builtin| {
                let manifest = match file::parse_builtin(builtin.manifest()) {
                    Ok(m) => Arc::new(m),
                    Err(err) => {
                        warn!("[extensions] a built-in's manifest does not read: {err}");
                        return None;
                    }
                };
                let api = Arc::new(Api {
                    manifest: Arc::clone(&manifest),
                    caller: Caller::builtin(&manifest),
                    router: Arc::clone(&router),
                });
                Some(Entry {
                    builtin,
                    manifest,
                    api,
                    task: Mutex::new(None),
                })
            })
            .collect();
        let mut on = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if merge_workflow_switches(&mut on) {
            let saved = serde_json::to_vec_pretty(&on).unwrap_or_default();
            if let Err(err) = std::fs::write(&file, saved) {
                warn!("[extensions] could not keep the built-ins' switches: {err}");
            }
        }
        Builtins {
            entries,
            on: Mutex::new(on),
            file,
            router,
        }
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id() == id)
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.entry(id).is_some_and(|e| {
            guard(&self.on)
                .get(id)
                .copied()
                .unwrap_or_else(|| e.builtin.on_by_default())
        })
    }

    /// Whether `id` names a built-in, on or off.
    pub fn has(&self, id: &str) -> bool {
        self.entry(id).is_some()
    }

    /// Built-in `id`'s manifest, on or off.
    pub fn manifest(&self, id: &str) -> Option<Arc<Manifest>> {
        self.entry(id).map(|e| Arc::clone(&e.manifest))
    }

    /// Starts every built-in that is on.
    pub fn start(&self) {
        for e in self.entries.iter().filter(|e| self.is_on(e.id())) {
            self.run(e);
        }
    }

    fn run(&self, e: &Entry) {
        let names: Vec<String> = e.builtin.events().iter().map(|n| (*n).to_owned()).collect();
        let (tx, mut rx) = mpsc::channel(QUEUE);
        self.router
            .bus()
            .subscribe_with(e.owner(), &names, Queue { tx, tally: None });
        let (builtin, api) = (Arc::clone(&e.builtin), Arc::clone(&e.api));
        let task = tokio::spawn(async move {
            builtin.start(&api).await;
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                builtin.on_events(&api, batch).await;
            }
        });
        if let Some(old) = guard(&e.task).replace(task) {
            old.abort();
        }
    }

    fn stop(&self, e: &Entry) {
        self.router.bus().unsubscribe(&e.owner());
        if let Some(task) = guard(&e.task).take() {
            task.abort();
        }
        self.router.drop_rows(e.id());
        let (builtin, api) = (Arc::clone(&e.builtin), Arc::clone(&e.api));
        tokio::spawn(async move { builtin.stop(&api).await });
    }

    /// Stops every built-in, for vornd's shutdown.
    pub fn stop_all(&self) {
        for e in &self.entries {
            if let Some(task) = guard(&e.task).take() {
                task.abort();
            }
        }
    }

    /// Switches `id` on or off, and keeps the choice.
    pub fn set(&self, id: &str, on: bool) -> Result<(), String> {
        let e = self
            .entry(id)
            .ok_or_else(|| format!("Vorn ships no extension \"{id}\""))?;
        let was = self.is_on(id);
        let saved = {
            let mut map = guard(&self.on);
            map.insert(id.to_owned(), on);
            serde_json::to_vec_pretty(&*map).unwrap_or_default()
        };
        if let Err(err) = std::fs::write(&self.file, saved) {
            warn!("[extensions] could not keep the built-ins' switches: {err}");
        }
        if was != on {
            e.builtin.switched(on);
        }
        match (was, on) {
            (false, true) => self.run(e),
            (true, false) => self.stop(e),
            _ => {}
        }
        Ok(())
    }

    /// Each built-in, as Settings lists it, saying whether a service of it runs.
    pub fn list(&self) -> Value {
        let running = self.router.lifecycle().running();
        Value::Array(
            self.entries
                .iter()
                .map(|e| {
                    let m = &e.manifest;
                    json!({
                        "id": e.id(),
                        "name": m.name,
                        "description": m.description,
                        "version": m.version,
                        "contributes": m.contributes,
                        "declares": m.declares,
                        "service": m.service,
                        "on": self.is_on(e.id()),
                        "preferences": self.router.listed_preferences(m),
                        "running": running.iter().any(|k| k.extension == e.id()),
                    })
                })
                .collect(),
        )
    }

    /// What the built-ins that are on contribute, as [`points::listed`] gives it.
    pub fn contributions(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for e in self.entries.iter().filter(|e| self.is_on(e.id())) {
            let settings = self.router.preferences(&e.manifest);
            for mut c in points::listed(&e.manifest, &settings) {
                c["extensionName"] = json!(e.manifest.name);
                c["builtin"] = json!(true);
                out.push(c);
            }
        }
        out
    }

    /// The MCP tools the built-ins that are on add.
    pub fn mcp_tools(&self) -> Vec<Value> {
        self.entries
            .iter()
            .filter(|e| self.is_on(e.id()))
            .flat_map(|e| e.builtin.mcp_tools())
            .collect()
    }

    /// An agent called tool `name`: the answer of the built-in that is on and adds it.
    pub async fn mcp_call(&self, name: &str, arguments: Value) -> Option<Value> {
        for e in self.entries.iter().filter(|e| self.is_on(e.id())) {
            if let Some(answered) = e.builtin.mcp_call(name, arguments.clone()).await {
                return Some(answered);
            }
        }
        None
    }

    /// A call to built-in `id`; `None` when no built-in is called that.
    pub async fn call(
        &self,
        id: &str,
        action: &str,
        args: Value,
    ) -> Option<Result<Value, ExtError>> {
        let e = self.entry(id)?;
        if !self.is_on(id) {
            let off = format!(
                "{} is off; switch it on in Settings › Extensions",
                e.manifest.name
            );
            return Some(Err(ExtError::new("off", off)));
        }
        Some(e.builtin.handle(&e.api, action, args).await)
    }
}

pub(crate) fn unknown(name: &str, action: &str) -> ExtError {
    ExtError::new("unknown-action", format!("{name} has no action {action}"))
}

#[cfg(test)]
mod tests;
