//! The two components on Wasmtime: the backend (both models) and model B's UI.

use std::time::Instant;

use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxView, WasiView};

pub mod backend {
    wasmtime::component::bindgen!({ world: "backend", path: "../wit" });
}
pub mod ui {
    wasmtime::component::bindgen!({ world: "ui", path: "../wit" });
}

pub use ui::vorn::ext_ui_poc::surface::{Frame, Input, KeyPress};

/// A store's state: WASI, and the most linear memory it asked for.
pub struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    pub peak: usize,
    /// Model B's calls back to its backend, answered by the host.
    pub answer: Box<dyn FnMut(&str, &str) -> Result<String, String> + Send>,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ResourceLimiter for Host {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _max: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.peak = self.peak.max(desired);
        Ok(desired <= 512 << 20)
    }
    fn table_growing(&mut self, _c: usize, _d: usize, _m: Option<usize>) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

impl ui::vorn::ext_ui_poc::surface::Host for Host {}

impl ui::UiImports for Host {
    fn call(&mut self, method: String, args: String) -> Result<String, String> {
        (self.answer)(&method, &args)
    }
}

pub fn engine() -> Engine {
    let mut cfg = Config::new();
    cfg.epoch_interruption(false);
    Engine::new(&cfg).expect("engine")
}

/// Compiles once, as an install would, and reports the time it took.
pub fn compile(engine: &Engine, path: &str) -> (Component, f64) {
    let t = Instant::now();
    let c = Component::from_file(engine, path).expect("component");
    let compiled = t.elapsed().as_secs_f64() * 1e3;
    let bytes = c.serialize().expect("serialize");
    let c = unsafe { Component::deserialize(engine, &bytes) }.expect("deserialize");
    (c, compiled)
}

fn store(engine: &Engine, fonts: bool) -> Store<Host> {
    let mut b = WasiCtx::builder();
    b.inherit_stderr();
    if fonts {
        // The UI component reads the system faces, read-only, and nothing else.
        b.preopened_dir(
            "/System/Library/Fonts",
            "/System/Library/Fonts",
            FsPerms::ReadOnly,
        )
        .expect("fonts");
    }
    let mut s = Store::new(
        engine,
        Host {
            wasi: b.build(),
            table: ResourceTable::new(),
            peak: 0,
            answer: Box::new(|_, _| Err("no backend".into())),
        },
    );
    s.limiter(|h| h);
    s
}

pub struct Backend {
    pub store: Store<Host>,
    pub api: backend::Backend,
}

impl Backend {
    pub fn new(engine: &Engine, c: &Component) -> Backend {
        let mut linker: Linker<Host> = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("wasi");
        let mut store = store(engine, false);
        let api = backend::Backend::instantiate(&mut store, c, &linker).expect("backend");
        Backend { store, api }
    }
    pub fn view(&mut self) -> String {
        self.api.call_view(&mut self.store).expect("view")
    }
    pub fn rows(&mut self, inventory: &str) -> String {
        self.api
            .call_rows(&mut self.store, inventory)
            .expect("rows")
    }
    pub fn plan(&mut self, ids: &[String]) -> Result<String, String> {
        self.api
            .call_remove_plan(&mut self.store, ids)
            .expect("plan")
    }
}

pub struct UiB {
    pub store: Store<Host>,
    pub api: ui::Ui,
}

impl UiB {
    pub fn new(engine: &Engine, c: &Component, cells: bool, scale: f32) -> UiB {
        let mut linker: Linker<Host> = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("wasi");
        ui::Ui::add_to_linker::<_, HasSelf<Host>>(&mut linker, |h| h).expect("imports");
        let mut store = store(engine, true);
        let api = ui::Ui::instantiate(&mut store, c, &linker).expect("ui");
        api.call_init(&mut store, cells, scale).expect("init");
        UiB { store, api }
    }
    pub fn data(&mut self, rows: &str) {
        self.api.call_data(&mut self.store, rows).expect("data");
    }
    pub fn frame(&mut self, w: u32, h: u32, inputs: &[Input]) -> Frame {
        self.api
            .call_frame(&mut self.store, w, h, inputs)
            .expect("frame")
    }
}
