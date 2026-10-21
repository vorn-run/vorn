//! Measures the bench's extensions in each tier: a Wasm component and a
//! program on the same manifest, and the RSS connector as Vorn ships it in
//! Node. Each runs through the router vornd runs, against [`Bench`].
//!
//! `ext-tiers [--reps N] [--out DIR]` builds the components, compiles each
//! once into a cache (as vornd does on install), then runs every extension
//! in every tier `N` times, each repetition in a fresh process (`ext-tiers
//! run`) so memory reads clean, and the crash probe once a tier (`ext-tiers
//! crash`). It prints the medians as a table and writes the raw runs.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_bench_ext_tiers::node::{self, NodeRss};
use vorn_bench_ext_tiers::rig::{Ext, Rig, Tier, ALL, BARRIER, CRASH, RSS, WIDGET, WORKTREES};
use vorn_bench_ext_tiers::world::{project, Bench, TERMINALS};
use vorn_bench_ext_tiers::{fixtures, scenario, stats, sys};

/// The most events one delivery carries, as the router's.
const EVENTS: usize = 5000;
const CALLS: usize = 2000;
const RESTARTS: usize = 15;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[allow(clippy::cast_precision_loss)]
fn mb(bytes: usize) -> f64 {
    stats::round(bytes as f64 / (1024.0 * 1024.0))
}

fn me() -> u32 {
    std::process::id()
}

/// What the extension holds: what it added to this process (the host),
/// its own process's footprint (a program), its linear memory (a component).
fn held(base: f64, child: Option<u32>, linear: Option<usize>) -> Value {
    let host = stats::round(sys::memory(me()).unwrap_or(0.0) - base);
    let child = child.and_then(sys::memory).map(stats::round);
    json!({
        "host_delta_mb": host,
        "child_mb": child,
        "linear_mb": linear.map(mb),
        "total_mb": stats::round(host + child.unwrap_or(0.0)),
    })
}

/// CPU seconds of this process and the extension's own.
fn cpu(child: Option<u32>) -> f64 {
    sys::cpu(me()).unwrap_or(0.0) + child.and_then(sys::cpu).unwrap_or(0.0)
}

/// Waits until the world has seen more than `before` publishes; how long that took.
async fn published_after(rig: &Rig, before: u64, t: Instant) -> f64 {
    while rig.world.published() <= before {
        assert!(t.elapsed() < Duration::from_secs(10), "{} never published", rig.ext.name);
        tokio::task::yield_now().await;
    }
    ms(t.elapsed())
}

/// An event that makes `ext` publish, the `i`th of a burst.
fn event(ext: Ext, i: usize) -> Option<(&'static str, Value)> {
    match ext.id {
        "bench.widget" => Some(("session.status", json!({ "id": format!("s{}", i % TERMINALS) }))),
        "bench.worktrees" => {
            let w = (i % 20) | 1;
            let p = project(w);
            Some(("worktree.idle", json!({ "worktreePath": format!("{p}/.worktrees/w{w}"), "projectPath": p })))
        }
        _ => None,
    }
}

/// The extension's typical action, timed apart from the bare round trip.
fn work(ext: Ext) -> (&'static str, Value) {
    match ext.id {
        "bench.widget" => ("ui.event", json!({ "point": "widget", "event": { "id": "suggest:r1:0", "kind": "click" } })),
        "bench.worktrees" => ("scan", Value::Null),
        "bench.rss" => ("readFeed", json!({ "url": scenario::feed("rss2.xml"), "limit": 20 })),
        _ => ("echo", json!({ "said": "hello" })),
    }
}

/// The trees the extension draws.
fn points(ext: Ext) -> &'static [&'static str] {
    match ext.id {
        "bench.widget" => &["widget", "compact"],
        "bench.worktrees" => &["worktrees", "idle"],
        _ => &[],
    }
}

/// One repetition of `ext` in `tier`, in this process.
async fn run(ext: Ext, tier: Tier, cache: &Path, feeds: SocketAddr) -> Value {
    let rig = Rig::new(ext, tier, Bench::new(Some(feeds)), Some(cache)).unwrap_or_else(|e| panic!("{e}"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = sys::memory(me()).unwrap_or(0.0);
    let t = Instant::now();
    let _ = rig.call(BARRIER, Value::Null).await;
    let cold = ms(t.elapsed());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let idle = held(base, rig.pid(), rig.linear_bytes());

    let mut warm = Vec::new();
    for _ in 0..RESTARTS {
        rig.stop().await;
        let t = Instant::now();
        let _ = rig.call(BARRIER, Value::Null).await;
        warm.push(ms(t.elapsed()));
    }
    let mut cold_event = Vec::new();
    let mut warm_event = Vec::new();
    if let Some((name, data)) = event(ext, 1) {
        for stopped in [true, false] {
            for _ in 0..RESTARTS {
                if stopped {
                    rig.stop().await;
                } else {
                    rig.handled().await;
                }
                let before = rig.world.published();
                let t = Instant::now();
                rig.emit(name, data.clone());
                let took = published_after(&rig, before, t).await;
                if stopped { &mut cold_event } else { &mut warm_event }.push(took);
            }
        }
    }
    let mut call = Vec::new();
    for _ in 0..CALLS {
        let t = Instant::now();
        let _ = rig.call(BARRIER, Value::Null).await;
        call.push(us(t.elapsed()));
    }
    // The Widget's world: two agents asking, so its trees and clicks have something to draw.
    if ext.id == WIDGET.id {
        for i in 0..TERMINALS {
            rig.emit("session.created", json!({ "id": format!("s{i}") }));
        }
        for ask in scenario::asks() {
            rig.emit("agent.permission", ask);
        }
        rig.handled().await;
    }
    let (action, args) = work(ext);
    let mut worked = Vec::new();
    for _ in 0..300 {
        let t = Instant::now();
        rig.call(action, args.clone()).await.unwrap_or_else(|e| panic!("{action}: {}", e.message));
        worked.push(us(t.elapsed()));
    }
    let mut trees = serde_json::Map::new();
    for point in points(ext) {
        let mut took = Vec::new();
        let mut bytes = 0;
        for cells in [false, true] {
            for _ in 0..300 {
                let surface = json!({ "cells": cells, "width": 280.0, "height": 400.0 });
                let t = Instant::now();
                let drawn = rig
                    .call("ui.render", json!({ "point": point, "surface": surface }))
                    .await
                    .unwrap_or_else(|e| panic!("{point}: {}", e.message));
                took.push(us(t.elapsed()));
                bytes = drawn.to_string().len();
            }
        }
        trees.insert((*point).to_owned(), json!({ "us": stats::summary(&took), "bytes": bytes }));
    }
    let busy = busy(&rig).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let memory_busy = held(base, rig.pid(), rig.linear_bytes());
    rig.stop().await;
    json!({
        "cold_start_ms": stats::round(cold),
        "warm_start_ms": stats::summary(&warm),
        "cold_event_to_publish_ms": (!cold_event.is_empty()).then(|| stats::summary(&cold_event)),
        "warm_event_to_publish_ms": (!warm_event.is_empty()).then(|| stats::summary(&warm_event)),
        "call_us": stats::summary(&call),
        "work": { "action": action, "us": stats::summary(&worked) },
        "render": trees,
        "busy": busy,
        "memory_idle": idle,
        "memory_busy": memory_busy,
    })
}

/// A burst: 5000 events for an extension that takes them, else 200 reads of every feed.
async fn busy(rig: &Rig) -> Value {
    let pid = rig.pid();
    let (cpu0, calls0) = (cpu(pid), rig.world.calls.load(std::sync::atomic::Ordering::Relaxed));
    let t = Instant::now();
    let n = if event(rig.ext, 0).is_some() {
        for i in 0..EVENTS {
            if let Some((name, data)) = event(rig.ext, i) {
                rig.emit(name, data);
            }
        }
        rig.handled().await;
        EVENTS
    } else {
        let args = json!({ "urls": scenario::feeds().join("\n"), "perFeed": 5 });
        for _ in 0..200 {
            rig.call("readFeeds", args.clone()).await.unwrap_or_else(|e| panic!("readFeeds: {}", e.message));
        }
        200
    };
    let took = t.elapsed();
    let spent = cpu(pid) - cpu0;
    #[allow(clippy::cast_precision_loss)]
    let per = n as f64;
    json!({
        "what": if n == EVENTS { "events" } else { "readFeeds" },
        "per_s": stats::round(per / took.as_secs_f64()),
        "cpu_us_each": stats::round(spent * 1e6 / per),
        "core_calls": rig.world.calls.load(std::sync::atomic::Ordering::Relaxed) - calls0,
    })
}

/// One repetition of the Node connector, in this process.
async fn run_node(node: &Path, feeds: SocketAddr) -> Value {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = sys::memory(me()).unwrap_or(0.0);
    let list = scenario::feeds();
    let open = || NodeRss::open(node, feeds, &list);
    let t = Instant::now();
    let mut rss = open().await.unwrap_or_else(|e| panic!("{e}"));
    let _ = rss.action("bench.barrier", &Value::Null).await;
    let cold = ms(t.elapsed());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let idle = held(base, node::pid(), None);
    let mut warm = Vec::new();
    for _ in 0..RESTARTS {
        rss.close().await;
        let t = Instant::now();
        rss = open().await.unwrap_or_else(|e| panic!("{e}"));
        let _ = rss.action("bench.barrier", &Value::Null).await;
        warm.push(ms(t.elapsed()));
    }
    let mut call = Vec::new();
    for _ in 0..CALLS {
        let t = Instant::now();
        let _ = rss.action("bench.barrier", &Value::Null).await;
        call.push(us(t.elapsed()));
    }
    let (action, args) = work(RSS);
    let mut worked = Vec::new();
    for _ in 0..300 {
        let t = Instant::now();
        rss.action(action, &args).await.unwrap_or_else(|e| panic!("{action}: {e}"));
        worked.push(us(t.elapsed()));
    }
    let pid = node::pid();
    let cpu0 = cpu(pid);
    let args = json!({ "urls": scenario::feeds().join("\n"), "perFeed": 5 });
    let t = Instant::now();
    for _ in 0..200 {
        rss.action("readFeeds", &args).await.unwrap_or_else(|e| panic!("readFeeds: {e}"));
    }
    let took = t.elapsed();
    let spent = cpu(pid) - cpu0;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let memory_busy = held(base, pid, None);
    rss.close().await;
    json!({
        "cold_start_ms": stats::round(cold),
        "warm_start_ms": stats::summary(&warm),
        "call_us": stats::summary(&call),
        "work": { "action": action, "us": stats::summary(&worked) },
        "busy": {
            "what": "readFeeds",
            "per_s": stats::round(200.0 / took.as_secs_f64()),
            "cpu_us_each": stats::round(spent * 1e6 / 200.0),
        },
        "memory_idle": idle,
        "memory_busy": memory_busy,
    })
}

async fn timed(rig: &Rig, action: &str, args: Value) -> Value {
    let t = Instant::now();
    let answer = rig.call(action, args).await;
    json!({
        "ms": stats::round(ms(t.elapsed())),
        "answer": match answer {
            Ok(v) => json!({ "ok": v.to_string().len() > 0 }),
            Err(e) => json!({ "error": e.code, "message": e.message }),
        },
    })
}

/// What a misbehaving extension costs the host and a neighbour: a panic,
/// a call that never returns, and a grab of 128 MB, each followed by a
/// call that starts it again; the Widget beside it keeps answering.
async fn crash(tier: Tier, sibling: Tier, cache: &Path) -> Value {
    let rig = Rig::new(CRASH, tier, Bench::new(None), Some(cache)).unwrap_or_else(|e| panic!("{e}"));
    let neighbour = Rig::new(WIDGET, sibling, Bench::new(None), Some(cache)).unwrap_or_else(|e| panic!("{e}"));
    let echo = json!({ "said": "hello" });
    let _ = rig.call("echo", echo.clone()).await;
    let _ = neighbour.call(BARRIER, Value::Null).await;
    let base = sys::memory(me()).unwrap_or(0.0);

    let panicked = timed(&rig, "crash", Value::Null).await;
    let after_panic = timed(&rig, "echo", echo.clone()).await;

    // The neighbour's round trips while the probe spins until the call times out.
    let spinning = {
        let rig = &rig;
        async move { timed(rig, "spin", Value::Null).await }
    };
    let beside = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut took = Vec::new();
        for _ in 0..200 {
            let t = Instant::now();
            let _ = neighbour.call(BARRIER, Value::Null).await;
            took.push(us(t.elapsed()));
        }
        stats::summary(&took)
    };
    let (spun, neighbour_during_spin) = tokio::join!(spinning, beside);
    let after_spin = timed(&rig, "echo", echo.clone()).await;

    let grew = timed(&rig, "grow", json!({ "mb": 128 })).await;
    let memory_after_grow = held(base, rig.pid(), rig.linear_bytes());
    let after_grow = timed(&rig, "echo", echo).await;
    let neighbour_after = timed(&neighbour, BARRIER, Value::Null).await;
    rig.stop().await;
    neighbour.stop().await;
    json!({
        "panic": panicked,
        "restart_after_panic": after_panic,
        "spin": spun,
        "neighbour_during_spin_us": neighbour_during_spin,
        "restart_after_spin": after_spin,
        "grow_128mb": grew,
        "memory_after_grow": memory_after_grow,
        "restart_after_grow": after_grow,
        "neighbour_after": neighbour_after,
        "host_survived": true,
    })
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn ext_named(id: &str) -> Ext {
    ALL.into_iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no extension {id}"))
}

fn tier_from(args: &[String]) -> Tier {
    let path = PathBuf::from(flag(args, "--path").expect("--path"));
    match flag(args, "--tier").as_deref() {
        Some("wasm") => Tier::Wasm(path),
        _ => Tier::Program(path),
    }
}

/// The child's `run` or `crash`, from its arguments.
fn child(args: &[String], rt: &tokio::runtime::Runtime) -> Value {
    let cache = PathBuf::from(flag(args, "--cache").expect("--cache"));
    rt.block_on(async {
        match args[1].as_str() {
            "crash" => {
                let sibling = PathBuf::from(flag(args, "--sibling").expect("--sibling"));
                let sibling = match tier_from(args) {
                    Tier::Wasm(_) => Tier::Wasm(sibling),
                    Tier::Program(_) => Tier::Program(sibling),
                };
                crash(tier_from(args), sibling, &cache).await
            }
            _ => {
                let feeds = fixtures::serve().await.expect("the fixture server");
                match flag(args, "--tier").as_deref() {
                    Some("node") => run_node(&PathBuf::from(flag(args, "--path").expect("--path")), feeds).await,
                    _ => run(ext_named(&flag(args, "--ext").expect("--ext")), tier_from(args), &cache, feeds).await,
                }
            }
        }
    })
}

/// Runs this program as a child with `args`, answering what it printed.
fn spawn(args: &[&str]) -> Value {
    let exe = std::env::current_exe().expect("own path");
    let ran = vorn_spawn::command(exe).args(args).output().expect("a run");
    let text = String::from_utf8_lossy(&ran.stdout);
    serde_json::from_str(text.trim())
        .unwrap_or_else(|_| panic!("{args:?} failed: {}", String::from_utf8_lossy(&ran.stderr)))
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

/// Lines of Rust that are neither blank, comments nor tests.
fn loc(dir: &Path) -> usize {
    let mut n = 0;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += loc(&path);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.file_name().is_some_and(|f| f == "tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        n += text
            .lines()
            .take_while(|l| l.trim() != "#[cfg(test)]")
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
            .count();
    }
    n
}

/// A tier of one extension the parent runs.
struct Column {
    ext: Ext,
    tier: &'static str,
    path: PathBuf,
    sizes: Value,
}

fn median(runs: &[Value], pointer: &str) -> Option<f64> {
    let mut v: Vec<f64> = runs.iter().filter_map(|r| r.pointer(pointer)?.as_f64()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(v[v.len() / 2])
}

const ROWS: [(&str, &str); 22] = [
    ("cold start: first call (ms)", "/cold_start_ms"),
    ("warm start: call after a stop, p50 (ms)", "/warm_start_ms/p50"),
    ("cold start: event → first publish, p50 (ms)", "/cold_event_to_publish_ms/p50"),
    ("event → publish while running, p50 (ms)", "/warm_event_to_publish_ms/p50"),
    ("call round trip p50 (µs)", "/call_us/p50"),
    ("call round trip p99 (µs)", "/call_us/p99"),
    ("typical action p50 (µs)", "/work/us/p50"),
    ("typical action p99 (µs)", "/work/us/p99"),
    ("main tree render p50 (µs)", "/render/main/us/p50"),
    ("main tree render p99 (µs)", "/render/main/us/p99"),
    ("main tree size (bytes)", "/render/main/bytes"),
    ("small tree render p50 (µs)", "/render/small/us/p50"),
    ("busy: events or reads per s", "/busy/per_s"),
    ("busy: CPU µs each", "/busy/cpu_us_each"),
    ("memory idle, total (MB)", "/memory_idle/total_mb"),
    ("of which in the host (MB)", "/memory_idle/host_delta_mb"),
    ("of which its own process (MB)", "/memory_idle/child_mb"),
    ("memory busy, total (MB)", "/memory_busy/total_mb"),
    ("of which linear memory (MB)", "/memory_busy/linear_mb"),
    ("binary (KB)", "/size/binary_kb"),
    ("compiled cache (KB)", "/size/compiled_kb"),
    ("lines of code: extension + tier glue", "/size/loc"),
];

/// The render rows read the main and small trees by name.
fn normalise(ext: Ext, mut run: Value, sizes: &Value) -> Value {
    if let [main, small] = points(ext) {
        let render = run["render"].take();
        run["render"] = json!({ "main": render[*main], "small": render[*small] });
    }
    run["size"] = sizes.clone();
    run
}

fn table(columns: &[Column], runs: &[Vec<Value>]) -> String {
    let mut out = String::from("| |");
    for c in columns {
        out.push_str(&format!(" {} ({}) |", c.ext.name, c.tier));
    }
    out.push_str("\n|---|");
    out.push_str(&"---:|".repeat(columns.len()));
    out.push('\n');
    for (label, pointer) in ROWS {
        out.push_str(&format!("| {label} |"));
        for (c, r) in columns.iter().zip(runs) {
            let normal: Vec<Value> = r.iter().map(|v| normalise(c.ext, v.clone(), &c.sizes)).collect();
            let cell = match normal.first().and_then(|v| v.pointer(pointer)) {
                Some(Value::String(s)) => s.clone(),
                _ => median(&normal, pointer).map_or("—".to_owned(), |n| {
                    if n >= 100.0 {
                        format!("{n:.0}")
                    } else {
                        format!("{n:.2}")
                    }
                }),
            };
            out.push_str(&format!(" {cell} |"));
        }
        out.push('\n');
    }
    out
}

fn crash_table(probes: &[(&str, Value)]) -> String {
    let mut out = String::from("| crash probe |");
    for (tier, _) in probes {
        out.push_str(&format!(" {tier} |"));
    }
    out.push_str("\n|---|");
    out.push_str(&"---|".repeat(probes.len()));
    out.push('\n');
    let said = |v: &Value| match v.pointer("/answer/error") {
        Some(code) => format!("{} in {} ms", code.as_str().unwrap_or("error"), v["ms"]),
        None => format!("answered in {} ms", v["ms"]),
    };
    let rows: [(&str, Box<dyn Fn(&Value) -> String>); 8] = [
        ("panic", Box::new(|p| said(&p["panic"]))),
        ("next call (restart)", Box::new(|p| said(&p["restart_after_panic"]))),
        ("endless loop", Box::new(|p| said(&p["spin"]))),
        ("neighbour p50 during the loop (µs)", Box::new(|p| format!("{}", p["neighbour_during_spin_us"]["p50"]))),
        ("next call (restart)", Box::new(|p| said(&p["restart_after_spin"]))),
        ("grab 128 MB", Box::new(|p| said(&p["grow_128mb"]))),
        ("held after the grab (MB)", Box::new(|p| format!("{}", p["memory_after_grow"]["total_mb"]))),
        ("neighbour round trip after all three (ms)", Box::new(|p| format!("{}", p["neighbour_after"]["ms"]))),
    ];
    for (label, cell) in rows {
        out.push_str(&format!("| {label} |"));
        for (_, probe) in probes {
            out.push_str(&format!(" {} |", cell(probe)));
        }
        out.push('\n');
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
    if matches!(args.get(1).map(String::as_str), Some("run" | "crash")) {
        println!("{}", child(&args, &rt));
        return;
    }
    let reps: usize = flag(&args, "--reps").and_then(|n| n.parse().ok()).unwrap_or(5);
    let out = flag(&args, "--out").map(PathBuf::from);
    let here = std::env::current_exe().expect("own path");
    let bins = here.parent().expect("the target dir").to_path_buf();
    let cache = tempfile::tempdir().expect("a temp dir");
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The tier glue: the SDK's component exports, or the stdio host and a two-line main.
    let glue_wasm = loc_file(&crates.join("../../crates/extension-sdk/src/guest.rs"));
    let glue_program = loc_file(&crates.join("src/program.rs")) + loc_file(&crates.join("src/bin/tier-rss.rs"));
    let mut columns = Vec::new();
    let mut compiled = serde_json::Map::new();
    for ext in [WIDGET, WORKTREES, RSS, CRASH] {
        let wasm = vorn_guest_build::component(ext.package).unwrap_or_else(|e| panic!("{e}"));
        let t = Instant::now();
        let cwasm = vorn_extensions::wasm::precompile(&wasm, cache.path()).unwrap_or_else(|e| panic!("{e}"));
        compiled.insert(ext.id.to_owned(), json!(stats::round(ms(t.elapsed()))));
        let source = match ext.id {
            "bench.widget" => crates.join("../../extensions/widget/src"),
            "bench.worktrees" => crates.join("worktrees/src"),
            "bench.rss" => crates.join("rss/src"),
            _ => crates.join("crash/src"),
        };
        let lines = loc(&source);
        let program = bins.join(ext.bin);
        columns.push(Column {
            ext,
            tier: "wasm",
            sizes: json!({
                "binary_kb": size(&wasm) / 1024,
                "compiled_kb": size(&cwasm) / 1024,
                "loc": format!("{lines} + {glue_wasm}"),
            }),
            path: wasm,
        });
        columns.push(Column {
            ext,
            tier: "program",
            sizes: json!({ "binary_kb": size(&program) / 1024, "loc": format!("{lines} + {glue_program}") }),
            path: program,
        });
        if ext.id == RSS.id {
            if let Some(node) = node::node() {
                let bundle = node::dir().join("index.js");
                let lines = std::fs::read_to_string(&bundle).map_or(0, |t| t.lines().count());
                columns.push(Column {
                    ext,
                    tier: "node",
                    sizes: json!({
                        "binary_kb": size(&bundle) / 1024,
                        "runtime_kb": size(&node) / 1024,
                        "loc": format!("{lines} (bundle)"),
                    }),
                    path: node,
                });
            }
        }
    }
    let (probes, measured): (Vec<&Column>, Vec<&Column>) = columns.iter().partition(|c| c.ext.id == CRASH.id);
    let cache_arg = cache.path().display().to_string();
    let mut runs: Vec<Vec<Value>> = Vec::new();
    for c in &measured {
        let mut these = Vec::with_capacity(reps);
        for rep in 0..reps {
            let path = c.path.display().to_string();
            these.push(spawn(&["run", "--ext", c.ext.id, "--tier", c.tier, "--path", &path, "--cache", &cache_arg]));
            eprintln!("{} ({}) run {}/{reps}", c.ext.name, c.tier, rep + 1);
        }
        runs.push(these);
    }
    let measured: Vec<Column> = measured
        .into_iter()
        .map(|c| Column { ext: c.ext, tier: c.tier, path: c.path.clone(), sizes: c.sizes.clone() })
        .collect();
    let mut crashes = Vec::new();
    for probe in probes {
        let sibling = measured
            .iter()
            .find(|c| c.ext.id == WIDGET.id && c.tier == probe.tier)
            .map(|c| c.path.display().to_string())
            .expect("the Widget in the same tier");
        let path = probe.path.display().to_string();
        let ran = spawn(&["crash", "--tier", probe.tier, "--path", &path, "--sibling", &sibling, "--cache", &cache_arg]);
        eprintln!("crash probe ({}) done", probe.tier);
        crashes.push((probe.tier, ran));
    }
    let md = format!(
        "{}\nCompiling each component once, as on install (ms): {}\n\n{}",
        table(&measured, &runs),
        Value::Object(compiled.clone()),
        crash_table(&crashes)
    );
    println!("{md}");
    if let Some(dir) = out {
        std::fs::create_dir_all(&dir).expect("the out dir");
        let raw: Vec<Value> = measured
            .iter()
            .zip(&runs)
            .map(|(c, r)| json!({ "ext": c.ext.id, "tier": c.tier, "size": c.sizes, "runs": r }))
            .collect();
        let crashes: Value = crashes.into_iter().map(|(t, v)| (t.to_owned(), v)).collect();
        let all = json!({ "columns": raw, "compile_ms": compiled, "crash": crashes });
        std::fs::write(dir.join("results.json"), format!("{all:#}\n")).expect("written");
        std::fs::write(dir.join("summary.md"), md).expect("written");
    }
}

fn loc_file(path: &Path) -> usize {
    std::fs::read_to_string(path).map_or(0, |text| {
        text.lines()
            .take_while(|l| l.trim() != "#[cfg(test)]")
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
            .count()
    })
}
