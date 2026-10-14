//! Runtime-only Wasmtime for built-in extensions: precompiled components, a memory cap and a deadline per call, and the bridge back to vornd.

use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

mod tasks_world {
    wasmtime::component::bindgen!({ world: "tasks", path: "../wit" });
}
mod workflows_world {
    wasmtime::component::bindgen!({ world: "workflows", path: "../wit" });
}

/// How often the deadline clock ticks.
const TICK: Duration = Duration::from_millis(10);
/// A call's deadline, in ticks: five seconds.
const DEADLINE_TICKS: u64 = 500;
/// The most linear memory one instance may hold.
const MEMORY_CAP: usize = 64 << 20;

/// What a component asks of vornd, answered on the calling thread.
pub trait Bridge: Send {
    /// One named storage call, JSON in and out.
    fn db(&mut self, op: &str, args: &str) -> Result<String, String>;
    /// A run changed (JSON): tell viewers and write it.
    fn run_changed(&mut self, run: &str);
    /// One info line for vornd's log.
    fn log(&mut self, line: &str);
}

/// A bridge that answers nothing, for an instance between calls.
struct Idle;

impl Bridge for Idle {
    fn db(&mut self, op: &str, _args: &str) -> Result<String, String> {
        Err(format!("{op}: no call is in progress"))
    }
    fn run_changed(&mut self, _run: &str) {}
    fn log(&mut self, _line: &str) {}
}

/// Why a component could not be loaded or called.
#[derive(Debug)]
pub enum Error {
    /// The precompiled file does not fit this runtime.
    Load(String),
    /// The component's imports could not be linked or it would not start.
    Instantiate(String),
    /// The call trapped, ran past its deadline or its memory cap.
    Trap(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Load(e) => write!(f, "the component could not be loaded: {e}"),
            Error::Instantiate(e) => write!(f, "the component could not start: {e}"),
            Error::Trap(e) => write!(f, "the component's call failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Linear memory capped, and the most an instance asked for.
struct Limits {
    peak: usize,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _max: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.peak = self.peak.max(desired);
        Ok(desired <= MEMORY_CAP)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _max: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= 100_000)
    }
}

/// One instance's state.
pub struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: Limits,
    bridge: Box<dyn Bridge>,
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl tasks_world::vorn::ext_perf::bridge::Host for Ctx {
    fn db(&mut self, op: String, args: String) -> Result<String, String> {
        self.bridge.db(&op, &args)
    }
    fn run_changed(&mut self, run: String) {
        self.bridge.run_changed(&run);
    }
    fn log(&mut self, message: String) {
        self.bridge.log(&message);
    }
}

impl workflows_world::vorn::ext_perf::bridge::Host for Ctx {
    fn db(&mut self, op: String, args: String) -> Result<String, String> {
        self.bridge.db(&op, &args)
    }
    fn run_changed(&mut self, run: String) {
        self.bridge.run_changed(&run);
    }
    fn log(&mut self, message: String) {
        self.bridge.log(&message);
    }
}

/// The engine settings precompiled code and the runtime must share.
pub fn config() -> Config {
    let mut c = Config::new();
    c.epoch_interruption(true);
    c
}

/// One engine per process, with the clock that ends calls past their deadline.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let engine = Engine::new(&config()).expect("the default engine settings are valid");
        let ticking = engine.clone();
        std::thread::Builder::new()
            .name("ext-deadlines".into())
            .spawn(move || loop {
                std::thread::sleep(TICK);
                ticking.increment_epoch();
            })
            .expect("a thread for the deadline clock starts");
        engine
    })
}

/// Compiles a `.wasm` component for this machine, as the out-of-process compiler would.
#[cfg(feature = "compile")]
pub fn precompile(wasm: &[u8]) -> Result<Vec<u8>, Error> {
    engine()
        .precompile_component(wasm)
        .map_err(|e| Error::Load(e.to_string()))
}

fn load(bytes: &[u8]) -> Result<Component, Error> {
    // SAFETY: the bytes are a component precompiled by this crate's own engine settings from a file vornd ships.
    unsafe { Component::deserialize(engine(), bytes) }.map_err(|e| Error::Load(e.to_string()))
}

fn store(bridge: Box<dyn Bridge>) -> Store<Ctx> {
    let mut store = Store::new(
        engine(),
        Ctx {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            limits: Limits { peak: 0 },
            bridge,
        },
    );
    store.limiter(|c| &mut c.limits);
    store.set_epoch_deadline(DEADLINE_TICKS);
    store
}

fn trap(e: wasmtime::Error) -> Error {
    Error::Trap(format!("{e:#}"))
}

/// Tasks, loaded once and instantiated as often as needed.
pub struct TasksComponent(tasks_world::TasksPre<Ctx>);

impl TasksComponent {
    pub fn load(cwasm: &[u8]) -> Result<TasksComponent, Error> {
        let component = load(cwasm)?;
        let mut linker = Linker::new(engine());
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        tasks_world::Tasks::add_to_linker::<_, HasSelf<Ctx>>(&mut linker, |c| c)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        let pre = linker
            .instantiate_pre(&component)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        tasks_world::TasksPre::new(pre)
            .map(TasksComponent)
            .map_err(|e| Error::Instantiate(e.to_string()))
    }

    pub fn instantiate(&self) -> Result<Tasks, Error> {
        let mut store = store(Box::new(Idle));
        let api = self
            .0
            .instantiate(&mut store)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        Ok(Tasks { store, api })
    }
}

/// One Tasks instance: one call at a time.
pub struct Tasks {
    store: Store<Ctx>,
    api: tasks_world::Tasks,
}

impl Tasks {
    /// One `task:*` call over `bridge`: the answer as JSON (none for void) and whether the board changed.
    pub fn call(
        &mut self,
        bridge: Box<dyn Bridge>,
        method: &str,
        params: &str,
        now: &str,
        id: &str,
    ) -> Result<Result<(Option<String>, bool), String>, Error> {
        self.store.data_mut().bridge = bridge;
        self.store.set_epoch_deadline(DEADLINE_TICKS);
        let out = self
            .api
            .call_call(&mut self.store, method, params, now, id)
            .map_err(trap);
        self.store.data_mut().bridge = Box::new(Idle);
        out
    }

    /// The most linear memory this instance has held, in bytes.
    pub fn memory(&self) -> usize {
        self.store.data().limits.peak
    }
}

/// The workflow step loop, loaded once; each run walks in an instance of its own.
pub struct WorkflowsComponent(workflows_world::WorkflowsPre<Ctx>);

/// How a walk ended: `success`, `error` or `parked`, and the steps it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: String,
    pub steps: u32,
}

impl WorkflowsComponent {
    pub fn load(cwasm: &[u8]) -> Result<WorkflowsComponent, Error> {
        let component = load(cwasm)?;
        let mut linker = Linker::new(engine());
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        workflows_world::Workflows::add_to_linker::<_, HasSelf<Ctx>>(&mut linker, |c| c)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        let pre = linker
            .instantiate_pre(&component)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        workflows_world::WorkflowsPre::new(pre)
            .map(WorkflowsComponent)
            .map_err(|e| Error::Instantiate(e.to_string()))
    }

    pub fn instantiate(&self, bridge: Box<dyn Bridge>) -> Result<Workflows, Error> {
        let mut store = store(bridge);
        let api = self
            .0
            .instantiate(&mut store)
            .map_err(|e| Error::Instantiate(e.to_string()))?;
        Ok(Workflows { store, api })
    }
}

/// One walk's instance.
pub struct Workflows {
    store: Store<Ctx>,
    api: workflows_world::Workflows,
}

fn outcome(o: workflows_world::Outcome) -> Outcome {
    Outcome {
        status: o.status,
        steps: o.steps,
    }
}

impl Workflows {
    /// Starts a run of `workflow` (stored JSON) and walks it until it ends or parks.
    pub fn start(
        &mut self,
        workflow: &str,
        run_id: &str,
    ) -> Result<Result<Outcome, String>, Error> {
        self.store.set_epoch_deadline(DEADLINE_TICKS);
        let out = self
            .api
            .call_start(&mut self.store, workflow, run_id)
            .map_err(trap)?;
        Ok(out.map(outcome))
    }

    /// Answers a parked gate on the stored run (JSON) and walks on.
    pub fn resume(
        &mut self,
        workflow: &str,
        run: &str,
        node_id: &str,
        approve: bool,
    ) -> Result<Result<Outcome, String>, Error> {
        self.store.set_epoch_deadline(DEADLINE_TICKS);
        let out = self
            .api
            .call_resume(&mut self.store, workflow, run, node_id, approve)
            .map_err(trap)?;
        Ok(out.map(outcome))
    }

    /// The most linear memory this instance has held, in bytes.
    pub fn memory(&self) -> usize {
        self.store.data().limits.peak
    }
}
