//! The router: calls and events in, to programs that speak the extension
//! API, and what those programs ask back, through the gate.
//!
//! An action the manifest gives a command of its own runs one-shot: a
//! process per call, sent one `guest.handle`. Anything else goes to the
//! service, a program or a Wasm component, started by the [`Lifecycle`] on
//! its first call or event. Each program's requests are answered on tasks of
//! their own, so a program waiting on the network holds up nobody else; a
//! component's imports are answered by the same [`answer`] on its own thread.
//! The extensions Vorn ships as components ([`Bundled`]) run the same way.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, error, info, warn};

use crate::api::types::{CallCtx, Event, InitCtx, Level, RowOp};
use crate::api::{params, ExtError, Method, Side};
use crate::blobs::{BlobAnswer, BlobCall, BlobStore};
use crate::child::{CallError, Child, Inbox, Incoming, Launch, Writer};
use crate::component::{Bundled, Compiler, Source};
use crate::events::{self, Bus, Queue, Tally};
use crate::gate::{self, Caller, StorageCall, World};
use crate::install::Trust;
use crate::lifecycle::{Lease, Lifecycle, Running, ServiceKey};
use crate::manifest::{Manifest, Run};
use crate::pack::{InstalledPack, PackStore, Stamp};
use crate::preferences::{self, Preferences};
use crate::sandbox;

/// What every program is started with.
#[derive(Debug, Clone)]
pub struct RouterSettings {
    /// The environment every program starts from.
    pub base_env: Vec<(String, String)>,
    /// The Vorn version `guest.init` names.
    pub version: String,
    /// How long a call may take.
    pub call_timeout: Duration,
    /// How long a service may take to answer `guest.init`.
    pub start_timeout: Duration,
    /// Where each extension's own files are kept (`blobs.*`); none keeps none.
    pub blobs: Option<PathBuf>,
    /// Extensions Vorn ships whose files are kept in a folder of their own, outside the quota.
    pub placed_blobs: Vec<(String, PathBuf)>,
    /// What compiles a Wasm component into the cache when it is not there.
    pub compiler: Compiler,
    /// The file the extensions' preferences are kept in; none keeps them in memory.
    pub preferences: Option<PathBuf>,
    /// The packs this build pins: one installed as such owns Vorn's namespace (`vorn.`).
    pub trust: Trust,
}

impl RouterSettings {
    pub fn new(base_env: Vec<(String, String)>, version: impl Into<String>) -> RouterSettings {
        RouterSettings {
            base_env,
            version: version.into(),
            call_timeout: Duration::from_secs(60),
            start_timeout: Duration::from_secs(15),
            blobs: None,
            placed_blobs: Vec::new(),
            compiler: Compiler::None,
            preferences: None,
            trust: Trust::default(),
        }
    }
}

/// When Vorn calls an installed extension's own action ([`crate::manifest::Service`]'s `start` and `stop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    /// Vorn started, or the extension was just installed or updated.
    Start,
    /// It is about to be uninstalled.
    Stop,
}

/// The most events one `guest.on-event` carries.
const MAX_BATCH: usize = 256;
/// How many of one program's own requests are answered at once.
const REQUESTS_AT_ONCE: usize = 16;
/// Rows a collection keeps; past this an upsert of a new row is dropped.
const MAX_ROWS: usize = 10_000;
/// The JSON-RPC codes for a method nobody answers and params that do not read.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// A service speaking the extension API: a program, or a Wasm component in its place.
#[derive(Debug)]
pub enum Program {
    Child(Child),
    #[cfg(feature = "wasm")]
    Component(crate::wasm::Service),
}

impl Running for Program {
    fn name(&self) -> &str {
        match self {
            Program::Child(c) => c.name(),
            #[cfg(feature = "wasm")]
            Program::Component(c) => c.name(),
        }
    }

    fn exited(&self) -> bool {
        match self {
            Program::Child(c) => c.exited(),
            #[cfg(feature = "wasm")]
            Program::Component(c) => c.exited(),
        }
    }

    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        match self {
            Program::Child(c) => Box::pin(c.close()),
            #[cfg(feature = "wasm")]
            Program::Component(c) => c.close(),
        }
    }
}

impl Program {
    async fn handle(
        &self,
        params: params::GuestHandle,
        timeout: Duration,
    ) -> Result<Value, CallError> {
        match self {
            Program::Child(c) => {
                c.request(Method::GuestHandle.name(), json!(params), timeout)
                    .await
            }
            #[cfg(feature = "wasm")]
            Program::Component(c) => {
                c.handle(params.method, params.args, params.ctx, timeout)
                    .await
            }
        }
    }

    async fn on_event(
        &self,
        params: params::GuestOnEvent,
        timeout: Duration,
    ) -> Result<(), CallError> {
        match self {
            Program::Child(c) => {
                let _ = timeout;
                c.notify(Method::GuestOnEvent.name(), json!(params)).await
            }
            #[cfg(feature = "wasm")]
            Program::Component(c) => c.on_event(params.batch, timeout).await,
        }
    }

    /// The process id of a program; `None` for a component, which runs in the host.
    pub fn pid(&self) -> Option<u32> {
        match self {
            Program::Child(c) => c.pid(),
            #[cfg(feature = "wasm")]
            Program::Component(_) => None,
        }
    }

    /// The most linear memory a component's instance has held; `None` for a program.
    pub fn component_memory(&self) -> Option<usize> {
        match self {
            Program::Child(_) => None,
            #[cfg(feature = "wasm")]
            Program::Component(c) => Some(c.memory()),
        }
    }
}

type Rows = HashMap<(String, String), Map<String, Value>>;

/// Packages as last read, re-read only when their files change.
type Packs = HashMap<String, (Stamp, Arc<InstalledPack>)>;

/// Calls and events in, rows out.
pub struct Router {
    store: PackStore,
    lifecycle: Arc<Lifecycle>,
    world: Arc<dyn World>,
    settings: RouterSettings,
    rows: Mutex<Rows>,
    packs: Mutex<Packs>,
    bus: Arc<Bus>,
    /// Each subscribed extension's queue, read by its own delivery task.
    queues: Mutex<HashMap<String, Queue>>,
    blobs: Option<BlobStore>,
    preferences: Mutex<Preferences>,
    me: Weak<Router>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("root", &self.store.root())
            .finish_non_exhaustive()
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A failed call to a program, as the API's error.
pub fn ext_error(err: CallError) -> ExtError {
    match err {
        CallError::Answered { message, data, .. } => {
            ExtError::new(data.code.as_deref().unwrap_or("failed"), message)
        }
        CallError::Transport(message) => ExtError::new("exited", message),
        CallError::Timeout(message) => ExtError::new("timeout", message),
    }
}

impl Router {
    /// A router delivering `bus`'s events to the extensions that subscribe;
    /// none is subscribed until [`Router::reindex`].
    pub fn new(
        store: PackStore,
        lifecycle: Arc<Lifecycle>,
        world: Arc<dyn World>,
        settings: RouterSettings,
        bus: Arc<Bus>,
    ) -> Arc<Router> {
        let blobs = settings.blobs.clone().map(|root| {
            settings
                .placed_blobs
                .iter()
                .fold(BlobStore::new(root), |s, (id, folder)| s.place(id, folder))
        });
        let preferences = Mutex::new(Preferences::load(settings.preferences.clone()));
        Arc::new_cyclic(|me| Router {
            preferences,
            store,
            lifecycle,
            world,
            settings,
            rows: Mutex::default(),
            packs: Mutex::default(),
            bus,
            queues: Mutex::default(),
            blobs,
            me: me.clone(),
        })
    }

    pub fn bus(&self) -> &Arc<Bus> {
        &self.bus
    }

    /// What lies behind the gate, for a built-in extension's calls.
    pub fn world(&self) -> &Arc<dyn World> {
        &self.world
    }

    /// Drops every row extension `id` published.
    pub fn drop_rows(&self, id: &str) {
        guard(&self.rows).retain(|(ext, _), _| ext != id);
    }

    /// Reads which events each installed program subscribes to into the
    /// bus's index. A queue kept from before keeps what it holds, in order.
    pub fn reindex(&self) {
        let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
        for id in self.store.ids() {
            let Some(pack) = self.program(&id) else {
                continue;
            };
            let names = pack
                .manifest
                .service
                .as_ref()
                .map(|s| s.events.clone())
                .unwrap_or_default();
            if !names.is_empty() {
                wanted.push((id, names));
            }
        }
        let mut queues = guard(&self.queues);
        queues.retain(|id, _| wanted.iter().any(|(w, _)| w == id));
        let mut subscribed = Vec::with_capacity(wanted.len());
        for (id, names) in wanted {
            let queue = queues
                .entry(id.clone())
                .or_insert_with(|| {
                    let (tx, rx) = mpsc::channel(events::QUEUE);
                    let tally = Arc::new(Tally::default());
                    tokio::spawn(deliver(self.me.clone(), id.clone(), rx, Arc::clone(&tally)));
                    Queue {
                        tx,
                        tally: Some(tally),
                    }
                })
                .clone();
            subscribed.push((id, names, queue));
        }
        self.bus.set_extensions(subscribed);
    }

    /// Resolves once every event queued for extension `id` so far has been
    /// handed to it, or dropped because it could not start.
    pub async fn settled(&self, id: &str) {
        let tally = guard(&self.queues).get(id).and_then(|q| q.tally.clone());
        if let Some(tally) = tally {
            tally.settled().await;
        }
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    pub fn lifecycle(&self) -> &Arc<Lifecycle> {
        &self.lifecycle
    }

    /// Package `id`, from what was last read of it while its files are
    /// unchanged: two `stat`s a call instead of a read and a parse.
    pub fn pack(&self, id: &str) -> Result<Arc<InstalledPack>, String> {
        let cached = guard(&self.packs).get(id).cloned();
        if let Some((stamp, pack)) = cached {
            if self.store.stamp(&pack) == Some(stamp) {
                return Ok(pack);
            }
        }
        let pack = Arc::new(self.store.inspect_unmeasured(id)?);
        if let Some(stamp) = self.store.stamp(&pack) {
            guard(&self.packs).insert(id.to_owned(), (stamp, Arc::clone(&pack)));
        }
        Ok(pack)
    }

    /// Whether extension `id`'s programs start under the macOS sandbox profile.
    pub fn sandboxed(&self, id: &str) -> bool {
        sandbox::available() && sandbox::is_on(&self.store.root().join(id))
    }

    /// Turns the sandbox profile on or off for extension `id`; its running
    /// programs stop, so the next start is under the new setting.
    pub async fn set_sandboxed(&self, id: &str, on: bool) -> Result<(), String> {
        let pack = self.pack(id)?;
        if pack.manifest.service.is_none() && pack.manifest.runs.is_empty() {
            return Err(format!("{} runs no program to hold", pack.name()));
        }
        let component = pack
            .manifest
            .service
            .as_ref()
            .is_some_and(|s| s.component.is_some());
        if component && pack.manifest.runs.is_empty() {
            return Err(format!(
                "{} runs as a Wasm component, which Vorn sandboxes always",
                pack.name()
            ));
        }
        sandbox::set(&self.store.root().join(id), on)?;
        self.lifecycle.stop_extension(id).await;
        Ok(())
    }

    /// Package `id`, if it is a program in the extension API.
    pub fn program(&self, id: &str) -> Option<Arc<InstalledPack>> {
        self.pack(id).ok().filter(|p| p.manifest.speaks_api())
    }

    fn program_pack(&self, id: &str) -> Result<Arc<InstalledPack>, ExtError> {
        let pack = self
            .pack(id)
            .map_err(|e| ExtError::new("not-installed", e))?;
        if !pack.manifest.speaks_api() {
            return Err(ExtError::new(
                "unsupported",
                format!(
                    "{} speaks the connector protocol, so it is called through its connection",
                    pack.name()
                ),
            ));
        }
        Ok(pack)
    }

    /// Calls `action` of extension `id`: one-shot when the manifest gives it
    /// a command, else on the service.
    pub async fn call(
        &self,
        id: &str,
        action: &str,
        args: Value,
        ctx: CallCtx,
    ) -> Result<Value, ExtError> {
        let pack = self.program_pack(id)?;
        if let Some(run) = pack.manifest.run(action) {
            return self.one_shot(&pack, run, action, args, ctx).await;
        }
        if pack.manifest.service.is_none() {
            return Err(ExtError::new(
                "unknown-action",
                format!("{} has no action {action}", pack.name()),
            ));
        }
        let key = ServiceKey::new(id, "");
        let lease = self
            .lifecycle
            .get_or_start(&key, || self.start_service(&pack))
            .await
            .map_err(|e| ExtError::new("start", e))?;
        self.serve(&key, lease, action, args, ctx).await
    }

    /// Calls `action` of the extension Vorn ships as component `bundled`.
    pub async fn call_bundled(
        &self,
        bundled: &Bundled,
        action: &str,
        args: Value,
        ctx: CallCtx,
    ) -> Result<Value, ExtError> {
        let key = ServiceKey::new(bundled.manifest.id.as_str(), "");
        let lease = self
            .lifecycle
            .get_or_start(&key, || self.start_bundled(bundled))
            .await
            .map_err(|e| ExtError::new("start", e))?;
        self.serve(&key, lease, action, args, ctx).await
    }

    /// Hands `batch` to the extension Vorn ships as component `bundled`, starting it for the first.
    pub async fn hand_bundled(&self, bundled: &Bundled, batch: &[Arc<Event>]) {
        let key = ServiceKey::new(bundled.manifest.id.as_str(), "");
        match self
            .lifecycle
            .get_or_start(&key, || self.start_bundled(bundled))
            .await
        {
            Ok(lease) => self.deliver_to(&key.extension, &lease, batch).await,
            Err(err) => warn!(
                "[extensions] {} missed {} events: {err}",
                key.extension,
                batch.len()
            ),
        }
    }

    /// Compiles `bundled` into the cache unless it is there, without starting it.
    pub async fn prepare_bundled(&self, bundled: &Bundled) -> Result<(), String> {
        self.prepare(&Source::Bundled(bundled.wasm), &bundled.manifest.name)
            .await
    }

    /// Compiles installed extension `id`'s component into the cache unless it
    /// is there, so its first start waits for nothing; nothing to do for a program.
    pub async fn prepare_installed(&self, id: &str) -> Result<(), String> {
        let pack = self.pack(id)?;
        let Some(component) = pack
            .manifest
            .service
            .as_ref()
            .and_then(|s| s.component.clone())
        else {
            return Ok(());
        };
        self.prepare(&Source::File(pack.path.join(component)), pack.name())
            .await
    }

    #[cfg(feature = "wasm")]
    async fn prepare(&self, source: &Source, name: &str) -> Result<(), String> {
        let cache = self.store.root().join(crate::component::COMPILED);
        crate::wasm::prepare(source, name, &cache, &self.settings.compiler).await
    }

    #[cfg(not(feature = "wasm"))]
    #[allow(clippy::unused_async)]
    async fn prepare(&self, _source: &Source, name: &str) -> Result<(), String> {
        Err(format!(
            "{name} is a Wasm component, and this Vorn runs none"
        ))
    }

    /// Calls installed extension `id`'s `hook` action, when its manifest
    /// names one. A failure is logged: nothing a person waits for depends on it.
    pub async fn call_hook(&self, id: &str, hook: Hook) {
        let Some(pack) = self.program(id) else {
            return;
        };
        let Some(action) = pack.manifest.service.as_ref().and_then(|s| match hook {
            Hook::Start => s.start.clone(),
            Hook::Stop => s.stop.clone(),
        }) else {
            return;
        };
        let ctx = CallCtx {
            caller: "client".to_owned(),
            project: None,
        };
        if let Err(e) = self.call(id, &action, Value::Null, ctx).await {
            warn!("[extensions] {} {action}: {}", pack.name(), e.message);
        }
    }

    /// Calls the start action of every installed extension whose manifest names one, one after another.
    pub async fn start_installed(&self) {
        let store = self.store.clone();
        let ids = tokio::task::spawn_blocking(move || store.ids())
            .await
            .unwrap_or_default();
        for id in ids {
            self.call_hook(&id, Hook::Start).await;
        }
    }

    /// Stops the service of extension `id`, whatever it runs as.
    pub async fn stop(&self, id: &str) {
        self.lifecycle.stop(&ServiceKey::new(id, "")).await;
    }

    /// `guest.handle(action)` on a running service, which a timeout stops.
    async fn serve(
        &self,
        key: &ServiceKey,
        lease: Lease<Program>,
        action: &str,
        args: Value,
        ctx: CallCtx,
    ) -> Result<Value, ExtError> {
        let params = params::GuestHandle {
            method: action.to_owned(),
            args,
            ctx,
        };
        let answer = lease.handle(params, self.settings.call_timeout).await;
        drop(lease);
        match answer {
            Ok(value) => Ok(value),
            Err(err @ CallError::Timeout(_)) => {
                // A service that let a call time out may be wedged; the next call starts it fresh.
                warn!("[extensions] {} timed out; stopping it", key.extension);
                self.lifecycle.stop(key).await;
                Err(ext_error(err))
            }
            Err(err) => Err(ext_error(err)),
        }
    }

    /// Queues `event` for every service subscribed to it, which starts on
    /// its first event; answers how many it was queued for.
    pub fn emit(&self, event: &str, data: Value) -> usize {
        self.bus.emit(event, || data)
    }

    /// The rows extension `id` published to `collection`, in the order they came.
    pub fn rows(&self, id: &str, collection: &str) -> Vec<Value> {
        guard(&self.rows)
            .get(&(id.to_owned(), collection.to_owned()))
            .map(|rows| rows.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The preferences `manifest` declares, by id: what the person chose, or the default.
    pub fn preferences(&self, manifest: &Manifest) -> Map<String, Value> {
        guard(&self.preferences).resolved(manifest)
    }

    /// The preferences `manifest` declares as Settings draws them, each with its value.
    pub fn listed_preferences(&self, manifest: &Manifest) -> Vec<Value> {
        guard(&self.preferences).listed(manifest)
    }

    /// The rows `manifest`'s extension reads from `collection`: its
    /// preferences for [`preferences::COLLECTION`], else what it published.
    pub fn rows_of(&self, manifest: &Manifest, collection: &str) -> Vec<Value> {
        if collection == preferences::COLLECTION {
            return guard(&self.preferences).rows(manifest);
        }
        self.rows(&manifest.id, collection)
    }

    /// Keeps a person's `value` for preference `key` of `manifest`'s
    /// extension, then tells clients drawing its rows, so its trees draw again.
    pub fn set_preference(
        &self,
        manifest: &Manifest,
        key: &str,
        value: Value,
    ) -> Result<(), ExtError> {
        let row = guard(&self.preferences).set(manifest, key, value)?;
        let caller = Caller::of(manifest);
        let told = [json!({ "op": "upsert", "row": row })];
        self.world
            .published(&caller, preferences::COLLECTION, &told);
        Ok(())
    }

    /// Forgets the preferences a person gave extension `id`, as it is removed.
    pub fn forget_preferences(&self, id: &str) {
        guard(&self.preferences).forget(id);
    }

    /// Removes every file extension `id` keeps in its own store; blocks on the disk.
    pub fn forget_files(&self, id: &str) -> std::io::Result<()> {
        match &self.blobs {
            Some(store) => store.forget(id),
            None => Ok(()),
        }
    }

    /// Where extension `id`'s file `name` is on disk, for the core to hand on
    /// (a task's image to an agent); `None` when this Vorn keeps no files.
    pub fn blob_path(&self, id: &str, name: &str) -> Option<Result<PathBuf, ExtError>> {
        self.blobs.as_ref().map(|s| s.path(id, name))
    }

    /// Runs `call` on extension `id`'s own files, off the runtime.
    async fn blobs(&self, id: &str, call: BlobCall) -> Result<Value, ExtError> {
        let store = self
            .blobs
            .clone()
            .ok_or_else(|| ExtError::new("unsupported", "this Vorn keeps no extension files"))?;
        let id = id.to_owned();
        let answer = tokio::task::spawn_blocking(move || store.run(&id, call))
            .await
            .map_err(|e| ExtError::new("failed", e.to_string()))??;
        Ok(match answer {
            BlobAnswer::Put => Value::Null,
            BlobAnswer::Got(Some(bytes)) => crate::api::base64::encode(&bytes),
            BlobAnswer::Got(None) => Value::Null,
            BlobAnswer::Listed(blobs) => json!(blobs),
            BlobAnswer::Deleted(was) => Value::Bool(was),
        })
    }

    /// Stops extension `id` and drops what it published.
    pub async fn forget(&self, id: &str) {
        self.lifecycle.stop_extension(id).await;
        guard(&self.rows).retain(|(ext, _), _| ext != id);
        guard(&self.packs).remove(id);
        self.reindex();
    }

    fn launch(&self, pack: &InstalledPack, argv: &[String]) -> Result<Launch, String> {
        let (program, args) = pack
            .resolve(argv)
            .ok_or_else(|| format!("{} names no program to run", pack.name()))?;
        let mut env = self.settings.base_env.clone();
        env.retain(|(k, _)| k != "VORN_EXTENSION_ID");
        env.push(("VORN_EXTENSION_ID".into(), pack.id.clone()));
        let launch = Launch {
            program,
            args,
            cwd: PathBuf::from(&pack.path),
            env,
        };
        if self.sandboxed(&pack.id) {
            return Ok(sandbox::wrap(launch, &pack.path));
        }
        Ok(launch)
    }

    /// Starts `pack`'s program with an inbox for what it asks, answered on its own tasks.
    fn spawn_program(&self, pack: &InstalledPack, argv: &[String]) -> Result<Child, String> {
        let launch = self.launch(pack, argv)?;
        let (tx, rx): (Inbox, _) = mpsc::channel(64);
        let child = Child::start_with(&launch, format!("[extensions] {}", pack.name()), Some(tx))?;
        let caller = self.caller_of(pack);
        tokio::spawn(dispatch(
            self.me.clone(),
            Arc::new(pack.manifest.clone()),
            caller,
            child.writer(),
            rx,
        ));
        Ok(child)
    }

    async fn start_service(&self, pack: &InstalledPack) -> Result<Program, String> {
        let service = pack
            .manifest
            .service
            .as_ref()
            .ok_or_else(|| format!("{} has no service", pack.name()))?;
        let ctx = InitCtx {
            id: pack.id.clone(),
            version: pack.version.clone(),
            vorn: self.settings.version.clone(),
            api: crate::api::API.to_owned(),
        };
        if let Some(component) = &service.component {
            let source = Source::File(pack.path.join(component));
            let caller = self.caller_of(pack);
            return self
                .start_component(source, Arc::new(pack.manifest.clone()), caller, ctx)
                .await;
        }
        let child = self.spawn_program(pack, &service.command)?;
        let init = params::GuestInit { ctx };
        let answered = child
            .request(
                Method::GuestInit.name(),
                json!(init),
                self.settings.start_timeout,
            )
            .await;
        if let Err(err) = answered {
            child.close().await;
            return Err(format!("{} did not start: {}", pack.name(), ext_error(err)));
        }
        info!("[extensions] {} started", pack.name());
        Ok(Program::Child(child))
    }

    /// Who installed `pack` asks as: Vorn's own when it is the pack this build pins for its id.
    fn caller_of(&self, pack: &InstalledPack) -> Caller {
        Caller {
            extension: pack.id.clone(),
            builtin: crate::pack::is_reserved(&pack.id) && self.settings.trust.vouches_for(pack),
            ..Caller::of(&pack.manifest)
        }
    }

    async fn start_bundled(&self, bundled: &Bundled) -> Result<Program, String> {
        let m = &bundled.manifest;
        let ctx = InitCtx {
            id: m.id.clone(),
            version: m.version.clone(),
            vorn: self.settings.version.clone(),
            api: crate::api::API.to_owned(),
        };
        let caller = Caller::builtin(m);
        self.start_component(Source::Bundled(bundled.wasm), Arc::clone(m), caller, ctx)
            .await
    }

    #[cfg(feature = "wasm")]
    async fn start_component(
        &self,
        source: Source,
        manifest: Arc<Manifest>,
        caller: Caller,
        init: InitCtx,
    ) -> Result<Program, String> {
        let name = format!("[extensions] {}", manifest.name);
        let start = crate::wasm::Start {
            source,
            cache: self.store.root().join(crate::component::COMPILED),
            compiler: self.settings.compiler.clone(),
            name,
            init,
            start_timeout: self.settings.start_timeout,
            router: self.me.clone(),
            manifest: Arc::clone(&manifest),
            caller,
        };
        let service = crate::wasm::Service::start(start).await?;
        info!("[extensions] {} started as a component", manifest.name);
        Ok(Program::Component(service))
    }

    #[cfg(not(feature = "wasm"))]
    #[allow(clippy::unused_async)]
    async fn start_component(
        &self,
        _source: Source,
        manifest: Arc<Manifest>,
        _caller: Caller,
        _init: InitCtx,
    ) -> Result<Program, String> {
        Err(format!(
            "{} is a Wasm component, and this Vorn runs none",
            manifest.name
        ))
    }

    /// The running service of extension `id`, without starting it.
    pub fn running(&self, id: &str) -> Option<Lease<Program>> {
        self.lifecycle.get(&ServiceKey::new(id, ""))
    }

    async fn one_shot(
        &self,
        pack: &InstalledPack,
        run: &Run,
        action: &str,
        args: Value,
        ctx: CallCtx,
    ) -> Result<Value, ExtError> {
        let child = self
            .spawn_program(pack, &run.command)
            .map_err(|e| ExtError::new("start", e))?;
        let params = params::GuestHandle {
            method: action.to_owned(),
            args,
            ctx,
        };
        // The first request a child gets has id 1, which a script may answer by rote.
        let answer = child
            .request(
                Method::GuestHandle.name(),
                json!(params),
                self.settings.call_timeout,
            )
            .await;
        // Its stdin closes now, and it is stopped if it lingers; nobody waits on that.
        tokio::spawn(async move { child.close().await });
        answer.map_err(ext_error)
    }

    /// Applies `ops` to `caller`'s rows in `collection`; clients drawing them are told the ops that held.
    pub fn publish(&self, caller: &Caller, collection: &str, ops: Vec<RowOp>) {
        let mut told = Vec::with_capacity(ops.len());
        {
            let mut rows = guard(&self.rows);
            let kept = rows
                .entry((caller.extension.clone(), collection.to_owned()))
                .or_default();
            for op in ops {
                match op {
                    RowOp::Upsert(row) => {
                        let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_owned)
                        else {
                            warn!(
                                "[extensions] {} published a row without an id",
                                caller.extension
                            );
                            continue;
                        };
                        if kept.len() >= MAX_ROWS && !kept.contains_key(&id) {
                            warn!(
                                "[extensions] {} holds {MAX_ROWS} rows in {collection}",
                                caller.extension
                            );
                            continue;
                        }
                        told.push(json!({ "op": "upsert", "row": row }));
                        kept.insert(id, row);
                    }
                    RowOp::Delete(id) => {
                        if kept.shift_remove(&id).is_some() {
                            told.push(json!({ "op": "delete", "id": id }));
                        }
                    }
                    RowOp::Clear => {
                        kept.clear();
                        told.push(json!({ "op": "clear" }));
                    }
                }
            }
        }
        if !told.is_empty() {
            self.world.published(caller, collection, &told);
        }
    }
}

/// Hands extension `id` its events in batches, in the order they came,
/// starting its service for the first; ends when its queue goes.
async fn deliver(
    router: Weak<Router>,
    id: String,
    mut rx: mpsc::Receiver<Arc<Event>>,
    tally: Arc<Tally>,
) {
    while let Some((batch, taken)) = events::next_batch(&mut rx, MAX_BATCH).await {
        if let Some(router) = router.upgrade() {
            router.hand(&id, &batch).await;
        }
        tally.handled(taken);
    }
}

impl Router {
    async fn hand(&self, id: &str, batch: &[Arc<Event>]) {
        let Some(pack) = self.program(id) else {
            return;
        };
        let key = ServiceKey::new(id, "");
        let lease = match self
            .lifecycle
            .get_or_start(&key, || self.start_service(&pack))
            .await
        {
            Ok(lease) => lease,
            Err(err) => {
                warn!("[extensions] {id} missed {} events: {err}", batch.len());
                return;
            }
        };
        self.deliver_to(id, &lease, batch).await;
    }

    async fn deliver_to(&self, id: &str, lease: &Lease<Program>, batch: &[Arc<Event>]) {
        let params = params::GuestOnEvent {
            batch: batch.iter().map(|e| Event::clone(e)).collect(),
        };
        if let Err(err) = lease.on_event(params, self.settings.start_timeout).await {
            warn!(
                "[extensions] {id} missed {} events: {}",
                batch.len(),
                ext_error(err)
            );
        }
    }
}

/// Answers each request a program makes, each on a task of its own.
async fn dispatch(
    router: Weak<Router>,
    manifest: Arc<Manifest>,
    caller: Caller,
    writer: Writer,
    mut inbox: mpsc::Receiver<Incoming>,
) {
    let slots = Arc::new(Semaphore::new(REQUESTS_AT_ONCE));
    while let Some(incoming) = inbox.recv().await {
        let Some(router) = router.upgrade() else {
            return;
        };
        let Ok(permit) = Arc::clone(&slots).acquire_owned().await else {
            return;
        };
        let (manifest, caller, writer) = (Arc::clone(&manifest), caller.clone(), writer.clone());
        tokio::spawn(async move {
            let id = incoming.id.clone();
            let answer = answer(&router, &manifest, &caller, incoming).await;
            if let (Some(id), Some(answer)) = (id, answer) {
                writer.reply(id, answer).await;
            }
            drop(permit);
        });
    }
}

type Reply = Result<Value, (i64, String, Value)>;

/// A `storage.*` request's params, read.
fn storage_call(method: Method, params: Value) -> Result<StorageCall, serde_json::Error> {
    Ok(match method {
        Method::StorageQuery => {
            let p: params::StorageQuery = serde_json::from_value(params)?;
            StorageCall::Query {
                sql: p.sql,
                params: p.params,
            }
        }
        Method::StorageExec => {
            let p: params::StorageExec = serde_json::from_value(params)?;
            StorageCall::Exec {
                sql: p.sql,
                params: p.params,
            }
        }
        Method::StorageGet => {
            let p: params::StorageGet = serde_json::from_value(params)?;
            StorageCall::Get { key: p.key }
        }
        Method::StorageSet => {
            let p: params::StorageSet = serde_json::from_value(params)?;
            StorageCall::Set {
                key: p.key,
                value: p.value,
            }
        }
        Method::StorageTransaction => {
            let p: params::StorageTransaction = serde_json::from_value(params)?;
            StorageCall::Transaction(
                p.statements
                    .into_iter()
                    .map(|s| (s.sql, s.params))
                    .collect(),
            )
        }
        _ => StorageCall::Prefix,
    })
}

/// A `blobs.*` request's params, read.
fn blob_call(method: Method, params: Value) -> Result<BlobCall, serde_json::Error> {
    Ok(match method {
        Method::BlobsPut => {
            let p: params::BlobsPut = serde_json::from_value(params)?;
            BlobCall::Put {
                name: p.name,
                data: p.data,
            }
        }
        Method::BlobsGet => {
            let p: params::BlobsGet = serde_json::from_value(params)?;
            BlobCall::Get { name: p.name }
        }
        Method::BlobsList => {
            let p: params::BlobsList = serde_json::from_value(params)?;
            BlobCall::List { prefix: p.prefix }
        }
        _ => {
            let p: params::BlobsDelete = serde_json::from_value(params)?;
            BlobCall::Delete { name: p.name }
        }
    })
}

fn invalid(method: Method, err: &serde_json::Error) -> Option<Reply> {
    Some(Err((
        INVALID_PARAMS,
        format!("{} has params that do not read: {err}", method.name()),
        Value::Null,
    )))
}

fn from_ext(result: Result<Value, ExtError>) -> Option<Reply> {
    Some(result.map_err(|e| e.to_rpc()))
}

/// One request a program made, or one import a component called; `None` for one that is not answered.
pub(crate) async fn answer(
    router: &Router,
    manifest: &Manifest,
    caller: &Caller,
    incoming: Incoming,
) -> Option<Reply> {
    let Some(method) = Method::from_name(&incoming.method) else {
        return Some(Err((
            METHOD_NOT_FOUND,
            format!("Vorn has no method {}", incoming.method),
            Value::Null,
        )));
    };
    if method.side() == Side::Guest {
        return Some(Err((
            METHOD_NOT_FOUND,
            format!(
                "{} is answered by the extension, not by Vorn",
                method.name()
            ),
            Value::Null,
        )));
    }
    let params = incoming.params;
    // Every method of the world is matched here, so a function added to the WIT cannot go unanswered.
    match method {
        Method::HostCall => {
            let p: params::HostCall = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            let result = match gate::admit_call(manifest, &p.method) {
                Ok(_) => router.world.call(caller, &p.method, p.args).await,
                Err(e) => Err(e),
            };
            from_ext(result)
        }
        Method::HostPublish => {
            let p: params::HostPublish = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            if p.collection == preferences::COLLECTION {
                warn!(
                    "[extensions] {} published to {}, which only Vorn writes",
                    caller.extension,
                    preferences::COLLECTION
                );
                return None;
            }
            router.publish(caller, &p.collection, p.ops);
            None
        }
        Method::HostLog => {
            let p: params::HostLog = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            let ext = &caller.extension;
            match p.level {
                Level::Debug => debug!("[extensions] {ext}: {}", p.message),
                Level::Info => info!("[extensions] {ext}: {}", p.message),
                Level::Warn => warn!("[extensions] {ext}: {}", p.message),
                Level::Error => error!("[extensions] {ext}: {}", p.message),
            }
            None
        }
        Method::HostRows => {
            let p: params::HostRows = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            Some(Ok(Value::Array(router.rows_of(manifest, &p.collection))))
        }
        Method::HostStartSession => {
            let p: params::HostStartSession = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            let result = match gate::admit_start(manifest) {
                Ok(()) => router
                    .world
                    .start_session(caller, p.req)
                    .await
                    .map(Value::String),
                Err(e) => Err(e),
            };
            from_ext(result)
        }
        Method::NetFetch => {
            let p: params::NetFetch = match serde_json::from_value(params) {
                Ok(p) => p,
                Err(e) => return invalid(method, &e),
            };
            let result = gate::fetch(&router.world, manifest, caller, p.req, p.connection)
                .await
                .map(|r| json!(r));
            from_ext(result)
        }
        Method::StoragePrefix
        | Method::StorageQuery
        | Method::StorageExec
        | Method::StorageGet
        | Method::StorageSet
        | Method::StorageTransaction => {
            let call = match storage_call(method, params) {
                Ok(call) => call,
                Err(e) => return invalid(method, &e),
            };
            let result = match gate::admit_storage(manifest) {
                Ok(()) => {
                    let migrations = router
                        .program(&caller.extension)
                        .and_then(|p| p.migrations());
                    router.world.storage(caller, migrations, call).await
                }
                Err(e) => Err(e),
            };
            from_ext(result)
        }
        Method::BlobsPut | Method::BlobsGet | Method::BlobsList | Method::BlobsDelete => {
            let call = match blob_call(method, params) {
                Ok(call) => call,
                Err(e) => return invalid(method, &e),
            };
            let result = match gate::admit_storage(manifest) {
                Ok(()) => router.blobs(&caller.extension, call).await,
                Err(e) => Err(e),
            };
            from_ext(result)
        }
        Method::GuestInit | Method::GuestHandle | Method::GuestOnEvent => None,
    }
}
