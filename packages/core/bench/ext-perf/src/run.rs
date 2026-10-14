//! One command for the whole comparison: components precompiled, then every case run as its own process, sides interleaved, after builds go quiet.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{json, Value};

use crate::sys;

fn flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0].strip_prefix("--") == Some(key))
        .map(|w| w[1].as_str())
}

/// One measurement to make: the case's arguments after its variant.
struct Group {
    kind: &'static str,
    variants: &'static [&'static str],
    rest: Vec<String>,
}

fn groups(only: &[String], tiers: &[usize]) -> Vec<Group> {
    let wants = |k: &str| only.is_empty() || only.iter().any(|o| o == k);
    let mut out = Vec::new();
    if wants("micro") {
        for t in tiers {
            out.push(Group {
                kind: "micro",
                variants: &[""],
                rest: vec![t.to_string()],
            });
        }
    }
    if wants("tasks") {
        for t in tiers {
            out.push(Group {
                kind: "tasks",
                variants: &["builtin", "ext", "ext-config"],
                rest: vec![t.to_string()],
            });
        }
    }
    if wants("flows") {
        for steps in [10, 100] {
            for c in [1, 10, 100] {
                out.push(Group {
                    kind: "flows",
                    variants: &["builtin", "native", "ext"],
                    rest: vec![steps.to_string(), c.to_string()],
                });
            }
        }
    }
    if wants("gates") {
        for c in [1, 10, 100] {
            out.push(Group {
                kind: "gates",
                variants: &["builtin", "native", "ext"],
                rest: vec![c.to_string()],
            });
        }
    }
    if wants("schedule") {
        out.push(Group {
            kind: "schedule",
            variants: &["builtin", "native", "ext"],
            rest: Vec::new(),
        });
    }
    if wants("board") {
        for t in tiers {
            out.push(Group {
                kind: "board",
                variants: &["builtin", "kit", "kit-window"],
                rest: vec![t.to_string()],
            });
        }
    }
    out
}

fn compile(wasm: &Path, out: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut sizes = serde_json::Map::new();
    for (from, to) in [
        ("ext_perf_tasks.wasm", "tasks.cwasm"),
        ("ext_perf_workflows.wasm", "workflows.cwasm"),
    ] {
        let bytes = std::fs::read(wasm.join(from)).map_err(|e| format!("{from}: {e}"))?;
        let t = Instant::now();
        let compiled = vorn_ext_perf_host::precompile(&bytes).map_err(|e| e.to_string())?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        std::fs::write(out.join(to), &compiled).map_err(|e| e.to_string())?;
        sizes.insert(from.into(), json!({ "wasm_kb": bytes.len() / 1024, "cwasm_kb": compiled.len() / 1024, "compile_ms": ms }));
    }
    Ok(Value::Object(sizes))
}

/// The cases a resumed run need not repeat: those that ended well with no build seen while they ran.
fn kept(out: &Path) -> Result<Vec<(u64, Vec<String>)>, String> {
    let Ok(text) = std::fs::read_to_string(out.join("raw.jsonl")) else {
        return Ok(Vec::new());
    };
    let mut keep = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if crate::report::invalid(&v) {
            continue;
        }
        if let (Some(rep), Some(argv)) = (v["rep"].as_u64(), case_argv(&v, out)) {
            keep.push((rep, argv));
        }
    }
    Ok(keep)
}

/// The arguments a row was measured with: recorded since resuming existed, rebuilt from its fields before.
fn case_argv(v: &Value, out: &Path) -> Option<Vec<String>> {
    if let Some(a) = v["argv"].as_array() {
        return a.iter().map(|x| x.as_str().map(str::to_owned)).collect();
    }
    let n = |k: &str| v[k].as_u64().map(|x| x.to_string());
    let kind = v["kind"].as_str()?;
    let mut argv = vec!["case".to_owned(), kind.to_owned()];
    match kind {
        "micro" => argv.extend([n("tier")?, out.join("cwasm").display().to_string()]),
        _ => {
            argv.push(v["variant"].as_str()?.to_owned());
            match kind {
                "tasks" | "board" => argv.push(n("tier")?),
                "flows" => argv.extend([n("steps")?, n("concurrent")?]),
                "gates" => argv.push(n("concurrent")?),
                _ => {}
            }
        }
    }
    Some(argv)
}

/// `run`: every group, `reps` times, the variants of a group in turn and their order rotated each time.
pub fn run(args: &[String]) -> Result<(), String> {
    let wasm = PathBuf::from(flag(args, "wasm").ok_or("--wasm is needed")?);
    let bin = PathBuf::from(flag(args, "bin").ok_or("--bin is needed")?);
    let out = PathBuf::from(flag(args, "out").ok_or("--out is needed")?);
    let reps: usize = flag(args, "reps")
        .unwrap_or("5")
        .parse()
        .map_err(|_| "bad --reps")?;
    let only: Vec<String> = flag(args, "only")
        .map(|o| o.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let tiers: Vec<usize> = flag(args, "tiers")
        .unwrap_or("100,1000,10000")
        .split(',')
        .map(|t| t.parse().map_err(|_| format!("bad tier {t:?}")))
        .collect::<Result<_, _>>()?;
    let resume = args.iter().any(|a| a == "--resume");
    let cwasm = out.join("cwasm");
    let kept = if resume { kept(&out)? } else { Vec::new() };
    // A resumed run keeps the components and meta of the run it finishes, so every row measured the same build.
    if !resume || !cwasm.join("workflows.cwasm").exists() {
        let compiled = compile(&wasm, &cwasm)?;
        let meta = json!({ "machine": sys::machine(), "components": compiled, "reps": reps, "tiers": tiers });
        std::fs::write(
            out.join("meta.json"),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut raw = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("raw.jsonl"))
        .map_err(|e| e.to_string())?;
    let plan = groups(&only, &tiers);
    let total: usize = plan.iter().map(|g| g.variants.len()).sum::<usize>() * reps;
    let mut done = 0;
    for rep in 0..reps {
        for g in &plan {
            for k in 0..g.variants.len() {
                let variant = g.variants[(k + rep) % g.variants.len()];
                let waited = sys::wait_quiet();
                let mut argv = vec!["case".to_owned(), g.kind.to_owned()];
                if g.kind == "micro" {
                    argv.extend(g.rest.iter().cloned());
                    argv.push(cwasm.display().to_string());
                } else {
                    argv.push(variant.to_owned());
                    argv.extend(g.rest.iter().cloned());
                }
                done += 1;
                if kept.contains(&(rep as u64, argv.clone())) {
                    continue;
                }
                eprintln!(
                    "ext-perf [{done}/{total}] rep {} {}",
                    rep + 1,
                    argv[1..].join(" ")
                );
                let load = sys::load();
                let watch = sys::BuildWatch::start();
                let t = Instant::now();
                let child = Command::new(&exe)
                    .args(&argv)
                    .env("VORN_EXT_SPIKE_DIR", &cwasm)
                    .env("VORN_APP_TEST_BIN", &bin)
                    .stdin(Stdio::null())
                    .stderr(Stdio::inherit())
                    .output()
                    .map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&child.stdout);
                let mut line = match serde_json::from_str::<Value>(text.trim()) {
                    Ok(v) if child.status.success() => v,
                    _ => {
                        json!({ "kind": g.kind, "variant": variant, "error": format!("exit {:?}", child.status.code()), "args": argv })
                    }
                };
                line["rep"] = json!(rep);
                line["load_before"] = json!(load);
                line["waited_s"] = json!(waited.as_secs_f64());
                line["wall_s"] = json!(t.elapsed().as_secs_f64());
                line["argv"] = json!(argv);
                line["builds_during"] = json!(watch.finish());
                writeln!(raw, "{line}").map_err(|e| e.to_string())?;
            }
        }
    }
    crate::report::write(&out)
}
