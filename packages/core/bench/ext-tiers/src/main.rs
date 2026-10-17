//! Worktree cleanup as the Wasm component Vorn ships, measured the way the
//! extension-tiers comparison measured its Wasm tier: first start, restart,
//! an event to its first publish, call round trips, scans, a burst of
//! `worktree.idle` events, and memory. It runs through the router as vornd
//! runs a shipped component ([`Router::call_bundled`]), against a world of
//! 32 sessions and 20 worktrees.
//!
//! `ext-tiers [--reps N] [--out DIR]` builds the component, compiles it once
//! into a cache, then runs each repetition in a fresh process (`ext-tiers
//! run`) so memory reads clean, and prints the medians as a table.

mod stats;
mod sys;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_extensions::api::types::{CallCtx, Event, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::component::{Bundled, Compiler, COMPILED};
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

const MANIFEST: &str = include_str!("../../../extensions/worktrees/vorn-extension.toml");
const ID: &str = "vorn.worktrees";
/// The busy terminals.
const TERMINALS: usize = 32;
/// The most events one delivery carries, as the router's.
const BATCH: usize = 256;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// 32 sessions and two projects' worktrees, counting what reaches them.
struct Fake {
    sessions: Vec<Value>,
    worktrees: Mutex<Vec<Value>>,
    calls: AtomicU64,
    published: AtomicU64,
}

fn worktree(i: usize) -> Value {
    let project = if i.is_multiple_of(2) { "/p/api" } else { "/p/web" };
    let sessions: Vec<String> = if i.is_multiple_of(5) {
        vec![format!("s{i}")]
    } else {
        Vec::new()
    };
    json!({
        "path": format!("{project}/.worktrees/w{i}"),
        "projectPath": project,
        "projectName": project.trim_start_matches("/p/"),
        "branch": format!("feature/{i}"),
        "main": i == 0,
        "orphan": i == 7,
        "dirty": i.is_multiple_of(3),
        "sessionIds": sessions,
        "sizeBytes": 50_000_000 * (i as u64 + 1),
        "artifactBytes": if i.is_multiple_of(4) { 10_000_000 } else { 0 },
        "verdict": if i.is_multiple_of(3) { "keep" } else { "remove" },
        "reasons": "merged",
    })
}

impl Fake {
    fn new() -> Fake {
        Fake {
            sessions: (0..TERMINALS)
                .map(|i| json!({ "id": format!("s{i}"), "status": "working" }))
                .collect(),
            worktrees: Mutex::new((0..20).map(worktree).collect()),
            calls: AtomicU64::new(0),
            published: AtomicU64::new(0),
        }
    }
}

impl World for Fake {
    fn call(&self, _: &Caller, method: &str, args: Value) -> Answer<Value> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let worktrees = self.worktrees.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let answer = match method {
            "sessions.list" => Ok(Value::Array(self.sessions.clone())),
            "worktrees.list" => {
                let project = args["project"].as_str();
                Ok(Value::Array(
                    worktrees
                        .iter()
                        .filter(|w| project.is_none_or(|p| w["projectPath"] == p))
                        .cloned()
                        .collect(),
                ))
            }
            other => Err(ExtError::new("unknown-method", format!("the bench has no {other}"))),
        };
        Box::pin(async move { answer })
    }

    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "the bench fetches nothing")) })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn published(&self, _: &Caller, _: &str, _: &[Value]) {
        self.published.fetch_add(1, Ordering::SeqCst);
    }
}

/// Worktree cleanup, shipped as a component, behind a router of its own.
struct Rig {
    router: Arc<Router>,
    world: Arc<Fake>,
    bundled: Bundled,
    _root: tempfile::TempDir,
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "client".into(),
        project: None,
    }
}

impl Rig {
    /// A router whose cache already holds the component, as vornd's does after the first start.
    fn new(wasm: &'static [u8], cache: &Path) -> Rig {
        let root = tempfile::tempdir().expect("a temp dir");
        let compiled = root.path().join(COMPILED);
        std::fs::create_dir_all(&compiled).expect("the cache");
        for entry in std::fs::read_dir(cache).expect("the compiled cache").flatten() {
            std::fs::copy(entry.path(), compiled.join(entry.file_name())).expect("copied");
        }
        let world = Arc::new(Fake::new());
        let mut settings = RouterSettings::new(Vec::new(), "bench");
        // Nothing compiles here: a run loads what the cache holds, as vornd does.
        settings.compiler = Compiler::None;
        let router = Router::new(
            PackStore::new(root.path()),
            Lifecycle::new(Policy::default()),
            Arc::clone(&world) as Arc<dyn World>,
            settings,
            vorn_extensions::events::Bus::new(),
        );
        let manifest = vorn_extensions::manifest::file::parse_builtin(MANIFEST).expect("the manifest");
        Rig {
            router,
            world,
            bundled: Bundled {
                manifest: Arc::new(manifest),
                wasm,
            },
            _root: root,
        }
    }

    async fn call(&self, action: &str, args: Value) -> Value {
        self.router
            .call_bundled(&self.bundled, action, args, ctx())
            .await
            .unwrap_or_else(|e| panic!("{action}: {}", e.message))
    }

    async fn events(&self, batch: Vec<Event>) {
        let batch: Vec<Arc<Event>> = batch.into_iter().map(Arc::new).collect();
        self.router.hand_bundled(&self.bundled, &batch).await;
    }

    async fn stop(&self) {
        self.router.stop(ID).await;
    }

    /// The most linear memory its instance has held, in MB.
    #[allow(clippy::cast_precision_loss)]
    fn linear_mb(&self) -> Option<f64> {
        let lease = self.router.running(ID)?;
        lease.component_memory().map(|b| stats::round(b as f64 / (1024.0 * 1024.0)))
    }
}

fn idle(i: usize) -> Event {
    let project = if i.is_multiple_of(2) { "/p/api" } else { "/p/web" };
    let w = (i % 20) | 1;
    Event {
        name: "worktree.idle".into(),
        data: json!({ "worktreePath": format!("{project}/.worktrees/w{w}"), "projectPath": project }),
    }
}

fn me() -> u32 {
    std::process::id()
}

/// What the run added to this process, and the component's own linear memory.
fn held(rig: &Rig, base: f64) -> Value {
    json!({
        "host_delta_mb": stats::round(sys::memory(me()).unwrap_or(0.0) - base),
        "linear_mb": rig.linear_mb(),
    })
}

/// One repetition, in this process.
async fn run(wasm: &'static [u8], cache: &Path) -> Value {
    let rig = Rig::new(wasm, cache);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = sys::memory(me()).unwrap_or(0.0);
    let t = Instant::now();
    rig.call("cancel", Value::Null).await;
    let first_start = ms(t.elapsed());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let idle_memory = held(&rig, base);

    let mut restart = Vec::new();
    for _ in 0..15 {
        rig.stop().await;
        let t = Instant::now();
        rig.call("cancel", Value::Null).await;
        restart.push(ms(t.elapsed()));
    }
    let mut to_publish = Vec::new();
    for _ in 0..15 {
        rig.stop().await;
        let before = rig.world.published.load(Ordering::SeqCst);
        let t = Instant::now();
        rig.events(vec![idle(1)]).await;
        assert!(rig.world.published.load(Ordering::SeqCst) > before, "the idle worktree was offered");
        to_publish.push(ms(t.elapsed()));
    }
    let mut call = Vec::new();
    for _ in 0..2000 {
        let t = Instant::now();
        rig.call("cancel", Value::Null).await;
        call.push(us(t.elapsed()));
    }
    let mut scan = Vec::new();
    for _ in 0..300 {
        let t = Instant::now();
        rig.call("scan", Value::Null).await;
        scan.push(us(t.elapsed()));
    }
    let (cpu0, calls0) = (sys::cpu(me()).unwrap_or(0.0), rig.world.calls.load(Ordering::Relaxed));
    let t = Instant::now();
    let events: Vec<Event> = (0..5000).map(idle).collect();
    for batch in events.chunks(BATCH) {
        rig.events(batch.to_vec()).await;
    }
    let took = t.elapsed();
    let spent = sys::cpu(me()).unwrap_or(0.0) - cpu0;
    let render = {
        let t = Instant::now();
        let drawn = rig
            .call("ui.render", json!({ "point": "worktrees", "surface": {} }))
            .await;
        assert!(drawn["tree"].is_object(), "a tree");
        us(t.elapsed())
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let busy = held(&rig, base);
    rig.stop().await;
    json!({
        "first_start_ms": stats::round(first_start),
        "restart_ms": stats::summary(&restart),
        "event_to_publish_ms": stats::summary(&to_publish),
        "call_us": stats::summary(&call),
        "scan_us": stats::summary(&scan),
        "render_us": stats::round(render),
        "idle_events": {
            "events_per_s": stats::round(5000.0 / took.as_secs_f64()),
            "cpu_us_per_event": stats::round(spent * 1e6 / 5000.0),
            "core_calls": rig.world.calls.load(Ordering::Relaxed) - calls0,
        },
        "memory_idle": idle_memory,
        "memory_busy": busy,
    })
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn median(runs: &[Value], pointer: &str) -> Value {
    let mut v: Vec<f64> = runs.iter().filter_map(|r| r.pointer(pointer)?.as_f64()).collect();
    if v.is_empty() {
        return Value::Null;
    }
    v.sort_by(f64::total_cmp);
    json!(v[v.len() / 2])
}

fn table(runs: &[Value], wasm_bytes: u64, cwasm_bytes: u64) -> String {
    let row = |label: &str, pointer: &str| {
        let v = median(runs, pointer);
        format!("| {label} | {} |\n", v.as_f64().map_or("—".to_owned(), |n| format!("{n:.2}")))
    };
    let mut out = String::from("| Worktree cleanup, Wasm (shipped) | median |\n|---|---:|\n");
    for (label, pointer) in [
        ("first start (ms)", "/first_start_ms"),
        ("restart p50 (ms)", "/restart_ms/p50"),
        ("cold start: event → first publish p50 (ms)", "/event_to_publish_ms/p50"),
        ("call round trip p50 (µs)", "/call_us/p50"),
        ("call round trip p99 (µs)", "/call_us/p99"),
        ("scan p50 (µs)", "/scan_us/p50"),
        ("scan p99 (µs)", "/scan_us/p99"),
        ("settings page render (µs)", "/render_us"),
        ("5k worktree.idle: events/s", "/idle_events/events_per_s"),
        ("5k worktree.idle: CPU µs/event", "/idle_events/cpu_us_per_event"),
        ("memory idle (MB)", "/memory_idle/host_delta_mb"),
        ("memory busy (MB)", "/memory_busy/host_delta_mb"),
        ("of which linear memory, busy (MB)", "/memory_busy/linear_mb"),
    ] {
        out.push_str(&row(label, pointer));
    }
    out.push_str(&format!(
        "| component .wasm (KB) | {} |\n| compiled .cwasm (KB) | {} |\n",
        wasm_bytes / 1024,
        cwasm_bytes / 1024
    ));
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    if args.get(1).map(String::as_str) == Some("run") {
        let wasm_path = PathBuf::from(flag(&args, "--component").expect("--component"));
        let cache = PathBuf::from(flag(&args, "--cache").expect("--cache"));
        let wasm: &'static [u8] =
            Box::leak(std::fs::read(&wasm_path).expect("the component").into_boxed_slice());
        let result = rt.block_on(run(wasm, &cache));
        println!("{result}");
        return;
    }
    let reps: usize = flag(&args, "--reps").and_then(|n| n.parse().ok()).unwrap_or(5);
    let out = flag(&args, "--out").map(PathBuf::from);
    let wasm = vorn_guest_build::component("vorn-ext-worktrees").unwrap_or_else(|e| panic!("{e}"));
    let cache = tempfile::tempdir().expect("a temp dir");
    let compiled = vorn_extensions::wasm::precompile(&wasm, cache.path()).unwrap_or_else(|e| panic!("{e}"));
    let size = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut runs = Vec::with_capacity(reps);
    for rep in 0..reps {
        let exe = std::env::current_exe().expect("own path");
        let ran = Command::new(exe)
            .arg("run")
            .arg("--component")
            .arg(&wasm)
            .arg("--cache")
            .arg(cache.path())
            .output()
            .expect("a run");
        let text = String::from_utf8_lossy(&ran.stdout);
        let result: Value = serde_json::from_str(text.trim()).unwrap_or_else(|_| {
            panic!("run {rep} failed: {}", String::from_utf8_lossy(&ran.stderr))
        });
        eprintln!("run {}/{reps} done", rep + 1);
        runs.push(result);
    }
    let md = table(&runs, size(&wasm), size(&compiled));
    println!("{md}");
    if let Some(dir) = out {
        std::fs::create_dir_all(&dir).expect("the out dir");
        let raw = json!({ "runs": runs, "wasm_bytes": size(&wasm), "cwasm_bytes": size(&compiled) });
        std::fs::write(dir.join("results.json"), format!("{raw:#}\n")).expect("written");
        std::fs::write(dir.join("summary.md"), md).expect("written");
    }
}
