//! `vorn-workflow-bench`: the workflow engine in a test vornd under concurrent runs, recorded so a change can be set against what came before.

mod daemon;
mod flows;
mod live;
mod report;
mod stats;
mod sys;

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const USAGE: &str = "\
Usage: vorn-workflow-bench run --out DIR --bin NAME=DIR [--bin NAME=DIR ...] [--reps 3] [--only flows,gates,schedule]
       vorn-workflow-bench report DIR

Each --bin DIR holds a vornd and vorn-sessiond (say before=... and after=...). run takes them in turn case by case,
so they share the machine's load, and appends one row per case to DIR/raw.jsonl tagged with NAME;
report writes DIR/summary.md, a column per NAME.";

/// Runs × steps of the flows case.
const FLOWS: [(usize, usize); 6] = [
    (1, 10),
    (10, 10),
    (100, 10),
    (1, 100),
    (10, 100),
    (100, 100),
];
const GATES: [usize; 3] = [1, 10, 100];

struct Args {
    bins: Vec<(String, PathBuf)>,
    out: PathBuf,
    reps: usize,
    only: Vec<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        bins: Vec::new(),
        out: PathBuf::new(),
        reps: 3,
        only: vec!["flows".into(), "gates".into(), "schedule".into()],
    };
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let v = it.next().ok_or(USAGE)?;
        match k.as_str() {
            "--bin" => {
                let (name, dir) = v.split_once('=').ok_or(USAGE)?;
                a.bins.push((name.to_owned(), PathBuf::from(dir)));
            }
            "--out" => a.out = PathBuf::from(v),
            "--reps" => a.reps = v.parse().map_err(|_| format!("bad --reps {v:?}"))?,
            "--only" => a.only = v.split(',').map(str::to_owned).collect(),
            _ => return Err(USAGE.into()),
        }
    }
    if a.bins.is_empty() || a.out.as_os_str().is_empty() {
        return Err(USAGE.into());
    }
    Ok(a)
}

fn case(bin: &Path, kind: &str, runs: usize, steps: usize) -> Result<Value, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    match kind {
        "flows" => rt.block_on(flows::runs(bin, steps, runs)),
        "gates" => rt.block_on(flows::gates(bin, runs)),
        _ => rt.block_on(flows::schedule(bin)),
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let a = parse(args)?;
    std::fs::create_dir_all(&a.out).map_err(|e| e.to_string())?;
    let meta = a.out.join("meta.json");
    if !meta.exists() {
        let m = json!({ "machine": sys::machine() });
        std::fs::write(&meta, m.to_string()).map_err(|e| e.to_string())?;
    }
    let mut cases: Vec<(&str, usize, usize)> = Vec::new();
    for kind in &a.only {
        match kind.as_str() {
            "flows" => cases.extend(FLOWS.iter().map(|&(r, s)| ("flows", r, s))),
            "gates" => cases.extend(GATES.iter().map(|&r| ("gates", r, 0))),
            "schedule" => cases.push(("schedule", 0, 0)),
            _ => return Err(format!("unknown case {kind:?}")),
        }
    }
    let mut raw = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(a.out.join("raw.jsonl"))
        .map_err(|e| e.to_string())?;
    for rep in 1..=a.reps {
        for &(kind, runs, steps) in &cases {
            // Alternate which build goes first, so neither always meets a warmer machine.
            let mut order: Vec<&(String, PathBuf)> = a.bins.iter().collect();
            if rep % 2 == 0 {
                order.reverse();
            }
            for (label, bin) in order {
                eprintln!("{rep}/{} {label}: {kind} runs={runs} steps={steps}", a.reps);
                let watch = sys::BuildWatch::start();
                let load = sys::load();
                let mut row = case(bin, kind, runs, steps).unwrap_or_else(
                    |e| json!({ "kind": kind, "concurrent": runs, "steps": steps, "error": e }),
                );
                row["label"] = json!(label);
                row["rep"] = json!(rep);
                row["load_before"] = json!(stats::round(load));
                row["builds_during"] = json!(watch.finish());
                if let Some(e) = row.get("error") {
                    eprintln!("  failed: {e}");
                }
                writeln!(raw, "{row}").map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("report") => args
            .get(1)
            .ok_or_else(|| USAGE.to_owned())
            .and_then(|d| report::write(Path::new(d))),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(e) = result {
        eprintln!("vorn-workflow-bench: {e}");
        std::process::exit(1);
    }
}
