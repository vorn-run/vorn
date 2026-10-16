//! Services as sandboxed Wasm components: the same manifest, events, gate
//! and lifecycle as a program, with `[service] component = "x.wasm"` in place
//! of `command`.
//!
//! A component targets the `extension` world of `wit/extension.wit` (WASI
//! preview 2, with no files, environment, network or clock beyond the
//! monotonic one). Each runs on a thread of its own, so a slow one holds up
//! nobody else's calls, and answers what it imports through the router's
//! [`answer`](crate::router) path: the same permission checks a program's
//! JSON-RPC requests get. A call past its deadline or a trap ends the
//! instance, never vornd; the lifecycle counts it as a crash and starts a
//! fresh one on the next use, within the crash budget.
//!
//! vornd links Wasmtime's runtime only, so it loads the `.cwasm` precompiled
//! beside the component; the `wasm-compile` feature adds the compiler, which
//! compiles a component with no `.cwasm` and keeps one beside it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, OnceLock, Weak};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::api::types::{CallCtx, Event, InitCtx, RowOp};
use crate::api::{ExtError, Method};
use crate::child::{CallError, ErrorData, Incoming};
use crate::gate::Caller;
use crate::manifest::Manifest;
use crate::router::Router;

mod bindings {
    wasmtime::component::bindgen!({ world: "extension", path: "wit/extension.wit" });
}

use bindings::vorn::extension::types as wit;

/// How often the deadline clock ticks while a component runs.
const TICK: Duration = Duration::from_millis(10);
/// The most linear memory one instance may hold.
const MEMORY_CAP: usize = 64 << 20;

/// The engine settings a `.cwasm` is compiled and loaded with; they must agree.
fn config() -> Config {
    let mut c = Config::new();
    c.epoch_interruption(true);
    c
}

/// One engine per process, and the clock that ends calls past their deadline.
struct Runtime {
    engine: Engine,
    busy: Arc<AtomicUsize>,
    ticker: thread::Thread,
}

fn runtime() -> Result<&'static Runtime, String> {
    static RT: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        let engine = Engine::new(&config()).map_err(|e| e.to_string())?;
        let busy = Arc::new(AtomicUsize::new(0));
        let (e, b) = (engine.clone(), Arc::clone(&busy));
        // Ticks only while a component runs, so an idle vornd is never woken.
        let ticker = thread::Builder::new()
            .name("ext-wasm-clock".into())
            .spawn(move || loop {
                if b.load(Ordering::Acquire) == 0 {
                    thread::park();
                    continue;
                }
                thread::sleep(TICK);
                e.increment_epoch();
            })
            .map_err(|e| e.to_string())?
            .thread()
            .clone();
        Ok(Runtime {
            engine,
            busy,
            ticker,
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}

impl Runtime {
    fn enter(&self) -> Busy<'_> {
        if self.busy.fetch_add(1, Ordering::AcqRel) == 0 {
            self.ticker.unpark();
        }
        Busy(&self.busy)
    }
}

struct Busy<'a>(&'a AtomicUsize);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A deadline in clock ticks, at least one.
fn ticks(deadline: Duration) -> u64 {
    u64::try_from(deadline.as_millis() / TICK.as_millis())
        .unwrap_or(u64::from(u32::MAX))
        .max(1)
}

/// Where the precompiled form of `wasm` is kept.
pub fn cwasm_of(wasm: &Path) -> PathBuf {
    wasm.with_extension("cwasm")
}

/// Compiles `wasm` into the `.cwasm` vornd loads.
#[cfg(feature = "wasm-compile")]
pub fn precompile(wasm: &Path) -> Result<PathBuf, String> {
    let rt = runtime()?;
    let bytes = std::fs::read(wasm).map_err(|e| format!("{}: {e}", wasm.display()))?;
    let compiled = rt
        .engine
        .precompile_component(&bytes)
        .map_err(|e| format!("{} does not compile: {e:#}", wasm.display()))?;
    let out = cwasm_of(wasm);
    std::fs::write(&out, compiled).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

fn load(wasm: &Path) -> Result<Component, String> {
    let rt = runtime()?;
    let cwasm = cwasm_of(wasm);
    #[cfg(feature = "wasm-compile")]
    if !cwasm.is_file() {
        precompile(wasm)?;
    }
    // SAFETY: a .cwasm is machine code Wasmtime maps as it is. vornd loads only
    // the one beside a package's component, compiled with `config()`; Wasmtime
    // still refuses one built by another version or for another CPU.
    unsafe { Component::deserialize_file(&rt.engine, &cwasm) }.map_err(|e| {
        format!(
            "{} is not precompiled for this Vorn ({e:#}); compile it to {}",
            wasm.display(),
            cwasm.display()
        )
    })
}

/// What an instance's store holds.
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: Limits,
    router: Weak<Router>,
    manifest: Arc<Manifest>,
    caller: Caller,
    tokio: tokio::runtime::Handle,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// The memory cap, and the most linear memory the instance has held.
struct Limits {
    peak: Arc<AtomicUsize>,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _: usize,
        desired: usize,
        _: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let ok = desired <= MEMORY_CAP;
        if ok {
            self.peak.fetch_max(desired, Ordering::Relaxed);
        }
        Ok(ok)
    }

    fn table_growing(
        &mut self,
        _: usize,
        desired: usize,
        _: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 100_000)
    }
}

impl State {
    /// One import, answered as the router answers a program's request of the same method.
    fn ask(&self, method: Method, params: Value) -> Result<Value, wit::ExtError> {
        let Some(router) = self.router.upgrade() else {
            return Err(wit_err(ExtError::new("stopped", "Vorn is stopping")));
        };
        let incoming = Incoming {
            id: None,
            method: method.name().to_owned(),
            params,
        };
        let answered = self.tokio.block_on(crate::router::answer(
            &router,
            &self.manifest,
            &self.caller,
            incoming,
        ));
        match answered {
            None => Ok(Value::Null),
            Some(Ok(v)) => Ok(v),
            Some(Err((_, message, data))) => {
                let code = data["code"].as_str().unwrap_or("failed").to_owned();
                Err(wit::ExtError { code, message })
            }
        }
    }
}

fn wit_err(e: ExtError) -> wit::ExtError {
    wit::ExtError {
        code: e.code,
        message: e.message,
    }
}

fn parsed(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Null)
}

impl wit::Host for State {}

impl bindings::vorn::extension::host::Host for State {
    fn call(&mut self, method: String, args: String) -> Result<String, wit::ExtError> {
        self.ask(
            Method::HostCall,
            json!({ "method": method, "args": parsed(&args) }),
        )
        .map(|v| v.to_string())
    }

    fn publish(&mut self, collection: String, ops: Vec<wit::RowOp>) {
        let Some(router) = self.router.upgrade() else {
            return;
        };
        let ops = ops
            .into_iter()
            .map(|op| match op {
                wit::RowOp::Upsert(row) => RowOp::Upsert(parsed(&row)),
                wit::RowOp::Delete(id) => RowOp::Delete(id),
                wit::RowOp::Clear => RowOp::Clear,
            })
            .collect();
        router.publish(&self.caller, &collection, ops);
    }

    fn log(&mut self, level: wit::Level, message: String) {
        let level = match level {
            wit::Level::Debug => "debug",
            wit::Level::Info => "info",
            wit::Level::Warn => "warn",
            wit::Level::Error => "error",
        };
        let _ = self.ask(
            Method::HostLog,
            json!({ "level": level, "message": message }),
        );
    }
}

impl bindings::vorn::extension::net::Host for State {
    fn fetch(
        &mut self,
        req: wit::Request,
        connection: Option<String>,
    ) -> Result<wit::Response, wit::ExtError> {
        let headers: Vec<Value> = req
            .headers
            .iter()
            .map(|h| json!({ "name": h.name, "value": h.value }))
            .collect();
        let params = json!({
            "req": { "method": req.method, "url": req.url, "headers": headers, "body": req.body },
            "connection": connection,
        });
        let v = self.ask(Method::NetFetch, params)?;
        let headers = v["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| wit::Header {
                name: h["name"].as_str().unwrap_or_default().to_owned(),
                value: h["value"].as_str().unwrap_or_default().to_owned(),
            })
            .collect();
        Ok(wit::Response {
            status: v["status"]
                .as_u64()
                .and_then(|s| u16::try_from(s).ok())
                .unwrap_or(0),
            headers,
            body: v["body"].as_str().unwrap_or_default().to_owned(),
        })
    }
}

impl bindings::vorn::extension::storage::Host for State {
    fn prefix(&mut self) -> Result<String, wit::ExtError> {
        let v = self.ask(Method::StoragePrefix, json!({}))?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    fn query(&mut self, sql: String, params: Vec<String>) -> Result<Vec<String>, wit::ExtError> {
        let params: Vec<Value> = params.iter().map(|p| parsed(p)).collect();
        let v = self.ask(
            Method::StorageQuery,
            json!({ "sql": sql, "params": params }),
        )?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string)
            .collect())
    }

    fn exec(&mut self, sql: String, params: Vec<String>) -> Result<u64, wit::ExtError> {
        let params: Vec<Value> = params.iter().map(|p| parsed(p)).collect();
        let v = self.ask(Method::StorageExec, json!({ "sql": sql, "params": params }))?;
        Ok(v.as_u64().unwrap_or(0))
    }

    fn get(&mut self, key: String) -> Result<Option<String>, wit::ExtError> {
        let v = self.ask(Method::StorageGet, json!({ "key": key }))?;
        Ok((!v.is_null()).then(|| v.to_string()))
    }

    fn set(&mut self, key: String, value: String) -> Result<(), wit::ExtError> {
        self.ask(
            Method::StorageSet,
            json!({ "key": key, "value": parsed(&value) }),
        )
        .map(drop)
    }
}

enum Job {
    Handle {
        method: String,
        args: Value,
        ctx: CallCtx,
        deadline: Duration,
        reply: oneshot::Sender<Result<Value, CallError>>,
    },
    /// `done` is dropped once the guest has the batch behind it, so a busy
    /// guest holds its queue back as a full pipe holds back a program.
    Events {
        batch: Vec<Event>,
        done: oneshot::Sender<()>,
    },
}

/// A component service: its thread, and what it was last seen to hold.
pub struct Service {
    name: String,
    jobs: std::sync::Mutex<Option<std_mpsc::Sender<Job>>>,
    ended: Arc<AtomicBool>,
    memory: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("wasm::Service")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn transport(e: impl std::fmt::Display) -> CallError {
    CallError::Transport(e.to_string())
}

/// What a trap or a passed deadline reads as.
fn stopped(name: &str, trap: &wasmtime::Error) -> CallError {
    match trap.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => {
            CallError::Timeout(format!("{name} ran past its deadline and was stopped"))
        }
        // The trap alone: its wasm backtrace names nothing a reader can use.
        Some(t) => CallError::Transport(format!("{name} stopped: {t}")),
        None => CallError::Transport(format!("{name} stopped: {trap:#}")),
    }
}

/// Everything a component's thread is started with.
pub(crate) struct Start {
    pub wasm: PathBuf,
    pub name: String,
    pub init: InitCtx,
    pub start_timeout: Duration,
    pub router: Weak<Router>,
    pub manifest: Arc<Manifest>,
    pub caller: Caller,
}

impl Service {
    /// Loads and instantiates the component and answers its `guest.init`.
    pub(crate) async fn start(s: Start) -> Result<Service, String> {
        let component = load(&s.wasm)?;
        let (jobs, rx) = std_mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
        let ended = Arc::new(AtomicBool::new(false));
        let memory = Arc::new(AtomicUsize::new(0));
        let state = State {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits: Limits {
                peak: Arc::clone(&memory),
            },
            router: s.router,
            manifest: s.manifest,
            caller: s.caller,
            tokio: tokio::runtime::Handle::current(),
        };
        let thread_name = format!("ext-wasm {}", s.init.id);
        let (name, init, start_deadline) = (s.name.clone(), s.init, s.start_timeout);
        let flag = Arc::clone(&ended);
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                run(
                    &name,
                    component,
                    state,
                    &init,
                    start_deadline,
                    Ends {
                        ready: ready_tx,
                        jobs: &rx,
                        ended: &flag,
                    },
                );
                flag.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        let ready = tokio::time::timeout(s.start_timeout, ready_rx).await;
        match ready {
            Ok(Ok(Ok(()))) => Ok(Service {
                name: s.name,
                jobs: std::sync::Mutex::new(Some(jobs)),
                ended,
                memory,
            }),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(format!("{} ended as it started", s.name)),
            Err(_) => Err(format!("{} did not start in time", s.name)),
        }
    }

    fn send(&self, job: Job) -> Result<(), CallError> {
        let jobs = self
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        jobs.as_ref()
            .ok_or_else(|| transport(format!("{} was stopped", self.name)))?
            .send(job)
            .map_err(|_| transport(format!("{} is not running", self.name)))
    }

    /// `guest.handle`, held to `deadline`.
    pub async fn handle(
        &self,
        method: String,
        args: Value,
        ctx: CallCtx,
        deadline: Duration,
    ) -> Result<Value, CallError> {
        let (reply, answer) = oneshot::channel();
        self.send(Job::Handle {
            method,
            args,
            ctx,
            deadline,
            reply,
        })?;
        answer.await.unwrap_or_else(|_| {
            Err(transport(format!(
                "{} stopped before it answered",
                self.name
            )))
        })
    }

    /// `guest.on-event`; nothing answers it, but it resolves only once the guest is done with it.
    pub async fn on_event(&self, batch: Vec<Event>) -> Result<(), CallError> {
        let (done, handled) = oneshot::channel();
        self.send(Job::Events { batch, done })?;
        let _ = handled.await;
        Ok(())
    }

    /// The most linear memory its instance has held, in bytes.
    pub fn memory(&self) -> usize {
        self.memory.load(Ordering::Relaxed)
    }
}

impl crate::lifecycle::Running for Service {
    fn name(&self) -> &str {
        &self.name
    }

    fn exited(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    fn close(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        // Its thread ends once the queue is gone and the job it runs, if any, returns.
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Box::pin(async {})
    }
}

/// What the component's thread talks to: who waits for it to start, its
/// jobs, and the flag that tells the lifecycle it ended.
struct Ends<'a> {
    ready: oneshot::Sender<Result<(), String>>,
    jobs: &'a std_mpsc::Receiver<Job>,
    ended: &'a AtomicBool,
}

/// The component's thread: instantiate, init, then calls and events until the queue goes or it traps.
fn run(
    name: &str,
    component: Component,
    state: State,
    init: &InitCtx,
    start_deadline: Duration,
    ends: Ends<'_>,
) {
    let Ends { ready, jobs, ended } = ends;
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // WASI's own blocking calls run on vornd's runtime.
    let tokio = state.tokio.clone();
    let _in_tokio = tokio.enter();
    let mut linker = Linker::<State>::new(&rt.engine);
    let linked = wasmtime_wasi::p2::add_to_linker_sync(&mut linker).and_then(|()| {
        bindings::Extension::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
    });
    if let Err(e) = linked {
        let _ = ready.send(Err(format!("{name}: {e:#}")));
        return;
    }
    let mut store = Store::new(&rt.engine, state);
    store.limiter(|s| &mut s.limits);
    store.set_epoch_deadline(ticks(start_deadline));
    let started = {
        let _busy = rt.enter();
        bindings::Extension::instantiate(&mut store, &component, &linker).and_then(|api| {
            let ctx = wit::InitCtx {
                id: init.id.clone(),
                version: init.version.clone(),
                vorn: init.vorn.clone(),
            };
            api.vorn_extension_guest()
                .call_init(&mut store, &ctx)
                .map(|r| (api, r))
        })
    };
    let api = match started {
        Ok((api, Ok(()))) => api,
        Ok((_, Err(e))) => {
            let _ = ready.send(Err(format!("{name} did not start: {}", e.message)));
            return;
        }
        Err(e) => {
            let _ = ready.send(Err(format!("{name} did not start: {e:#}")));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let guest = api.vorn_extension_guest();
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Handle {
                method,
                args,
                ctx,
                deadline,
                reply,
            } => {
                store.set_epoch_deadline(ticks(deadline));
                let wctx = wit::CallCtx {
                    caller: ctx.caller,
                    project: ctx.project,
                };
                let answer = {
                    let _busy = rt.enter();
                    guest.call_handle(&mut store, &method, &args.to_string(), &wctx)
                };
                match answer {
                    Ok(Ok(json)) => {
                        let _ = reply.send(serde_json::from_str(&json).map_err(|e| {
                            transport(format!("{name} answered {method} with bad JSON: {e}"))
                        }));
                    }
                    Ok(Err(e)) => {
                        let _ = reply.send(Err(CallError::Answered {
                            code: -32000,
                            message: e.message,
                            data: ErrorData {
                                code: Some(e.code),
                                ..ErrorData::default()
                            },
                        }));
                    }
                    Err(trap) => {
                        // Ended before the answer, so the next call starts a fresh instance.
                        ended.store(true, Ordering::Release);
                        let _ = reply.send(Err(stopped(name, &trap)));
                        return;
                    }
                }
            }
            Job::Events { batch, done } => {
                store.set_epoch_deadline(ticks(start_deadline));
                let batch: Vec<wit::Event> = batch
                    .into_iter()
                    .map(|e| wit::Event {
                        name: e.name,
                        data: e.data.to_string(),
                    })
                    .collect();
                let handled = {
                    let _busy = rt.enter();
                    guest.call_on_event(&mut store, &batch)
                };
                if handled.is_err() {
                    ended.store(true, Ordering::Release);
                }
                drop(done);
                if let Err(trap) = handled {
                    tracing::warn!("[extensions] {}", stopped(name, &trap));
                    return;
                }
            }
        }
    }
}
