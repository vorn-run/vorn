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
use crate::gate::{self, Caller, World};
use crate::lifecycle::{Lifecycle, Running, ServiceKey};
use crate::manifest::{Manifest, Run};
use crate::pack::{InstalledPack, PackStore};

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

/// How many of one program's own requests are answered at once.
const REQUESTS_AT_ONCE: usize = 16;
/// Rows a collection keeps; past this an upsert of a new row is dropped.
const MAX_ROWS: usize = 10_000;
/// The JSON-RPC codes for a method nobody answers and params that do not read.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// A service speaking the extension API.
#[derive(Debug)]
pub struct Program {
    child: Child,
}

impl Running for Program {
    fn child(&self) -> &Child {
        &self.child
    }
}

type Rows = HashMap<(String, String), Map<String, Value>>;

/// Calls and events in, rows out.
pub struct Router {
    store: PackStore,
    lifecycle: Arc<Lifecycle>,
    world: Arc<dyn World>,
    settings: RouterSettings,
    rows: Mutex<Rows>,
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
    pub fn new(
        store: PackStore,
        lifecycle: Arc<Lifecycle>,
        world: Arc<dyn World>,
        settings: RouterSettings,
    ) -> Arc<Router> {
        Arc::new_cyclic(|me| Router {
            store,
            lifecycle,
            world,
            settings,
            rows: Mutex::default(),
            me: me.clone(),
        })
    }

    pub fn store(&self) -> &PackStore {
        &self.store
    }

    pub fn lifecycle(&self) -> &Arc<Lifecycle> {
        &self.lifecycle
    }

    fn program_pack(&self, id: &str) -> Result<InstalledPack, ExtError> {
        let pack = self
            .store
            .inspect(id)
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
        let answer = lease
            .child
            .request(
                Method::GuestHandle.name(),
                json!(params),
                self.settings.call_timeout,
            )
            .await;
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

    /// Delivers `event` to every service that declares it, starting the
    /// ones that are not running; answers how many it reached.
    pub async fn emit(&self, event: &str, data: Value) -> usize {
        let subscribers: Vec<InstalledPack> = self
            .store
            .list()
            .into_iter()
            .filter(|p| p.manifest.speaks_api())
            .filter(|p| {
                p.manifest
                    .service
                    .as_ref()
                    .is_some_and(|s| s.events.iter().any(|e| e == event))
            })
            .collect();
        let batch = params::GuestOnEvent {
            batch: vec![Event {
                name: event.to_owned(),
                data,
            }],
        };
        let mut deliveries = Vec::with_capacity(subscribers.len());
        for pack in subscribers {
            let Some(me) = self.me.upgrade() else {
                return 0;
            };
            let batch = json!(batch);
            deliveries.push(tokio::spawn(async move {
                let key = ServiceKey::new(pack.id.clone(), "");
                let lease = me
                    .lifecycle
                    .get_or_start(&key, || me.start_service(&pack))
                    .await;
                match lease {
                    Ok(lease) => lease
                        .child
                        .notify(Method::GuestOnEvent.name(), batch)
                        .await
                        .is_ok(),
                    Err(err) => {
                        warn!("[extensions] {} missed an event: {err}", pack.id);
                        false
                    }
                }
            }));
        }
        let mut reached = 0;
        for delivery in deliveries {
            reached += usize::from(delivery.await.unwrap_or(false));
        }
        reached
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
    }

    fn launch(&self, pack: &InstalledPack, argv: &[String]) -> Result<Launch, String> {
        let (program, args) = pack
            .resolve(argv)
            .ok_or_else(|| format!("{} names no program to run", pack.name()))?;
        let mut env = self.settings.base_env.clone();
        env.retain(|(k, _)| k != "VORN_EXTENSION_ID");
        env.push(("VORN_EXTENSION_ID".into(), pack.id.clone()));
        Ok(Launch {
            program,
            args,
            cwd: PathBuf::from(&pack.path),
            env,
        })
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
        let argv = pack
            .manifest
            .service
            .as_ref()
            .map(|s| s.command.clone())
            .ok_or_else(|| format!("{} has no service", pack.name()))?;
        let child = self.spawn_program(pack, &argv)?;
        let init = params::GuestInit {
            ctx: InitCtx {
                id: pack.id.clone(),
                version: pack.version.clone(),
                vorn: self.settings.version.clone(),
            },
        };
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
        Ok(Program { child })
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

    fn publish(&self, caller: &Caller, collection: &str, ops: Vec<RowOp>) {
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
                        kept.insert(id, row);
                    }
                    RowOp::Delete(id) => {
                        kept.shift_remove(&id);
                    }
                    RowOp::Clear => kept.clear(),
                }
            }
        }
        self.world.published(caller, collection);
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
async fn answer(
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
        Method::GuestInit | Method::GuestHandle | Method::GuestOnEvent => None,
    }
}
