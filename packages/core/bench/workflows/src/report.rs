//! The tables: a column per build (before, after), each cell the median of the repeated runs with their lowest and highest.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::stats::Spread;

/// Rows by case, then by label.
type Groups = BTreeMap<(String, u64, u64), BTreeMap<String, Vec<Value>>>;

fn spread(runs: &[Value], pointer: &str) -> Option<Spread> {
    let vals: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.pointer(pointer).and_then(Value::as_f64))
        .collect();
    Spread::of(&vals)
}

fn fmt(x: f64) -> String {
    if x >= 100.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

fn cell(s: Option<Spread>) -> String {
    s.map_or("–".into(), |s| {
        format!("{} ({}–{})", fmt(s.median), fmt(s.low), fmt(s.high))
    })
}

fn table(
    out: &mut String,
    title: &str,
    runs: &BTreeMap<String, Vec<Value>>,
    labels: &[String],
    rows: &[(&str, &str)],
) {
    let _ = writeln!(out, "\n**{title}**\n");
    let mut head = String::from("| Metric |");
    let mut rule = String::from("| --- |");
    for l in labels {
        let _ = write!(head, " {l} |");
        rule.push_str(" ---: |");
    }
    let ratio = labels.len() == 2;
    if ratio {
        let _ = write!(head, " {} / {} |", labels[1], labels[0]);
        rule.push_str(" ---: |");
    }
    let _ = writeln!(out, "{head}\n{rule}");
    let empty = Vec::new();
    for (label, pointer) in rows {
        let mut line = format!("| {label} |");
        let cells: Vec<Option<Spread>> = labels
            .iter()
            .map(|l| spread(runs.get(l).unwrap_or(&empty), pointer))
            .collect();
        for c in &cells {
            let _ = write!(line, " {} |", cell(*c));
        }
        if ratio {
            let r = match (cells[0], cells[1]) {
                (Some(a), Some(b)) if a.median > 0.0 => format!("×{:.2}", b.median / a.median),
                _ => "–".into(),
            };
            let _ = write!(line, " {r} |");
        }
        let _ = writeln!(out, "{line}");
    }
}

/// Writes `summary.md` beside the raw results and prints it.
pub fn write(dir: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(dir.join("raw.jsonl")).map_err(|e| e.to_string())?;
    let mut groups = Groups::new();
    let mut labels: Vec<String> = Vec::new();
    let mut left_out = 0;
    let mut loads = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let label = v["label"].as_str().unwrap_or("").to_owned();
        loads.extend(v["load_before"].as_f64());
        if !labels.contains(&label) {
            labels.push(label.clone());
        }
        if v.get("error").is_some() {
            left_out += 1;
            continue;
        }
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        let key = (
            v["kind"].as_str().unwrap_or("").to_owned(),
            n("concurrent"),
            n("steps"),
        );
        groups
            .entry(key)
            .or_default()
            .entry(label)
            .or_default()
            .push(v);
    }
    loads.sort_by(f64::total_cmp);
    let meta: Value = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let mut out = String::from("# Workflow engine under load\n\n");
    let _ = writeln!(
        out,
        "Machine: {}. Cells: median of the repeated runs (lowest–highest). Failed rows left out: {left_out}. Load average before each case: {} (median); builds by other users may overlap, so the labels take turns case by case.",
        meta["machine"],
        fmt(loads.get(loads.len() / 2).copied().unwrap_or(0.0))
    );
    for ((kind, runs, steps), by) in &groups {
        match kind.as_str() {
            "flows" => table(
                &mut out,
                &format!("{runs} concurrent runs × {steps} steps"),
                by,
                &labels,
                &[
                    ("Trigger → first step p50 (ms)", "/trigger_to_first_ms/p50"),
                    ("Trigger → first step p99 (ms)", "/trigger_to_first_ms/p99"),
                    ("First step seen by a viewer p99 (ms)", "/seen_first_ms/p99"),
                    ("Per step p50 (µs)", "/per_step_us/p50"),
                    ("vornd CPU per step (µs)", "/cpu_us_per_step/p50"),
                    ("Steps/s", "/steps_per_s/p50"),
                    ("Peak memory per run (KB)", "/peak_kb_per_run/p50"),
                    ("Run broadcasts", "/run_notes"),
                    ("Runs that did not succeed", "/failed_runs"),
                ],
            ),
            "gates" => table(
                &mut out,
                &format!("Gates, {runs} concurrent runs"),
                by,
                &labels,
                &[
                    (
                        "Trigger → waiting at the gate p99 (ms)",
                        "/trigger_to_gate_ms/p99",
                    ),
                    ("Approve answered p50 (ms)", "/answer_ms/p50"),
                    ("Approve → next step p50 (ms)", "/answer_to_resume_ms/p50"),
                    ("Approve → next step p99 (ms)", "/answer_to_resume_ms/p99"),
                    ("Memory per parked run (KB)", "/parked_kb_per_run/p50"),
                ],
            ),
            _ => table(
                &mut out,
                "Schedules: 12 workflows due two seconds apart",
                by,
                &labels,
                &[
                    ("Fired", "/fired"),
                    ("Missed", "/missed"),
                    ("Late p50 (ms)", "/late_ms/p50"),
                    ("Late max (ms)", "/late_ms/max"),
                ],
            ),
        }
    }
    std::fs::write(dir.join("summary.md"), &out).map_err(|e| e.to_string())?;
    print!("{out}");
    Ok(())
}
