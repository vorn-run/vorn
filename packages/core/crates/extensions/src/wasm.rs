//! Services as sandboxed Wasm components: the same manifest, events, gate
//! and lifecycle as a program, with `[service] component = "x.wasm"` in place
//! of `command`.
//!
//! A component targets the `extension` world of `wit/extension.wit` on
//! `wasm32-wasip2`, with WASI preview 2 and nothing granted: no files,
//! environment or network, only its own imports. Each instance runs on a
//! thread of its own, so a slow one holds up nobody else, and answers every
//! import through the router's `answer`: the same permission checks a
//! program's JSON-RPC requests get. A call past its deadline, a trap or a
//! memory grab past [`MEMORY_CAP`] ends the instance, never vornd; the
//! lifecycle counts it and starts a fresh one on the next use, within the
//! crash budget. What the guest writes to stderr is logged, and its last
//! lines are said with the crash.
//!
//! Wasmtime maps compiled code as it is, so the only compiled code loaded is
//! what Vorn compiled itself into the [`COMPILED`](crate::component::COMPILED)
//! cache; a package's own `.cwasm` is never read. A component missing from
//! the cache, or compiled by another Wasmtime, is compiled by the
//! [`Compiler`] vornd was given, then loaded once per process.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tracing::warn;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, ResourceLimiter, Store};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::api::types::{CallCtx, Event, InitCtx, RowOp};
use crate::api::Method;
use crate::child::{CallError, ErrorData, Incoming};
use crate::component::{Compiler, Source};
use crate::gate::Caller;
use crate::manifest::Manifest;
use crate::router::Router;

mod bindings {
    wasmtime::component::bindgen!({ world: "extension", path: "wit/extension.wit" });
}

use bindings::vorn::extension::types as wit;

/// How often the deadline clock ticks while a component runs.
const TICK: Duration = Duration::from_millis(10);
/// The most linear memory one instance may hold, over all its memories.
pub const MEMORY_CAP: usize = 64 << 20;
/// The most table elements one instance may hold.
const MAX_TABLE: usize = 100_000;
/// How long the `vorn` program may take to compile a component.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);
/// The stderr lines of an instance kept to say with its crash.
const TAIL_LINES: usize = 12;
/// The longest stderr line kept; past it the line is cut.
const MAX_LINE: usize = 2048;
/// Compiled components kept loaded; past this the oldest is let go.
const LOADED: usize = 32;
/// How long past its deadline a call is waited for: a guest blocked in an
/// import cannot be interrupted until the import returns.
const GRACE: Duration = Duration::from_secs(2);

/// The engine settings a `.cwasm` is compiled and loaded with; they must agree.
fn config() -> Config {
    let mut c = Config::new();
    c.epoch_interruption(true);
    // Fuel bounds a call's compute however busy the machine is; the epoch bounds its wall time.
    c.consume_fuel(true);
    c
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One engine per process, the clock that ends calls past their deadline,
/// and the components loaded so far, by the digest of their `.wasm`.
struct Runtime {
    engine: Engine,
    busy: Arc<AtomicUsize>,
    ticker: thread::Thread,
    loaded: Mutex<VecDeque<([u8; 32], Component)>>,
}

fn runtime() -> Result<&'static Runtime, String> {
    static RT: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        let engine = Engine::new(&config()).map_err(|e| format!("{e:#}"))?;
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
            loaded: Mutex::default(),
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

    fn loaded(&self, digest: &[u8; 32]) -> Option<Component> {
        guard(&self.loaded)
            .iter()
            .find(|(d, _)| d == digest)
            .map(|(_, c)| c.clone())
    }

    fn keep(&self, digest: [u8; 32], component: Component) {
        let mut loaded = guard(&self.loaded);
        if loaded.len() >= LOADED {
            loaded.pop_front();
        }
        loaded.push_back((digest, component));
    }
}

/// While held, the deadline clock ticks.
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

fn digest(wasm: &[u8]) -> [u8; 32] {
    Sha256::digest(wasm).into()
}

fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Where `cache` keeps the compiled form of the component `wasm`.
pub fn compiled(cache: &Path, wasm: &[u8]) -> PathBuf {
    cache.join(format!("{}.cwasm", hex(&digest(wasm))))
}

/// Writes `bytes` as `path`, whole or not at all, so a reader never maps half a file.
fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let part = path.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&part, bytes)
        .and_then(|()| std::fs::rename(&part, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&part);
        })
}

/// A name no other compile in this process uses at the same time.
fn unique() -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Compiles the component `wasm` into `cache`, where it is loaded from;
/// answers the compiled file. What `vorn ext compile` runs.
#[cfg(feature = "wasm-compile")]
pub fn precompile(wasm: &Path, cache: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(wasm).map_err(|e| format!("{}: {e}", wasm.display()))?;
    compile_bytes(&wasm.display().to_string(), &bytes, cache)
}

#[cfg(feature = "wasm-compile")]
fn compile_bytes(name: &str, bytes: &[u8], cache: &Path) -> Result<PathBuf, String> {
    let rt = runtime()?;
    let code = rt
        .engine
        .precompile_component(bytes)
        .map_err(|e| format!("{name} does not compile as a component: {e:#}"))?;
    let out = compiled(cache, bytes);
    std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    write_whole(&out, &code).map_err(|e| format!("{}: {e}", out.display()))?;
    prune(cache);
    Ok(out)
}

/// The compiled components a cache keeps; a live rebuild adds one per save.
#[cfg(feature = "wasm-compile")]
const KEPT: usize = 64;

/// Removes all but the [`KEPT`] newest compiled components; one removed is compiled again when next used.
#[cfg(feature = "wasm-compile")]
fn prune(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut compiled: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "cwasm"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    compiled.sort_by_key(|c| std::cmp::Reverse(c.0));
    for (_, old) in compiled.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(old);
    }
}

/// Runs `vorn ext compile` on a copy of `bytes` in `cache`, so what it
/// compiles is what was hashed even while a rebuild rewrites the package's file.
async fn compile_with(vorn: &Path, name: &str, bytes: &[u8], cache: &Path) -> Result<(), String> {
    // Its own name, so two compiles of the same bytes never remove each other's copy.
    let snapshot = cache.join(format!("{}.{}.wasm", hex(&digest(bytes)), unique()));
    let (dir, copy, owned) = (cache.to_path_buf(), snapshot.clone(), bytes.to_vec());
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir).and_then(|()| write_whole(&copy, &owned))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{}: {e}", snapshot.display()))?;
    let mut command = vorn_spawn::tokio_command(vorn);
    command
        .args(["ext", "compile"])
        .arg(&snapshot)
        .arg("--into")
        .arg(cache)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let ran = tokio::time::timeout(COMPILE_TIMEOUT, command.output()).await;
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(snapshot)).await;
    let output = ran
        .map_err(|_| {
            format!(
                "{name} did not compile within {} s",
                COMPILE_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("{name} could not be compiled: {} {e}", vorn.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "{name} did not compile: {}",
        said.trim().trim_start_matches("vorn: ")
    ))
}

/// Fills `cache` with the compiled form of `bytes`, by `compiler`.
async fn compile(
    compiler: &Compiler,
    name: &str,
    bytes: &[u8],
    cache: &Path,
) -> Result<(), String> {
    match compiler {
        Compiler::None => Err(format!(
            "{name} is not compiled for this Vorn, and nothing here compiles it: \
             the vorn command does, beside vornd"
        )),
        Compiler::Program(vorn) => compile_with(vorn, name, bytes, cache).await,
        #[cfg(feature = "wasm-compile")]
        Compiler::InProcess => {
            let (name, bytes, cache) = (name.to_owned(), bytes.to_vec(), cache.to_path_buf());
            tokio::task::spawn_blocking(move || compile_bytes(&name, &bytes, &cache).map(drop))
                .await
                .map_err(|e| e.to_string())?
        }
    }
}

/// Loads compiled code from Vorn's own cache.
///
/// Read into memory rather than mapped from the file: macOS's hardened
/// runtime only lets a file's pages run when they are signed like Vorn's own
/// code, and the cache is not, so a mapping would need library validation
/// off for the whole app.
fn deserialize(rt: &Runtime, code: &Path) -> Result<Component, String> {
    let bytes = std::fs::read(code).map_err(|e| format!("{}: {e}", code.display()))?;
    // SAFETY: the bytes are compiled code, run as they are. `code` is in
    // Vorn's own cache, written only by `compile_bytes` with `config()` from
    // the bytes its name hashes; Wasmtime still refuses one built by another
    // version or for another CPU, which `load` then compiles again.
    unsafe { Component::deserialize(&rt.engine, bytes) }.map_err(|e| format!("{e:#}"))
}

/// The bytes of `source`: borrowed when they are compiled into the host.
async fn read(source: &Source) -> Result<std::borrow::Cow<'static, [u8]>, String> {
    match source {
        Source::Bundled(bytes) => Ok(std::borrow::Cow::Borrowed(*bytes)),
        Source::File(path) => {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                std::fs::read(&path)
                    .map(std::borrow::Cow::Owned)
                    .map_err(|e| format!("{}: {e}", path.display()))
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

/// The component `source`, compiled into `cache` first when it is not there.
async fn load(
    source: &Source,
    name: &str,
    cache: &Path,
    compiler: &Compiler,
) -> Result<Component, String> {
    let rt = runtime()?;
    let bytes = read(source).await?;
    let d = digest(&bytes);
    if let Some(c) = rt.loaded(&d) {
        return Ok(c);
    }
    let code = cache.join(format!("{}.cwasm", hex(&d)));
    let mapped = {
        let code = code.clone();
        tokio::task::spawn_blocking(move || {
            let mapped = code.is_file().then(|| deserialize(rt, &code));
            if let Some(Err(_)) = &mapped {
                // Compiled by another Vorn's Wasmtime: compiled again below.
                let _ = std::fs::remove_file(&code);
            }
            mapped
        })
        .await
        .map_err(|e| e.to_string())?
    };
    let component = match mapped {
        Some(Ok(c)) => c,
        stale => {
            if let Some(Err(why)) = stale {
                warn!("[extensions] {name}: the compiled copy no longer loads ({why})");
            }
            compile(compiler, name, &bytes, cache).await?;
            let code = code.clone();
            tokio::task::spawn_blocking(move || deserialize(rt, &code))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("{name} is not compiled for this Vorn: {e}"))?
        }
    };
    rt.keep(d, component.clone());
    Ok(component)
}

/// Compiles `source` into `cache` unless it is there, without starting it:
/// done once an extension is installed or switched on, so its first use waits for nothing.
pub async fn prepare(
    source: &Source,
    name: &str,
    cache: &Path,
    compiler: &Compiler,
) -> Result<(), String> {
    load(source, name, cache, compiler).await.map(drop)
}

/// What a guest writes to stderr: each line logged, the last few kept for its crash.
#[derive(Clone)]
struct Stderr {
    name: Arc<str>,
    tail: Arc<Mutex<VecDeque<String>>>,
}

impl Stderr {
    fn said(&self) -> String {
        let tail = guard(&self.tail);
        if tail.is_empty() {
            return String::new();
        }
        let lines: Vec<&str> = tail.iter().map(String::as_str).collect();
        format!("; it said: {}", lines.join(" | "))
    }
}

impl IsTerminal for Stderr {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for Stderr {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(Lines {
            to: self.clone(),
            partial: Vec::new(),
        })
    }
}

/// One stream of [`Stderr`], cut into lines.
struct Lines {
    to: Stderr,
    partial: Vec<u8>,
}

impl Lines {
    fn line(&mut self) {
        let line = String::from_utf8_lossy(&self.partial).trim_end().to_owned();
        self.partial.clear();
        if line.is_empty() {
            return;
        }
        warn!("[extensions] {}: {line}", self.to.name);
        let mut tail = guard(&self.to.tail);
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

impl tokio::io::AsyncWrite for Lines {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let me = self.get_mut();
        let mut rest = buf;
        while let Some(at) = rest.iter().position(|b| *b == b'\n') {
            let room = MAX_LINE.saturating_sub(me.partial.len());
            me.partial.extend_from_slice(&rest[..at.min(room)]);
            me.line();
            rest = &rest[at + 1..];
        }
        let room = MAX_LINE.saturating_sub(me.partial.len());
        me.partial.extend_from_slice(&rest[..rest.len().min(room)]);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        if !me.partial.is_empty() {
            me.line();
        }
        Poll::Ready(Ok(()))
    }
}

impl Drop for Lines {
    fn drop(&mut self) {
        if !self.partial.is_empty() {
            self.line();
        }
    }
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

/// The memory cap over every linear memory of the instance, and the most it has held.
struct Limits {
    total: usize,
    peak: Arc<AtomicUsize>,
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let total = (self.total - current.min(self.total)).saturating_add(desired);
        let ok = total <= MEMORY_CAP;
        if ok {
            self.total = total;
            self.peak.fetch_max(total, Ordering::Relaxed);
        }
        Ok(ok)
    }

    fn table_growing(
        &mut self,
        _: usize,
        desired: usize,
        _: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE)
    }
}

fn wit_err(code: &str, message: impl Into<String>) -> wit::ExtError {
    wit::ExtError {
        code: code.to_owned(),
        message: message.into(),
    }
}

fn parsed(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Null)
}

/// A JSON answer read as `T`, or the error that says it did not read.
fn typed<T: serde::de::DeserializeOwned>(method: Method, v: Value) -> Result<T, wit::ExtError> {
    serde_json::from_value(v).map_err(|e| {
        wit_err(
            "failed",
            format!("{} answered what does not read: {e}", method.name()),
        )
    })
}

impl State {
    /// One import, answered as the router answers a program's request of the same method.
    fn ask(&self, method: Method, params: Value) -> Result<Value, wit::ExtError> {
        let Some(router) = self.router.upgrade() else {
            return Err(wit_err("stopping", "Vorn is stopping"));
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
                let code = data["code"].as_str().unwrap_or("failed");
                Err(wit_err(code, message))
            }
        }
    }
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

    fn rows(&mut self, collection: String) -> Result<Vec<String>, wit::ExtError> {
        let v = self.ask(Method::HostRows, json!({ "collection": collection }))?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string)
            .collect())
    }

    fn start_session(&mut self, req: wit::SessionStart) -> Result<String, wit::ExtError> {
        let req = json!({
            "project": req.project, "agent": req.agent, "prompt": req.prompt,
            "worktree": req.worktree, "branch": req.branch, "name": req.name,
            "headless": req.headless,
        });
        let v = self.ask(Method::HostStartSession, json!({ "req": req }))?;
        typed(Method::HostStartSession, v)
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
        let r: crate::api::types::Response = typed(Method::NetFetch, v)?;
        Ok(wit::Response {
            status: r.status,
            headers: r
                .headers
                .into_iter()
                .map(|h| wit::Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
            body: r.body,
        })
    }
}

impl bindings::vorn::extension::storage::Host for State {
    fn prefix(&mut self) -> Result<String, wit::ExtError> {
        let v = self.ask(Method::StoragePrefix, json!({}))?;
        typed(Method::StoragePrefix, v)
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
        typed(Method::StorageExec, v)
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

    fn transaction(
        &mut self,
        statements: Vec<wit::Statement>,
    ) -> Result<Vec<String>, wit::ExtError> {
        let statements: Vec<Value> = statements
            .iter()
            .map(|s| {
                let params: Vec<Value> = s.params.iter().map(|p| parsed(p)).collect();
                json!({ "sql": s.sql, "params": params })
            })
            .collect();
        let v = self.ask(
            Method::StorageTransaction,
            json!({ "statements": statements }),
        )?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string)
            .collect())
    }
}

impl bindings::vorn::extension::blobs::Host for State {
    fn put(&mut self, name: String, data: Vec<u8>) -> Result<(), wit::ExtError> {
        let data = crate::api::base64::encode(&data);
        self.ask(Method::BlobsPut, json!({ "name": name, "data": data }))
            .map(drop)
    }

    fn get(&mut self, name: String) -> Result<Option<Vec<u8>>, wit::ExtError> {
        match self.ask(Method::BlobsGet, json!({ "name": name }))? {
            Value::String(b64) => crate::api::base64::decode(&b64)
                .map(Some)
                .map_err(|e| wit_err("failed", e)),
            _ => Ok(None),
        }
    }

    fn list(&mut self, prefix: String) -> Result<Vec<wit::Blob>, wit::ExtError> {
        let v = self.ask(Method::BlobsList, json!({ "prefix": prefix }))?;
        let listed: Vec<crate::api::types::Blob> = typed(Method::BlobsList, v)?;
        Ok(listed
            .into_iter()
            .map(|b| wit::Blob {
                name: b.name,
                size: b.size,
                modified: b.modified,
            })
            .collect())
    }

    fn delete(&mut self, name: String) -> Result<bool, wit::ExtError> {
        let v = self.ask(Method::BlobsDelete, json!({ "name": name }))?;
        typed(Method::BlobsDelete, v)
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
        deadline: Duration,
        done: oneshot::Sender<()>,
    },
}

/// A component service: its thread, and what it was last seen to hold.
pub struct Service {
    name: String,
    jobs: Mutex<Option<std_mpsc::Sender<Job>>>,
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

/// What a trap or a passed deadline reads as, with what the guest last said.
fn stopped(name: &str, trap: &wasmtime::Error, stderr: &Stderr) -> CallError {
    let said = stderr.said();
    match trap.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => {
            CallError::Timeout(format!("{name} ran past its deadline and was stopped"))
        }
        Some(wasmtime::Trap::OutOfFuel) => CallError::Timeout(format!(
            "{name} used all the compute one call may and was stopped"
        )),
        // The trap alone: its wasm backtrace names nothing a reader can use.
        Some(t) => CallError::Transport(format!("{name} stopped: {t}{said}")),
        None => CallError::Transport(format!("{name} stopped: {trap:#}{said}")),
    }
}

/// Everything a component's thread is started with.
pub(crate) struct Start {
    pub source: Source,
    /// Vorn's cache of compiled components.
    pub cache: PathBuf,
    pub compiler: Compiler,
    pub name: String,
    pub init: InitCtx,
    pub start_timeout: Duration,
    /// The fuel each call, and the start, may burn.
    pub call_fuel: u64,
    pub router: Weak<Router>,
    pub manifest: Arc<Manifest>,
    pub caller: Caller,
}

impl Service {
    /// Loads and instantiates the component and answers its `guest.init`.
    pub(crate) async fn start(s: Start) -> Result<Service, String> {
        let component = load(&s.source, &s.name, &s.cache, &s.compiler).await?;
        let (jobs, rx) = std_mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
        let ended = Arc::new(AtomicBool::new(false));
        let memory = Arc::new(AtomicUsize::new(0));
        let stderr = Stderr {
            name: Arc::from(s.name.trim_start_matches("[extensions] ")),
            tail: Arc::default(),
        };
        let state = State {
            wasi: WasiCtxBuilder::new().stderr(stderr.clone()).build(),
            table: ResourceTable::new(),
            limits: Limits {
                total: 0,
                peak: Arc::clone(&memory),
            },
            router: s.router,
            manifest: s.manifest,
            caller: s.caller,
            tokio: tokio::runtime::Handle::current(),
        };
        let thread_name = format!("ext-wasm {}", s.init.id);
        let (name, init) = (s.name.clone(), s.init);
        let budget = Budget {
            start: s.start_timeout,
            fuel: s.call_fuel,
        };
        let flag = Arc::clone(&ended);
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                run(
                    &name,
                    component,
                    state,
                    &init,
                    budget,
                    Ends {
                        ready: ready_tx,
                        jobs: &rx,
                        ended: &flag,
                        stderr: &stderr,
                    },
                );
                flag.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        // The thread holds the start to its deadline; this waits a little past it.
        let ready = tokio::time::timeout(s.start_timeout + TICK * 10, ready_rx).await;
        match ready {
            Ok(Ok(Ok(()))) => Ok(Service {
                name: s.name,
                jobs: Mutex::new(Some(jobs)),
                ended,
                memory,
            }),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(format!("{} ended as it started", s.name)),
            Err(_) => Err(format!("{} did not start in time", s.name)),
        }
    }

    fn send(&self, job: Job) -> Result<(), CallError> {
        let jobs = guard(&self.jobs);
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
        match tokio::time::timeout(deadline + GRACE, answer).await {
            Ok(Ok(answered)) => answered,
            Ok(Err(_)) => Err(transport(format!(
                "{} stopped before it answered",
                self.name
            ))),
            Err(_) => Err(CallError::Timeout(format!(
                "{} ran past its deadline and was stopped",
                self.name
            ))),
        }
    }

    /// `guest.on-event`; nothing answers it, but it resolves only once the guest is done with it.
    pub async fn on_event(&self, batch: Vec<Event>, deadline: Duration) -> Result<(), CallError> {
        let (done, handled) = oneshot::channel();
        self.send(Job::Events {
            batch,
            deadline,
            done,
        })?;
        let _ = tokio::time::timeout(deadline + GRACE, handled).await;
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

    fn close(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        // Its thread ends once the queue is gone and the job it runs, if any, returns.
        guard(&self.jobs).take();
        Box::pin(async {})
    }
}

/// Gives the next call its whole fuel, whatever the last one left.
fn refuel(store: &mut Store<State>, fuel: u64) {
    // Fails only on an engine without fuel, which `config` rules out.
    let _ = store.set_fuel(fuel);
}

/// What the component's thread talks to: who waits for it to start, its
/// jobs, the flag that tells the lifecycle it ended, and its stderr.
struct Ends<'a> {
    ready: oneshot::Sender<Result<(), String>>,
    jobs: &'a std_mpsc::Receiver<Job>,
    ended: &'a AtomicBool,
    stderr: &'a Stderr,
}

/// The handshake a component makes before it is linked: the API version its
/// imports name against the one this Vorn speaks, refused naming both rather
/// than as a link error.
fn speaks(component: &Component, engine: &Engine) -> Result<(), String> {
    let ty = component.component_type();
    let built = crate::compat::imported(ty.imports(engine).map(|(name, _)| name));
    built.map_or(Ok(()), |built| {
        crate::compat::admit(built, crate::compat::ApiVersion::host())
    })
}

/// What the start may take, and the fuel each call gets afresh.
#[derive(Debug, Clone, Copy)]
struct Budget {
    start: Duration,
    fuel: u64,
}

/// The component's thread: instantiate, init, then calls and events until the queue goes or it traps.
fn run(
    name: &str,
    component: Component,
    state: State,
    init: &InitCtx,
    budget: Budget,
    ends: Ends<'_>,
) {
    let Ends {
        ready,
        jobs,
        ended,
        stderr,
    } = ends;
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // WASI's own waits (a guest's sleep) run on vornd's runtime.
    let tokio = state.tokio.clone();
    let _in_tokio = tokio.enter();
    if let Err(why) = speaks(&component, &rt.engine) {
        let _ = ready.send(Err(format!("{name} did not start: {why}")));
        return;
    }
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
    store.set_epoch_deadline(ticks(budget.start));
    refuel(&mut store, budget.fuel);
    let started = {
        let _busy = rt.enter();
        bindings::Extension::instantiate(&mut store, &component, &linker).and_then(|api| {
            let ctx = wit::InitCtx {
                id: init.id.clone(),
                version: init.version.clone(),
                vorn: init.vorn.clone(),
                api: init.api.clone(),
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
            let why = stopped(name, &e, stderr);
            let _ = ready.send(Err(format!(
                "{name} did not start: {}",
                crate::router::ext_error(why)
            )));
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
                refuel(&mut store, budget.fuel);
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
                            code: crate::api::EXT_ERROR,
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
                        let _ = reply.send(Err(stopped(name, &trap, stderr)));
                        return;
                    }
                }
            }
            Job::Events {
                batch,
                deadline,
                done,
            } => {
                store.set_epoch_deadline(ticks(deadline));
                refuel(&mut store, budget.fuel);
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
                if let Err(trap) = handled {
                    ended.store(true, Ordering::Release);
                    drop(done);
                    warn!(
                        "[extensions] {}",
                        crate::router::ext_error(stopped(name, &trap, stderr))
                    );
                    return;
                }
                drop(done);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn keeps_the_last_stderr_lines_cut_to_length() {
        let stderr = Stderr {
            name: Arc::from("x"),
            tail: Arc::default(),
        };
        let mut w = Box::into_pin(stderr.async_stream());
        let long = "y".repeat(MAX_LINE * 2);
        w.write_all(format!("thread panicked at src/lib.rs:3\n{long}\nlast").as_bytes())
            .await
            .unwrap();
        drop(w);
        let tail = guard(&stderr.tail).clone();
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0], "thread panicked at src/lib.rs:3");
        assert_eq!(tail[1].len(), MAX_LINE);
        assert_eq!(tail[2], "last");
        for i in 0..TAIL_LINES + 5 {
            let mut w = Box::into_pin(stderr.async_stream());
            w.write_all(format!("{i}\n").as_bytes()).await.unwrap();
        }
        assert_eq!(guard(&stderr.tail).len(), TAIL_LINES);
        assert!(stderr.said().ends_with(&format!("{}", TAIL_LINES + 4)));
    }

    #[cfg(feature = "wasm-compile")]
    #[test]
    fn the_cache_keeps_the_newest_compiled_components() {
        let dir = tempfile::tempdir().unwrap();
        let start = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        for i in 0..KEPT + 3 {
            let path = dir.path().join(format!("{i:03}.cwasm"));
            let file = std::fs::File::create(&path).unwrap();
            file.set_modified(start + Duration::from_secs(u64::try_from(i).unwrap()))
                .unwrap();
        }
        std::fs::write(dir.path().join("other.txt"), b"x").unwrap();
        prune(dir.path());
        let left = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(left, KEPT + 1);
        assert!(!dir.path().join("000.cwasm").exists(), "the oldest went");
        assert!(dir.path().join(format!("{:03}.cwasm", KEPT + 2)).exists());
    }

    #[test]
    fn a_deadline_is_at_least_one_tick() {
        assert_eq!(ticks(Duration::ZERO), 1);
        assert_eq!(ticks(Duration::from_secs(1)), 100);
    }

    #[test]
    fn names_compiled_code_by_the_bytes_it_came_from() {
        let cache = Path::new("/c");
        assert_eq!(compiled(cache, b"a"), compiled(cache, b"a"));
        assert_ne!(compiled(cache, b"a"), compiled(cache, b"b"));
        assert_eq!(
            compiled(cache, b"").file_name().unwrap().to_str().unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855.cwasm"
        );
    }
}
