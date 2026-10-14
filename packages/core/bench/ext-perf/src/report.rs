//! The tables: each cell the median of the repeated runs with their lowest and highest, and the decision rule checked.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::stats::Spread;

/// Results by group (kind and its arguments), then by variant.
type Groups = BTreeMap<String, BTreeMap<String, Vec<Value>>>;

fn group_key(v: &Value) -> String {
    let n = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .map_or(String::new(), |x| format!("{x:>6}"))
    };
    format!(
        "{}|{}|{}|{}",
        v["kind"].as_str().unwrap_or(""),
        n("tier"),
        n("steps"),
        n("concurrent")
    )
}

/// A row the tables leave out: its case failed, or a build ran while it was timed.
pub fn invalid(v: &Value) -> bool {
    v.get("error").is_some() || v["builds_during"].as_array().is_some_and(|b| !b.is_empty())
}

fn load(dir: &Path) -> Result<(Groups, usize), String> {
    let text = std::fs::read_to_string(dir.join("raw.jsonl")).map_err(|e| e.to_string())?;
    let mut groups = Groups::new();
    let mut failed = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if invalid(&v) {
            failed += 1;
            continue;
        }
        let variant = v["variant"].as_str().unwrap_or("").to_owned();
        groups
            .entry(group_key(&v))
            .or_default()
            .entry(variant)
            .or_default()
            .push(v);
    }
    Ok((groups, failed))
}

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

fn ratio(a: Option<Spread>, b: Option<Spread>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) if b.median > 0.0 => Some(a.median / b.median),
        _ => None,
    }
}

/// One comparison table: a row per metric, a column per variant, and the extension against the built-in.
fn table(
    out: &mut String,
    title: &str,
    runs: &BTreeMap<String, Vec<Value>>,
    variants: &[&str],
    rows: &[(&str, &str)],
    vs: Option<(&str, &str)>,
) {
    let _ = writeln!(out, "\n**{title}**\n");
    let mut head = String::from("| Metric |");
    let mut rule = String::from("| --- |");
    for v in variants {
        let _ = write!(head, " {v} |");
        rule.push_str(" ---: |");
    }
    if let Some((a, b)) = vs {
        let _ = write!(head, " {a} / {b} |");
        rule.push_str(" ---: |");
    }
    let _ = writeln!(out, "{head}\n{rule}");
    let empty = Vec::new();
    for (label, pointer) in rows {
        let mut line = format!("| {label} |");
        for v in variants {
            let _ = write!(
                line,
                " {} |",
                cell(spread(runs.get(*v).unwrap_or(&empty), pointer))
            );
        }
        if let Some((a, b)) = vs {
            let r = ratio(
                spread(runs.get(a).unwrap_or(&empty), pointer),
                spread(runs.get(b).unwrap_or(&empty), pointer),
            );
            let _ = write!(line, " {} |", r.map_or("–".into(), |r| format!("×{r:.2}")));
        }
        let _ = writeln!(out, "{line}");
    }
}

/// The decision rule's checks: the worst extension-to-built-in ratio of each kind of measure.
struct Rule {
    lines: Vec<(String, f64)>,
}

impl Rule {
    fn check(
        &mut self,
        what: String,
        runs: &BTreeMap<String, Vec<Value>>,
        ext: &str,
        builtin: &str,
        pointer: &str,
    ) {
        let empty = Vec::new();
        if let Some(r) = ratio(
            spread(runs.get(ext).unwrap_or(&empty), pointer),
            spread(runs.get(builtin).unwrap_or(&empty), pointer),
        ) {
            self.lines.push((what, r));
        }
    }
}

/// Writes `summary.md` beside the raw results and prints it.
pub fn write(dir: &Path) -> Result<(), String> {
    let (groups, failed) = load(dir)?;
    let meta: Value = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let mut out = String::new();
    let _ = writeln!(out, "# Tasks and Workflows as extensions: measurements\n");
    let _ = writeln!(
        out,
        "Machine: {}. Cells: median of the repeated runs (lowest–highest). Rows left out (failed, or a build ran during them): {failed}.",
        meta["machine"]
    );
    let _ = writeln!(out, "Components: {}", meta["components"]);
    let mut rule = Rule { lines: Vec::new() };
    for (key, runs) in &groups {
        let mut parts = key.split('|');
        let kind = parts.next().unwrap_or("");
        let (tier, steps, conc) = (
            parts.next().unwrap_or("").trim(),
            parts.next().unwrap_or("").trim(),
            parts.next().unwrap_or("").trim(),
        );
        match kind {
            "micro" => {
                let mut by: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for r in runs.values().flatten() {
                    for t in r["tasks"].as_array().into_iter().flatten() {
                        by.entry(t["path"].as_str().unwrap_or("").to_owned())
                            .or_default()
                            .push(t.clone());
                    }
                }
                table(
                    &mut out,
                    &format!(
                        "In process, Tasks calls at {tier} tasks (µs, p50 and p99 of each round)"
                    ),
                    &by,
                    &["builtin", "native-json", "component"],
                    &[
                        ("task:list p50", "/list_us/p50"),
                        ("task:create p50", "/create_us/p50"),
                        ("task:create p99", "/create_us/p99"),
                        ("task:update p50", "/update_us/p50"),
                        ("task:update p99", "/update_us/p99"),
                        ("task:setStatus p50", "/move_us/p50"),
                        ("task:setStatus p99", "/move_us/p99"),
                    ],
                    Some(("component", "builtin")),
                );
                let mut walks: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for r in runs.values().flatten() {
                    for w in r["walks"].as_array().into_iter().flatten() {
                        walks
                            .entry(format!("{} steps", w["steps"]))
                            .or_default()
                            .push(w.clone());
                    }
                }
                let _ = writeln!(
                    out,
                    "\n**In process, the step loop at {tier} tasks (no socket, no store)**\n"
                );
                let _ = writeln!(out, "| Chain | native µs/step p50 | component µs/step p50 | component µs/step p99 | instantiate µs p50 | instance KB |\n| --- | ---: | ---: | ---: | ---: | ---: |");
                for (k, w) in &walks {
                    let _ = writeln!(
                        out,
                        "| {k} | {} | {} | {} | {} | {} |",
                        cell(spread(w, "/native_us_per_step/p50")),
                        cell(spread(w, "/component_us_per_step/p50")),
                        cell(spread(w, "/component_us_per_step/p99")),
                        cell(spread(w, "/instantiate_us/p50")),
                        cell(spread(w, "/instance_kb/max")),
                    );
                }
            }
            "tasks" => {
                let rows = [
                    ("task:list ms p50", "/list_ms/p50"),
                    ("task:list ms p99", "/list_ms/p99"),
                    (
                        "config:load ms p50 (today's board read)",
                        "/config_load_ms/p50",
                    ),
                    ("create answer ms p50", "/create/answer_ms/p50"),
                    ("create answer ms p99", "/create/answer_ms/p99"),
                    ("create settle ms p99", "/create/settle_ms/p99"),
                    ("update answer ms p50", "/update/answer_ms/p50"),
                    ("update answer ms p99", "/update/answer_ms/p99"),
                    ("move answer ms p50", "/move/answer_ms/p50"),
                    ("move answer ms p99", "/move/answer_ms/p99"),
                    ("move settle ms p99", "/move/settle_ms/p99"),
                    ("change broadcast bytes p50", "/move/note_bytes/p50"),
                    (
                        "board drop via config:save ms p50",
                        "/board_save/answer_ms/p50",
                    ),
                    (
                        "board drop via config:save ms p99",
                        "/board_save/answer_ms/p99",
                    ),
                    ("vornd footprint MB at end", "/vornd_mb/end/footprint"),
                ];
                table(
                    &mut out,
                    &format!("Tasks against a test vornd, {tier} tasks"),
                    runs,
                    &["builtin", "ext", "ext-config"],
                    &rows,
                    Some(("ext", "builtin")),
                );
                for (label, p) in [
                    ("create p99", "/create/answer_ms/p99"),
                    ("update p99", "/update/answer_ms/p99"),
                    ("move p99", "/move/answer_ms/p99"),
                    ("list p99", "/list_ms/p99"),
                    ("vornd MB", "/vornd_mb/end/footprint"),
                ] {
                    rule.check(
                        format!("Tasks {tier}: {label} (rows)"),
                        runs,
                        "ext",
                        "builtin",
                        p,
                    );
                    rule.check(
                        format!("Tasks {tier}: {label} (config broadcast)"),
                        runs,
                        "ext-config",
                        "builtin",
                        p,
                    );
                }
            }
            "board" => {
                let rows = [
                    ("load ms (read + first frame)", "/load_ms"),
                    ("idle frame ms p50", "/idle_ms/p50"),
                    ("scroll frame ms p50", "/scroll_ms/p50"),
                    ("scroll frame ms p99", "/scroll_ms/p99"),
                    ("scroll frame ms max", "/scroll_ms/max"),
                    ("drag frame ms p50", "/drag_ms/p50"),
                    ("drag frame ms p99", "/drag_ms/p99"),
                    ("drag frame ms max", "/drag_ms/max"),
                    ("frames timed", "/frames"),
                    ("frames over 16.7 ms, %", "/over_16_7_pct"),
                    ("app footprint MB taken by the board", "/app_mb/board"),
                ];
                table(
                    &mut out,
                    &format!("Board frames, {tier} tasks"),
                    runs,
                    &["builtin", "kit", "kit-window"],
                    &rows,
                    Some(("kit-window", "builtin")),
                );
            }
            "flows" => {
                let rows = [
                    ("trigger → first step ms p50", "/trigger_to_first_ms/p50"),
                    ("trigger → first step ms p99", "/trigger_to_first_ms/p99"),
                    ("first step seen by a viewer ms p50", "/seen_first_ms/p50"),
                    ("first step seen by a viewer ms p99", "/seen_first_ms/p99"),
                    ("wall µs per step p50", "/per_step_us/p50"),
                    ("wall µs per step p99", "/per_step_us/p99"),
                    ("vornd CPU µs per step", "/cpu_us_per_step/p50"),
                    ("steps per second", "/steps_per_s/p50"),
                    ("peak KB per run", "/peak_kb_per_run/p50"),
                    ("run broadcasts MB per case", "/run_note_mb"),
                    ("failed runs", "/failed_runs"),
                ];
                table(
                    &mut out,
                    &format!("Workflows: {conc} concurrent runs of {steps} steps"),
                    runs,
                    &["builtin", "native", "ext"],
                    &rows,
                    Some(("ext", "builtin")),
                );
                // With runs in parallel, one run's wall time per step measures how the engine shares out steps, not its cost, so it counts alone.
                let solo = conc == "1";
                for (label, p) in [
                    ("trigger→first p99", "/trigger_to_first_ms/p99"),
                    ("per step p99", "/per_step_us/p99"),
                    ("CPU per step", "/cpu_us_per_step/p50"),
                    ("peak KB/run", "/peak_kb_per_run/p50"),
                ] {
                    if p == "/per_step_us/p99" && !solo {
                        continue;
                    }
                    rule.check(
                        format!("Workflows {steps}×{conc}: {label}"),
                        runs,
                        "ext",
                        "builtin",
                        p,
                    );
                }
                if let Some(r) = ratio(
                    spread(
                        runs.get("builtin").unwrap_or(&Vec::new()),
                        "/steps_per_s/p50",
                    ),
                    spread(runs.get("ext").unwrap_or(&Vec::new()), "/steps_per_s/p50"),
                ) {
                    rule.lines.push((format!("Workflows {steps}×{conc}: time per step at throughput (builtin steps/s ÷ ext)"), r));
                }
            }
            "gates" => {
                let rows = [
                    ("trigger → gate waiting ms p50", "/trigger_to_gate_ms/p50"),
                    ("resolveGate answer ms p50", "/answer_ms/p50"),
                    ("resolveGate answer ms p99", "/answer_ms/p99"),
                    (
                        "answer → next step starts ms p50",
                        "/answer_to_resume_ms/p50",
                    ),
                    (
                        "answer → next step starts ms p99",
                        "/answer_to_resume_ms/p99",
                    ),
                    ("answer → viewer sees it ms p99", "/seen_resume_ms/p99"),
                    ("parked KB per run", "/parked_kb_per_run/p50"),
                ];
                table(
                    &mut out,
                    &format!("Gates: {conc} runs parked at once"),
                    runs,
                    &["builtin", "native", "ext"],
                    &rows,
                    Some(("ext", "builtin")),
                );
                for (label, p) in [
                    ("answer p99", "/answer_ms/p99"),
                    ("resume p99", "/answer_to_resume_ms/p99"),
                ] {
                    rule.check(format!("Gates {conc}: {label}"), runs, "ext", "builtin", p);
                }
            }
            "schedule" => {
                let rows = [
                    (
                        "start after the earliest of its case ms p99 (jitter)",
                        "/jitter_ms/p99",
                    ),
                    ("seen by a viewer late ms p50", "/seen_late_ms/p50"),
                    ("start late ms p50", "/late_ms/p50"),
                    ("start late ms p99", "/late_ms/p99"),
                    ("start late ms max", "/late_ms/max"),
                    ("fired of 12", "/fired"),
                ];
                table(
                    &mut out,
                    "Schedules: 12 workflows due at their own second",
                    runs,
                    &["builtin", "native", "ext"],
                    &rows,
                    Some(("ext", "builtin")),
                );
                rule.check(
                    "Schedule: jitter p99".into(),
                    runs,
                    "ext",
                    "builtin",
                    "/jitter_ms/p99",
                );
            }
            _ => {}
        }
    }
    let _ = writeln!(out, "\n**Decision rule: extension within ~10% of built-in (p99 and memory)**\n\n| Check | ext / builtin | Within 10% |\n| --- | ---: | :---: |");
    for (what, r) in &rule.lines {
        let _ = writeln!(
            out,
            "| {what} | ×{r:.2} | {} |",
            if *r <= 1.10 { "yes" } else { "no" }
        );
    }
    std::fs::write(dir.join("summary.md"), &out).map_err(|e| e.to_string())?;
    print!("{out}");
    Ok(())
}
