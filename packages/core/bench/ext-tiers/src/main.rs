//! E9.4: Worktree cleanup and the Widget as built-ins, programs and Wasm
//! components, measured side by side. See `README.md` for what it measures.
//!
//! `ext-tiers all --program P --component C --out DIR` runs every tier of
//! both extensions, each in a fresh process of its own so memory reads
//! clean, and writes `results.json` and `results.md`. `ext-tiers run ..` is
//! one of those processes; `ext-tiers precompile C` compiles a component
//! once, ahead of the runs.

mod scenario;
mod stats;
mod sys;
mod tier;
mod world;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tier::{Ext, Kind, Paths, Tier, WIDGET, WORKTREES};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn me() -> u32 {
    std::process::id()
}

/// The extra memory a tier holds: its program's, or what it added to this process.
fn held(tier: &Tier, base: f64) -> Value {
    let own = sys::memory(me()).unwrap_or(0.0) - base;
    let child = tier.pid().and_then(sys::memory);
    #[allow(clippy::cast_precision_loss)]
    let linear = tier
        .linear_memory()
        .map(|b| stats::round(b as f64 / (1024.0 * 1024.0)));
    json!({
        "host_delta_mb": stats::round(own),
        "program_mb": child.map(stats::round),
        "linear_mb": linear,
        "total_mb": stats::round(own.max(0.0) + child.unwrap_or(0.0)),
    })
}

/// CPU seconds of this process and the tier's program.
fn cpu(tier: &Tier) -> f64 {
    sys::cpu(me()).unwrap_or(0.0) + tier.pid().and_then(sys::cpu).unwrap_or(0.0)
}

/// Starts it, as the first call or event would, and answers once it answers.
async fn first_answer(tier: &Tier) -> Duration {
    let t = Instant::now();
    tier.start().await.expect("started");
    tier.call("bench.echo", json!({})).await.expect("echo");
    t.elapsed()
}

/// Stopped, then one event: the time until its first publish. A built-in
/// does not stop, so its start is counted with it.
async fn first_publish(tier: &Tier, kind: Kind, name: &str, data: &Value) -> Duration {
    tier.stop().await;
    let before = tier.world.published.load(Ordering::SeqCst);
    let t = Instant::now();
    if kind == Kind::Builtin {
        tier.start().await.expect("started");
    }
    tier.emit(name, data).await;
    tier.published_after(before).await;
    t.elapsed()
}

/// `count` events from `make`, then the time until every one was handled, and the CPU it took.
async fn burst(tier: &Tier, count: usize, make: impl Fn(usize) -> (&'static str, Value)) -> Value {
    let (cpu0, calls0, pub0) = (
        cpu(tier),
        tier.world.calls.load(Ordering::Relaxed),
        tier.world.published.load(Ordering::Relaxed),
    );
    let t = Instant::now();
    let mut retries = 0;
    for i in 0..count {
        let (name, data) = make(i);
        retries += tier.emit(name, &data).await - 1;
    }
    tier.settled().await;
    let took = t.elapsed();
    let spent = cpu(tier) - cpu0;
    #[allow(clippy::cast_precision_loss)]
    let n = count as f64;
    json!({
        "events": count,
        "ms": stats::round(ms(took)),
        "events_per_s": stats::round(n / took.as_secs_f64()),
        "cpu_us_per_event": stats::round(spent * 1e6 / n),
        "core_calls": tier.world.calls.load(Ordering::Relaxed) - calls0,
        "publishes": tier.world.published.load(Ordering::Relaxed) - pub0,
        "full_queue_retries": retries,
    })
}

async fn perf(kind: Kind, ext: Ext, paths: &Paths) -> Value {
    let tier = Tier::new(kind, ext, paths, Duration::from_secs(60));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let base = sys::memory(me()).unwrap_or(0.0);
    let cold_first = first_answer(&tier).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let idle = held(&tier, base);
    let mut cold = Vec::new();
    for _ in 0..15 {
        tier.stop().await;
        cold.push(ms(first_answer(&tier).await));
    }
    let mut to_publish = Vec::new();
    for n in 0..15 {
        let (name, data) = if ext == WORKTREES {
            let path = "/p/web/.worktrees/w1";
            (
                "worktree.idle",
                json!({ "worktreePath": path, "projectPath": "/p/web" }),
            )
        } else {
            (
                "agent.permission",
                scenario::request(&format!("cold{n}"), n % 32, 0),
            )
        };
        to_publish.push(ms(first_publish(&tier, kind, name, &data).await));
    }
    tier.start().await.expect("started");
    let mut echo = Vec::new();
    for i in 0..2000 {
        let t = Instant::now();
        tier.call("bench.echo", json!({ "n": i }))
            .await
            .expect("echo");
        echo.push(us(t.elapsed()));
    }
    let mut out = json!({
        "tier": kind.name(),
        "ext": ext.short(),
        "cold_first_ms": stats::round(ms(cold_first)),
        "cold_ms": stats::summary(&cold),
        "cold_event_to_publish_ms": stats::summary(&to_publish),
        "echo_us": stats::summary(&echo),
        "idle": idle,
    });
    if ext == WORKTREES {
        let mut scan = Vec::new();
        for _ in 0..300 {
            let t = Instant::now();
            tier.call("scan", json!({})).await.expect("scan");
            scan.push(us(t.elapsed()));
        }
        out["scan_us"] = stats::summary(&scan);
        out["idle_events"] = burst(&tier, 5000, |i| {
            let project = if i % 2 == 0 { "/p/api" } else { "/p/web" };
            let w = (i % 20) | 1;
            let path = format!("{project}/.worktrees/w{w}");
            (
                "worktree.idle",
                json!({ "worktreePath": path, "projectPath": project }),
            )
        })
        .await;
    } else {
        let mut to_row = Vec::new();
        for n in 0..300 {
            let id = format!("lat{n}");
            let t = Instant::now();
            tier.emit("agent.permission", &scenario::request(&id, n % 32, 2))
                .await;
            tier.row_appears("permissions", &id).await;
            to_row.push(us(t.elapsed()));
        }
        out["event_to_row_us"] = stats::summary(&to_row);
        out["status_events"] = burst(&tier, 20_000, |i| {
            let status = if i % 7 == 0 { "waiting" } else { "working" };
            (
                "session.status",
                json!({ "sessionId": world::session_id(i % world::TERMINALS), "status": status }),
            )
        })
        .await;
        out["permission_events"] = burst(&tier, 5000, |i| {
            (
                "agent.permission",
                scenario::request(&format!("b{i}"), i % world::TERMINALS, i),
            )
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    out["busy"] = held(&tier, base);
    tier.stop().await;
    out
}

async fn crash(kind: Kind, paths: &Paths) -> Value {
    let tier = Tier::new(kind, WIDGET, paths, Duration::from_secs(2));
    tier.start().await.expect("started");
    let code = |r: Result<Value, vorn_extensions::api::ExtError>| match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": e.code, "message": e.message }),
    };
    let t = Instant::now();
    let spin = code(tier.call("bench.spin", json!({})).await);
    let spin_ms = ms(t.elapsed());
    let t = Instant::now();
    let after_spin = code(tier.call("bench.echo", json!({})).await);
    let restart_ms = ms(t.elapsed());
    let grow = code(tier.call("bench.grow", json!({ "mb": 256 })).await);
    let after_grow = code(tier.call("bench.echo", json!({})).await);
    let mut crashes = Vec::new();
    for _ in 0..6 {
        let crashed = code(tier.call("bench.crash", json!({})).await);
        let t = Instant::now();
        let next = code(tier.call("bench.echo", json!({})).await);
        crashes.push(
            json!({ "crash": crashed, "next": next, "next_ms": stats::round(ms(t.elapsed())) }),
        );
    }
    tier.stop().await;
    json!({
        "tier": kind.name(),
        "spin": spin, "spin_ms": stats::round(spin_ms),
        "after_spin": after_spin, "restart_ms": stats::round(restart_ms),
        "grow_256mb": grow, "after_grow": after_grow,
        "crashes": crashes,
        "host_alive": true,
    })
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn paths(args: &[String]) -> Paths {
    Paths {
        program: flag(args, "--program")
            .map(PathBuf::from)
            .unwrap_or_default(),
        component: flag(args, "--component")
            .map(PathBuf::from)
            .unwrap_or_default(),
    }
}

/// One measurement in this process: `run perf|scenario|crash --tier T --ext E`.
async fn run(args: &[String]) -> Value {
    let mode = args.get(2).map_or("perf", String::as_str);
    let kind = flag(args, "--tier")
        .and_then(|t| Kind::parse(&t))
        .expect("--tier builtin|program|wasm");
    let ext = flag(args, "--ext")
        .and_then(|e| Ext::parse(&e))
        .unwrap_or(WIDGET);
    let paths = paths(args);
    match mode {
        "scenario" => {
            let tier = Tier::new(kind, ext, &paths, Duration::from_secs(60));
            let log = if ext == WIDGET {
                scenario::widget(&tier).await
            } else {
                scenario::worktrees(&tier).await
            };
            tier.stop().await;
            Value::Array(log)
        }
        "crash" => crash(kind, &paths).await,
        _ => perf(kind, ext, &paths).await,
    }
}

/// Runs `run ..` in a fresh process and reads what it printed.
fn child(args: &[&str], paths: &Paths) -> Value {
    let exe = std::env::current_exe().expect("own path");
    let out = Command::new(exe)
        .arg("run")
        .args(args)
        .arg("--program")
        .arg(&paths.program)
        .arg("--component")
        .arg(&paths.component)
        .output()
        .expect("a run");
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(
        |_| json!({ "failed": String::from_utf8_lossy(&out.stderr).trim().to_owned() }),
    )
}

fn size(path: &Path) -> Value {
    std::fs::metadata(path).map_or(Value::Null, |m| json!(m.len()))
}

/// Lines of code: neither blank nor a comment, before any `mod tests`.
fn loc(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .take_while(|l| !l.starts_with("#[cfg(test)]"))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .count()
}

fn lines() -> Value {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let core = here.join("../..");
    let builtin = Path::new(tier::BUILTIN);
    json!({
        "builtin_worktrees": loc(&builtin.join("worktrees.rs")),
        "builtin_widget": loc(&builtin.join("widget.rs")),
        "ported_worktrees": loc(&here.join("logic/src/worktrees.rs")),
        "ported_widget": loc(&here.join("logic/src/widget.rs")),
        "shared_logic": loc(&here.join("logic/src/lib.rs")),
        "program_glue": loc(&here.join("program/src/main.rs")),
        "component_glue": loc(&here.join("component/src/lib.rs")),
        "host_wasm": loc(&core.join("crates/extensions/src/wasm.rs")),
    })
}

/// The first step at which two scenario logs differ, if any.
fn differs(a: &Value, b: &Value) -> Option<Value> {
    let (a, b) = (a.as_array()?, b.as_array()?);
    if a.len() != b.len() {
        return Some(json!({ "steps": [a.len(), b.len()] }));
    }
    a.iter()
        .zip(b)
        .find(|(x, y)| x != y)
        .map(|(x, y)| json!({ "left": x, "right": y }))
}

/// A log with each collection's rows in id order: what the rows are, whatever order they came in.
fn unordered(log: &Value) -> Value {
    let mut log = log.clone();
    for step in log.as_array_mut().into_iter().flatten() {
        for rows in step["rows"]
            .as_object_mut()
            .into_iter()
            .flat_map(|m| m.values_mut())
        {
            if let Some(rows) = rows.as_array_mut() {
                rows.sort_by(|x, y| x["id"].to_string().cmp(&y["id"].to_string()));
            }
        }
    }
    log
}

/// How `runs` plays of the scenario under `kind` compare with the built-in's rows.
fn against(builtin: &Value, kind: Kind, ext: &str, runs: usize, paths: &Paths) -> (Value, Value) {
    let (mut same_rows, mut same_order) = (0, 0);
    let mut first_difference = Value::Null;
    let mut last = Value::Null;
    for _ in 0..runs {
        eprintln!("scenario {} {ext}", kind.name());
        let log = child(&["scenario", "--tier", kind.name(), "--ext", ext], paths);
        if differs(builtin, &log).is_none() {
            same_order += 1;
        }
        match differs(&unordered(builtin), &unordered(&log)) {
            None => same_rows += 1,
            Some(d) => first_difference = d,
        }
        last = log;
    }
    let report = json!({
        "runs": runs,
        "same_rows": same_rows,
        "same_order": same_order,
        "first_difference": first_difference,
    });
    (report, last)
}

fn all(args: &[String]) -> Value {
    let paths = paths(args);
    let runs: usize = flag(args, "--runs")
        .and_then(|r| r.parse().ok())
        .unwrap_or(3);
    let kinds = [Kind::Builtin, Kind::Program, Kind::Wasm];
    let mut perf = Vec::new();
    let mut same = serde_json::Map::new();
    let mut fixture = Value::Null;
    for ext in ["worktrees", "widget"] {
        for kind in kinds {
            for _ in 0..runs {
                eprintln!("perf {} {ext}", kind.name());
                perf.push(child(
                    &["perf", "--tier", kind.name(), "--ext", ext],
                    &paths,
                ));
            }
        }
        eprintln!("scenario builtin {ext}");
        let builtin = child(&["scenario", "--tier", "builtin", "--ext", ext], &paths);
        let (program, _) = against(&builtin, Kind::Program, ext, runs, &paths);
        let (wasm, last) = against(&builtin, Kind::Wasm, ext, runs, &paths);
        same.insert(
            ext.to_owned(),
            json!({ "steps": builtin.as_array().map_or(0, Vec::len), "program": program, "wasm": wasm }),
        );
        if ext == "widget" {
            fixture = last
                .as_array()
                .and_then(|l| l.iter().find(|s| s["step"] == "asking"))
                .map(|s| s["rows"].clone())
                .unwrap_or_default();
        }
    }
    let crashes: Vec<Value> = [Kind::Program, Kind::Wasm]
        .iter()
        .map(|k| {
            eprintln!("crash {}", k.name());
            child(&["crash", "--tier", k.name()], &paths)
        })
        .collect();
    let cwasm = tier::precompiled(&paths.component);
    let sizes = json!({
        "program": size(&paths.program),
        "component_wasm": size(&paths.component),
        "component_cwasm": size(&cwasm),
        "probe_without_wasm": flag(args, "--probe-off").map_or(Value::Null, |p| size(Path::new(&p))),
        "probe_with_wasm": flag(args, "--probe-on").map_or(Value::Null, |p| size(Path::new(&p))),
        "probe_with_compiler": flag(args, "--probe-compile").map_or(Value::Null, |p| size(Path::new(&p))),
    });
    json!({
        "machine": machine(),
        "runs": runs,
        "perf": perf,
        "same_rows": same,
        "crash": crashes,
        "sizes": sizes,
        "loc": lines(),
        "fixture": fixture,
    })
}

fn machine() -> Value {
    let sysctl = |k: &str| {
        Command::new("sysctl")
            .args(["-n", k])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    json!({
        "cpu": sysctl("machdep.cpu.brand_string"),
        "cores": std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        "os": std::env::consts::OS,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("precompile") => {
            let wasm = PathBuf::from(args.get(2).expect("precompile <component.wasm>"));
            println!("{}", tier::precompiled(&wasm).display());
        }
        Some("run") => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("runtime");
            let out = rt.block_on(run(&args));
            println!("{out}");
            // A program or component thread may still be ending; it is not waited for.
            std::process::exit(0);
        }
        Some("all") => {
            let out = all(&args);
            let dir = PathBuf::from(flag(&args, "--out").unwrap_or_else(|| ".".into()));
            std::fs::create_dir_all(&dir).expect("out dir");
            let text = serde_json::to_string_pretty(&out).expect("json");
            std::fs::write(dir.join("results.json"), &text).expect("results");
            if let Some(f) = flag(&args, "--fixture") {
                let rows = serde_json::to_string_pretty(&out["fixture"]).expect("json");
                std::fs::write(f, rows + "\n").expect("fixture");
            }
            println!("{}", dir.join("results.json").display());
        }
        _ => {
            eprintln!("ext-tiers all|run|precompile ..");
            std::process::exit(2);
        }
    }
}
