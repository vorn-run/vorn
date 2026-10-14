//! The safe tier: a precompiled component on Wasmtime without its compiler.
//!
//! vornd links only Wasmtime's runtime and component model, so it can load a
//! `.cwasm` and cannot compile one; `vorn-extc` compiles with [`config`], the
//! same settings, because a `.cwasm` loads only into an engine configured as
//! the one that built it. The guest imports nothing but the WIT's `host` and
//! `net`, each answered through the extension's [`Gate`]; there is no WASI.
//!
//! Each extension keeps one instance between calls. A call has a deadline
//! (epoch interruption) and the store a memory cap; a trap ends the call and
//! the instance, never vornd, and the next call starts a fresh one.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store};

use crate::bridge::{FetchRequest, Gate, World};
use crate::manifest::Manifest;
use crate::quick::CallCtx;
use crate::ExtError;

mod bindings {
    wasmtime::component::bindgen!({ world: "extension", path: "../../ext/wit" });
}

use bindings::vorn::extension::types as wit;

/// How often the deadline clock ticks while a call runs.
const TICK: Duration = Duration::from_millis(10);

/// The engine settings both vornd and `vorn-extc` use. Epoch interruption is
/// compiled into the code, so the two must agree on it.
pub fn config() -> Config {
    let mut c = Config::new();
    c.epoch_interruption(true);
    c
}

/// One engine per process, and the clock that ends calls past their deadline.
pub struct Runtime {
    engine: Engine,
    busy: Arc<AtomicUsize>,
    ticker: OnceLock<thread::Thread>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

impl Runtime {
    pub fn new() -> Result<Arc<Runtime>, ExtError> {
        let engine = Engine::new(&config()).map_err(|e| ExtError::new("wasm", e.to_string()))?;
        let rt = Arc::new(Runtime {
            engine,
            busy: Arc::new(AtomicUsize::new(0)),
            ticker: OnceLock::new(),
        });
        // Ticks only while a call runs, so an idle vornd is never woken.
        let (engine, busy) = (rt.engine.clone(), Arc::clone(&rt.busy));
        let t = thread::Builder::new()
            .name("ext-deadline".into())
            .spawn(move || loop {
                if busy.load(Ordering::Acquire) == 0 {
                    thread::park();
                    continue;
                }
                thread::sleep(TICK);
                engine.increment_epoch();
            })
            .map_err(|e| ExtError::new("wasm", e.to_string()))?;
        let _ = rt.ticker.set(t.thread().clone());
        Ok(rt)
    }

    /// Loads a component `vorn-extc` precompiled.
    pub fn load(&self, path: &Path) -> Result<Component, ExtError> {
        // SAFETY: a .cwasm is machine code Wasmtime trusts as it maps it. vornd
        // loads only what vorn-extc compiled on this machine with `config()`;
        // Wasmtime still refuses a file built by another version or for another CPU.
        unsafe { Component::deserialize_file(&self.engine, path) }
            .map_err(|e| ExtError::new("wasm", format!("cannot load {}: {e:#}", path.display())))
    }

    fn enter(&self) -> Busy<'_> {
        if self.busy.fetch_add(1, Ordering::AcqRel) == 0 {
            if let Some(t) = self.ticker.get() {
                t.unpark();
            }
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

/// What a store holds: the extension's manifest and world, and its limits.
struct State {
    manifest: Arc<Manifest>,
    world: Arc<dyn World>,
    project: String,
    limits: Limits,
}

/// The memory cap, and the most linear memory the instance asked for.
struct Limits {
    cap: usize,
    peak: usize,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        _: usize,
        desired: usize,
        _: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let ok = desired <= self.cap;
        if ok {
            self.peak = self.peak.max(desired);
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
    fn gate(&self) -> Gate<'_> {
        Gate {
            manifest: &self.manifest,
            world: self.world.as_ref(),
            project: &self.project,
        }
    }
}

fn wit_err(e: ExtError) -> wit::ExtError {
    wit::ExtError {
        code: e.code,
        message: e.message,
    }
}

impl wit::Host for State {}

impl bindings::vorn::extension::host::Host for State {
    fn call(&mut self, method: String, args: String) -> Result<String, wit::ExtError> {
        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
        self.gate()
            .call(&method, &args)
            .map(|v| v.to_string())
            .map_err(wit_err)
    }

    fn log(&mut self, _level: wit::Level, message: String) {
        eprintln!("[ext {}] {message}", self.manifest.id);
    }
}

impl bindings::vorn::extension::net::Host for State {
    fn fetch(
        &mut self,
        req: wit::Request,
        _connection: Option<String>,
    ) -> Result<wit::Response, wit::ExtError> {
        let req = FetchRequest {
            method: req.method,
            url: req.url,
            headers: req.headers.into_iter().map(|h| (h.name, h.value)).collect(),
            body: req.body,
        };
        let res = self.gate().fetch(&req).map_err(wit_err)?;
        Ok(wit::Response {
            status: res.status,
            headers: res
                .headers
                .into_iter()
                .map(|(name, value)| wit::Header { name, value })
                .collect(),
            body: res.body,
        })
    }
}

/// Timings of the last start and call, for the Extensions page and the spike's numbers.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    pub instantiate_us: f64,
    pub call_us: f64,
    /// Linear memory the instance holds, in bytes.
    pub memory: usize,
}

/// One safe extension: its component and, between calls, its instance.
pub struct Instance {
    runtime: Arc<Runtime>,
    component: Component,
    linker: Linker<State>,
    manifest: Arc<Manifest>,
    world: Arc<dyn World>,
    live: Option<(Store<State>, bindings::Extension)>,
    pub timings: Timings,
}

/// How long one call may run before it traps.
pub const CALL_DEADLINE: Duration = Duration::from_secs(10);

impl Instance {
    pub fn new(
        runtime: Arc<Runtime>,
        component: Component,
        manifest: Arc<Manifest>,
        world: Arc<dyn World>,
    ) -> Result<Instance, ExtError> {
        let mut linker = Linker::new(&runtime.engine);
        bindings::Extension::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
            .map_err(|e| ExtError::new("wasm", e.to_string()))?;
        Ok(Instance {
            runtime,
            component,
            linker,
            manifest,
            world,
            live: None,
            timings: Timings::default(),
        })
    }

    fn instantiate(&mut self) -> Result<(), ExtError> {
        let t = Instant::now();
        let limits = Limits {
            cap: self.manifest.memory_cap(),
            peak: 0,
        };
        let mut store = Store::new(
            &self.runtime.engine,
            State {
                manifest: Arc::clone(&self.manifest),
                world: Arc::clone(&self.world),
                project: String::new(),
                limits,
            },
        );
        store.limiter(|s| &mut s.limits);
        // Start-up code is held to a call's deadline too; a delta, as `current + delta` must not overflow.
        store.set_epoch_deadline(ticks(CALL_DEADLINE));
        let api = bindings::Extension::instantiate(&mut store, &self.component, &self.linker)
            .map_err(|e| ExtError::new("wasm", format!("cannot start the component: {e:#}")))?;
        self.timings.instantiate_us = t.elapsed().as_secs_f64() * 1e6;
        self.live = Some((store, api));
        Ok(())
    }

    /// `guest.handle`, with the deadline and a fresh instance after a trap.
    pub fn call(
        &mut self,
        method: &str,
        args: &Value,
        ctx: &CallCtx,
        deadline: Duration,
    ) -> Result<Value, ExtError> {
        if self.live.is_none() {
            self.instantiate()?;
        }
        let (store, api) = self.live.as_mut().expect("instantiated above");
        store.data_mut().project.clone_from(&ctx.project);
        store.set_epoch_deadline(ticks(deadline));
        let wctx = wit::CallCtx {
            project: ctx.project.clone(),
            caller: ctx.caller.clone(),
        };
        let t = Instant::now();
        let answer = {
            let _busy = self.runtime.enter();
            api.vorn_extension_guest()
                .call_handle(&mut *store, method, &args.to_string(), &wctx)
        };
        self.timings.call_us = t.elapsed().as_secs_f64() * 1e6;
        self.timings.memory = store.data().limits.peak;
        match answer {
            Ok(Ok(json)) => serde_json::from_str(&json).map_err(|e| {
                ExtError::new(
                    "bad-output",
                    format!("the component answered with bad JSON: {e}"),
                )
            }),
            Ok(Err(e)) => Err(ExtError::new(&e.code, e.message)),
            Err(trap) => {
                // The instance may be mid-way through anything; the next call starts afresh.
                self.live = None;
                let code =
                    if trap.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::Interrupt) {
                        "timeout"
                    } else {
                        "trap"
                    };
                Err(ExtError::new(
                    code,
                    format!("the component stopped: {trap:#}"),
                ))
            }
        }
    }
}

/// A deadline in clock ticks, at least one.
fn ticks(deadline: Duration) -> u64 {
    u64::try_from(deadline.as_millis() / TICK.as_millis())
        .unwrap_or(u32::MAX.into())
        .max(1)
}
