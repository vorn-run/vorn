//! The router: calls and events in, to programs that speak the extension
//! API, and what those programs ask back, through the gate.
//!
//! An action the manifest gives a command of its own runs one-shot: a
//! process per call, sent one `guest.handle`. Anything else goes to the
//! service, started by the [`Lifecycle`] on its first call or event. Each
//! program's requests are answered on tasks of their own, so a program
//! waiting on the network holds up nobody else.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, error, info, warn};

use crate::api::types::{CallCtx, Event, InitCtx, Level, RowOp};
use crate::api::{params, ExtError, Method, Side};
use crate::child::{CallError, Child, Inbox, Incoming, Launch, Writer};
use crate::events::{self, Bus, Queue, Tally};
use crate::gate::{self, Caller, StorageCall, World};
use crate::lifecycle::{Lifecycle, Running, ServiceKey};
use crate::manifest::{Manifest, Run};
use crate::pack::{InstalledPack, PackStore, Stamp};
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
}

impl RouterSettings {
    pub fn new(base_env: Vec<(String, String)>, version: impl Into<String>) -> RouterSettings {
        RouterSettings {
            base_env,
            version: version.into(),
            call_timeout: Duration::from_secs(60),
            start_timeout: Duration::from_secs(15),
        }
    }
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

    async fn on_event(&self, params: params::GuestOnEvent) -> Result<(), CallError> {
        match self {
            Program::Child(c) => c.notify(Method::GuestOnEvent.name(), json!(params)).await,
            #[cfg(feature = "wasm")]
            Program::Component(c) => c.on_event(params.batch).await,
        }
    }

    /// The process id of a program; `None` for a component, which runs in vornd.
    pub fn pid(&self) -> Option<u32> {
        match self {
            Program::Child(c) => c.pid(),
            #[cfg(feature = "wasm")]
            Program::Component(_) => None,
        }
    }

    /// The linear memory a component's instance holds; `None` for a program.
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
        Arc::new_cyclic(|me| Router {
            store,
            lifecycle,
            world,
            settings,
            rows: Mutex::default(),
            packs: Mutex::default(),
            bus,
            queues: Mutex::default(),
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
                // A program that let a call time out may be wedged; the next call starts it fresh.
                warn!("[extensions] {id} timed out; stopping it");
                self.lifecycle.stop(&key).await;
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
        let caller = Caller {
            extension: pack.id.clone(),
            project: None,
        };
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
        };
        if let Some(component) = &service.component {
            return self.start_component(pack, component, ctx).await;
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

    #[cfg(feature = "wasm")]
    async fn start_component(
        &self,
        pack: &InstalledPack,
        component: &str,
        init: InitCtx,
    ) -> Result<Program, String> {
        let start = crate::wasm::Start {
            wasm: PathBuf::from(&pack.path).join(component),
            name: format!("[extensions] {}", pack.name()),
            init,
            start_timeout: self.settings.start_timeout,
            router: self.me.clone(),
            manifest: Arc::new(pack.manifest.clone()),
            caller: Caller {
                extension: pack.id.clone(),
                project: None,
            },
        };
        let service = crate::wasm::Service::start(start).await?;
        info!("[extensions] {} started as a component", pack.name());
        Ok(Program::Component(service))
    }

    #[cfg(not(feature = "wasm"))]
    #[allow(clippy::unused_async)]
    async fn start_component(
        &self,
        pack: &InstalledPack,
        _component: &str,
        _init: InitCtx,
    ) -> Result<Program, String> {
        Err(format!(
            "{} is a Wasm component, and this Vorn runs none",
            pack.name()
        ))
    }

    /// The running service of extension `id`, without starting it.
    pub fn running(&self, id: &str) -> Option<crate::lifecycle::Lease<Program>> {
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
        let params = params::GuestOnEvent {
            batch: batch.iter().map(|e| Event::clone(e)).collect(),
        };
        if let Err(err) = lease.on_event(params).await {
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
        _ => StorageCall::Prefix,
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

/// One request a program made; `None` for one that is not answered.
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
        | Method::StorageSet => {
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
        Method::GuestInit | Method::GuestHandle | Method::GuestOnEvent => None,
    }
}
