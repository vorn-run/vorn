//! `vorn-extc`: the extension compiler, kept out of vornd so vornd links no
//! compiler.
//!
//! - `compile <in.wasm> <out.cwasm>` turns a guest's core module into a
//!   component (no WASI adapter: the guest imports only `vorn:extension`),
//!   refuses any import Vorn does not provide, and precompiles it with the
//!   settings vornd loads with ([`vorn_ext::wasm::config`]).
//! - `pack <crate> [--against <quick-folder>]` builds the crate for wasm32,
//!   compiles it, and replays the recorded checks against every tier.
//! - `record <quick-folder> <crate> --project <dir> [--program gh]` records
//!   the world once, live, from both tiers, and says whether they agreed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_ext::bridge::{Gate, World};
use vorn_ext::checks::{self, Expect, Record, Recording, Replay};
use vorn_ext::live::Live;
use vorn_ext::manifest::{Manifest, Tier};
use vorn_ext::quick::{self, CallCtx, Program, Service};
use vorn_ext::wasm::{self, Instance, Runtime};
use vorn_ext::ExtError;

const DEADLINE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let positional: Vec<&String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in &args {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                out.push(a);
            }
        }
        out
    };
    let result = match positional.as_slice() {
        [cmd, input, output] if *cmd == "compile" => {
            compile(Path::new(input), Path::new(output)).map(|ms| println!("compiled in {ms:.1} ms"))
        }
        [cmd, dir] if *cmd == "pack" => pack(Path::new(dir), flag("--against").as_deref().map(Path::new)),
        [cmd, quick_dir, crate_dir] if *cmd == "record" => match flag("--project") {
            Some(project) => record(
                Path::new(quick_dir),
                Path::new(crate_dir),
                &project,
                &flag("--program").unwrap_or_else(|| "gh".into()),
            ),
            None => Err(ExtError::new("usage", "record needs --project <dir>")),
        },
        _ => Err(ExtError::new(
            "usage",
            "vorn-extc compile <in.wasm> <out.cwasm> | pack <crate> [--against <quick>] | record <quick> <crate> --project <dir> [--program gh]",
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vorn-extc: {}", e.message);
            ExitCode::FAILURE
        }
    }
}

/// Componentizes when needed, checks the imports and precompiles; the time it took.
fn compile(input: &Path, output: &Path) -> Result<f64, ExtError> {
    let io = |e: std::io::Error| ExtError::new("io", format!("{}: {e}", input.display()));
    let bytes = std::fs::read(input).map_err(io)?;
    let t = Instant::now();
    // A core module starts with version 1; a component with the component layer.
    let component = if bytes.get(4..8) == Some(&[1, 0, 0, 0]) {
        wit_component::ComponentEncoder::default()
            .module(&bytes)
            .and_then(|e| e.validate(true).encode())
            .map_err(|e| ExtError::new("componentize", format!("{e:#}")))?
    } else {
        bytes
    };
    let engine =
        wasmtime::Engine::new(&wasm::config()).map_err(|e| ExtError::new("wasm", e.to_string()))?;
    let c = wasmtime::component::Component::new(&engine, &component)
        .map_err(|e| ExtError::new("compile", format!("{e:#}")))?;
    let foreign: Vec<String> = c
        .component_type()
        .imports(&engine)
        .map(|(name, _)| name.to_owned())
        .filter(|n| !n.starts_with("vorn:extension/"))
        .collect();
    if !foreign.is_empty() {
        return Err(ExtError::new(
            "imports",
            format!(
                "the component imports {}, which Vorn does not provide",
                foreign.join(", ")
            ),
        ));
    }
    let code = engine
        .precompile_component(&component)
        .map_err(|e| ExtError::new("compile", format!("{e:#}")))?;
    let ms = t.elapsed().as_secs_f64() * 1e3;
    std::fs::write(output, code).map_err(io)?;
    std::fs::write(output.with_extension("wasm"), &component).map_err(io)?;
    Ok(ms)
}

/// Builds the crate's component and replays its checks against every tier.
fn pack(dir: &Path, against: Option<&Path>) -> Result<(), ExtError> {
    let m = Manifest::load(dir)?;
    let component =
        m.runtime.component.clone().ok_or_else(|| {
            ExtError::new("manifest", "runtime.component is not set; promote first")
        })?;
    let name = m.id.rsplit('.').next().unwrap_or(&m.id).replace('-', "_");
    let t = Instant::now();
    let built = vorn_spawn::command("cargo")
        .args([
            "rustc",
            "--quiet",
            "--lib",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--crate-type",
            "cdylib",
        ])
        .current_dir(dir)
        .status()
        .map_err(|e| ExtError::new("build", format!("cannot run cargo: {e}")))?;
    if !built.success() {
        return Err(ExtError::new(
            "build",
            "cargo could not build the component",
        ));
    }
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let wasm_path = dir.join(format!("target/wasm32-unknown-unknown/release/{name}.wasm"));
    let out = dir.join(&component);
    let compile_ms = compile(&wasm_path, &out)?;
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    println!(
        "built {} ({} KB component, {} KB precompiled) in {build_ms:.0} ms + {compile_ms:.0} ms",
        out.display(),
        size(&out.with_extension("wasm")) / 1024,
        size(&out) / 1024
    );

    let rec_path = dir.join(m.checks.as_deref().unwrap_or("checks/recorded.json"));
    let rec = Recording::load(&rec_path)?;
    let mut tiers: Vec<(&str, PathBuf, Tier)> = Vec::new();
    if let Some(q) = against {
        tiers.push(("script", q.to_owned(), Tier::Quick));
    }
    tiers.push(("native", dir.to_owned(), Tier::Quick));
    tiers.push(("component", dir.to_owned(), Tier::Safe));
    let mut failed = false;
    println!("{:<10} {:<8} {:>10}  result", "tier", "call", "time");
    for (label, folder, tier) in tiers {
        let answers = replay(&folder, tier, &rec);
        for (expect, (answer, took)) in rec.calls.iter().zip(answers) {
            let verdict = match answer.and_then(|a| {
                checks::same_rows(&expect.answer, &a).map_err(|e| ExtError::new("differs", e))
            }) {
                Ok(()) => "same rows".to_owned(),
                Err(e) => {
                    failed = true;
                    format!("FAILED: {}", e.message)
                }
            };
            println!(
                "{label:<10} {:<8} {:>8.1}ms  {verdict}",
                expect.method,
                took.as_secs_f64() * 1e3
            );
        }
    }
    if failed {
        return Err(ExtError::new("checks", "the tiers do not answer the same"));
    }
    println!("every tier answers the recorded checks with the same rows");
    Ok(())
}

/// Every recorded call against one tier, replayed.
fn replay(dir: &Path, tier: Tier, rec: &Recording) -> Vec<(Result<Value, ExtError>, Duration)> {
    let m = match Manifest::load(dir) {
        Ok(m) => Arc::new(m),
        Err(e) => {
            return rec
                .calls
                .iter()
                .map(|_| (Err(e.clone()), Duration::ZERO))
                .collect()
        }
    };
    let shims = tempdir("shims");
    let project = tempdir("project");
    if let Err(e) = checks::program_shims(rec, &shims) {
        return rec
            .calls
            .iter()
            .map(|_| (Err(e.clone()), Duration::ZERO))
            .collect();
    }
    let ctx = CallCtx {
        project: project.display().to_string(),
        caller: "pack".into(),
    };
    let world: Arc<dyn World> = Arc::new(Replay(rec.clone()));
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    run_calls(dir, tier, &m, world, &ctx, &path, &rec.calls)
}

fn run_calls(
    dir: &Path,
    tier: Tier,
    m: &Arc<Manifest>,
    world: Arc<dyn World>,
    ctx: &CallCtx,
    path: &str,
    calls: &[Expect],
) -> Vec<(Result<Value, ExtError>, Duration)> {
    let timed = |f: &mut dyn FnMut() -> Result<Value, ExtError>| {
        let t = Instant::now();
        let r = f();
        (r, t.elapsed())
    };
    match tier {
        Tier::Safe => {
            let inst = Runtime::new().and_then(|rt| {
                let c = rt.load(&dir.join(m.runtime.component.as_deref().unwrap_or_default()))?;
                Instance::new(rt, c, Arc::clone(m), Arc::clone(&world))
            });
            match inst {
                Ok(mut i) => calls
                    .iter()
                    .map(|c| timed(&mut || i.call(&c.method, &c.args, ctx, wasm::CALL_DEADLINE)))
                    .collect(),
                Err(e) => calls
                    .iter()
                    .map(|_| (Err(e.clone()), Duration::ZERO))
                    .collect(),
            }
        }
        Tier::Quick => {
            let program = Program {
                argv: m.runtime.command.clone().unwrap_or_default(),
                dir: dir.to_owned(),
                env: vec![("PATH".into(), path.to_owned())],
            };
            let gate = Gate {
                manifest: m,
                world: world.as_ref(),
                project: &ctx.project,
            };
            if m.runtime.service {
                let mut service = match Service::start(&program, ctx) {
                    Ok(s) => s,
                    Err(e) => {
                        return calls
                            .iter()
                            .map(|_| (Err(e.clone()), Duration::ZERO))
                            .collect()
                    }
                };
                calls
                    .iter()
                    .map(|c| timed(&mut || service.call(&c.method, &c.args, ctx, &gate, DEADLINE)))
                    .collect()
            } else {
                calls
                    .iter()
                    .map(|c| {
                        timed(&mut || quick::one_shot(&program, &c.method, &c.args, ctx, DEADLINE))
                    })
                    .collect()
            }
        }
    }
}

/// Runs both tiers live, once, and keeps what the world answered each.
fn record(
    quick_dir: &Path,
    crate_dir: &Path,
    project: &str,
    program: &str,
) -> Result<(), ExtError> {
    let real = which(program)
        .ok_or_else(|| ExtError::new("record", format!("{program} is not on PATH")))?;
    let shims = tempdir("record");
    checks::recording_shims(&[(program.to_owned(), real)], &shims)?;
    let ctx = CallCtx {
        project: project.to_owned(),
        caller: "record".into(),
    };
    let calls = vec![Expect {
        method: "rows".into(),
        args: json!({}),
        answer: Value::Null,
    }];
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let qm = Arc::new(Manifest::load(quick_dir)?);
    let live: Arc<dyn World> = Arc::new(Live::new(None, std::env::var("PATH").ok()));
    let (quick_answer, quick_took) = run_calls(
        quick_dir,
        Tier::Quick,
        &qm,
        Arc::clone(&live),
        &ctx,
        &path,
        &calls,
    )
    .pop()
    .expect("one call");
    let quick_answer = quick_answer?;

    let cm = Arc::new(Manifest::load(crate_dir)?);
    let tier = cm.tier(crate_dir, false);
    let recorder = Arc::new(RecordOwned(
        Live::new(None, std::env::var("PATH").ok()),
        Default::default(),
        Default::default(),
    ));
    let world: Arc<dyn World> = Arc::clone(&recorder) as Arc<dyn World>;
    let (safe_answer, safe_took) = run_calls(crate_dir, tier, &cm, world, &ctx, &path, &calls)
        .pop()
        .expect("one call");
    let safe_answer = safe_answer?;
    println!(
        "script: {} rows in {:.0} ms",
        quick_answer.as_array().map_or(0, Vec::len),
        quick_took.as_secs_f64() * 1e3
    );
    println!(
        "{tier:?}: {} rows in {:.0} ms",
        safe_answer.as_array().map_or(0, Vec::len),
        safe_took.as_secs_f64() * 1e3
    );
    match checks::same_rows(&quick_answer, &safe_answer) {
        Ok(()) => println!("live: both tiers answered the same rows"),
        Err(e) => println!(
            "live: the tiers differ ({e}); the world may have changed between the two runs"
        ),
    }
    let rec = Recording {
        recorded_at: now(),
        core: recorder.2.lock().map(|c| c.clone()).unwrap_or_default(),
        programs: checks::collect_programs(&shims),
        http: recorder.1.lock().map(|h| h.clone()).unwrap_or_default(),
        calls: vec![Expect {
            method: "rows".into(),
            args: json!({}),
            answer: quick_answer,
        }],
    };
    for dir in [quick_dir, crate_dir] {
        let p = dir.join(cm.checks.as_deref().unwrap_or("checks/recorded.json"));
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(dir));
        rec.save(&p)?;
        println!("recorded {}", p.display());
    }
    Ok(())
}

/// [`Record`] that owns what it wraps, so an instance can hold it.
struct RecordOwned(
    Live,
    std::sync::Mutex<Vec<checks::Exchange>>,
    std::sync::Mutex<std::collections::BTreeMap<String, Value>>,
);

impl World for RecordOwned {
    fn fetch(
        &self,
        req: &vorn_ext::bridge::FetchRequest,
    ) -> Result<vorn_ext::bridge::FetchResponse, ExtError> {
        let r = Record {
            inner: &self.0,
            http: Default::default(),
            core: Default::default(),
        };
        let res = r.fetch(req)?;
        if let (Ok(mut mine), Ok(theirs)) = (self.1.lock(), r.http.into_inner()) {
            mine.extend(theirs);
        }
        Ok(res)
    }

    fn call(&self, method: &str, args: &Value, project: &str) -> Result<Value, ExtError> {
        let v = self.0.call(method, args, project)?;
        if let Ok(mut c) = self.2.lock() {
            c.insert(method.to_owned(), v.clone());
        }
        Ok(v)
    }

    fn credential(&self, ext: &Manifest) -> Result<Option<String>, ExtError> {
        self.0.credential(ext)
    }
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(program))
            .find(|p| p.is_file())
    })
}

fn tempdir(what: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("vorn-extc-{what}-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn now() -> String {
    let out = vorn_spawn::command("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}
