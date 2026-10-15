//! Workflow runs as the run lists read them (`RunEntry`, `RunStepsList`):
//! their status dots, durations and the steps worth showing.

use std::collections::HashMap;

use serde_json::Value;

use crate::look::{self, Rgb};

/// `WORKFLOW_STATUS_DOT`: a run or step state's colour and alpha.
pub fn dot(status: &str) -> (Rgb, f32) {
    match status {
        "waiting" => (look::BRONZO, 1.0),
        "error" => (look::DANGER, 1.0),
        "running" => (look::INK, 1.0),
        "success" => (crate::tasks::SAGE, 1.0),
        _ => (look::WHITE, 0.18),
    }
}

/// `STATUS_LABELS`.
pub fn label(status: &str) -> &'static str {
    match status {
        "success" => "Success",
        "error" => "Error",
        "running" => "Running",
        "pending" => "Pending",
        "skipped" => "Skipped",
        "waiting" => "Waiting for approval",
        "cancelled" => "Stopped",
        _ => "Unknown",
    }
}

/// `formatRunDuration`.
pub fn duration(start: &str, end: Option<&str>) -> String {
    let Some(end) = end else {
        return "running...".to_owned();
    };
    let (Some(a), Some(b)) = (crate::tasks::ms(start), crate::tasks::ms(end)) else {
        return "\u{2014}".to_owned();
    };
    let ms = b - a;
    if ms < 0.0 {
        "\u{2014}".to_owned()
    } else if ms < 1000.0 {
        format!("{}ms", ms as u64)
    } else if ms < 60_000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        let s = (ms / 1000.0) as u64;
        format!("{}m {}s", s / 60, s % 60)
    }
}

/// `formatRelativeTime`.
pub fn relative(iso: &str, now: f64) -> String {
    let Some(t) = crate::tasks::ms(iso) else {
        return "Unknown".to_owned();
    };
    let mins = ((now - t) / 60_000.0).floor();
    if mins < 0.0 {
        return crate::tasks::long_date(iso);
    }
    let mins = mins as u64;
    match mins {
        0 => "Just now".to_owned(),
        1..=59 => format!("{mins}m ago"),
        60..=1439 => format!("{}h ago", mins / 60),
        1440..=43_199 => format!("{}d ago", mins / 1440),
        _ => crate::tasks::short_date(iso),
    }
}

/// A string field.
pub fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// `runDotStatus`: a run a gate rejected reads as stopped.
pub fn run_status(run: &Value) -> &str {
    let failed = states(run).find(|n| s(n, "status") == Some("error"));
    let rejected = failed.is_some_and(|n| n.get("rejectedAt").is_some_and(|r| !r.is_null()));
    if s(run, "status") == Some("error") && rejected {
        "cancelled"
    } else {
        s(run, "status").unwrap_or("pending")
    }
}

pub fn states(run: &Value) -> impl Iterator<Item = &Value> {
    run.get("nodeStates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// The workflow node `id` among `nodes`.
pub fn node<'a>(nodes: &'a Value, id: &str) -> Option<&'a Value> {
    nodes.as_array()?.iter().find(|n| s(n, "id") == Some(id))
}

/// `nodeLabel`: the node's label, else its id's first eight characters.
pub fn node_label(node: Option<&Value>, id: &str) -> String {
    node.and_then(|n| s(n, "label"))
        .filter(|l| !l.is_empty())
        .map_or_else(|| id.chars().take(8).collect(), str::to_owned)
}

/// The steps a run list shows: all but the trigger.
pub fn steps<'a>(run: &'a Value, nodes: &'a Value, trigger: bool) -> Vec<&'a Value> {
    states(run)
        .filter(|ns| {
            let id = s(ns, "nodeId").unwrap_or("");
            trigger || node(nodes, id).and_then(|n| s(n, "type")) != Some("trigger")
        })
        .collect()
}

/// The step the run list opens by itself: the running one, else where a
/// failed run broke.
pub fn focus(run: &Value) -> Option<&str> {
    let by = |st: &str| {
        states(run)
            .find(|n| s(n, "status") == Some(st))
            .and_then(|n| s(n, "nodeId"))
    };
    by("running").or_else(|| {
        (s(run, "status") == Some("error"))
            .then(|| by("error"))
            .flatten()
    })
}

/// The last `max` lines of a step's log, as the trace shows its tail.
pub fn tail(logs: &str, max: usize) -> String {
    let lines: Vec<&str> = logs.lines().collect();
    lines[lines.len().saturating_sub(max)..].join("\n")
}

/// `EMPTY_LOG`.
pub fn empty_log(status: &str) -> &'static str {
    match status {
        "running" => "No output captured yet\u{2026}",
        "pending" => "Step hasn't started yet.",
        "skipped" => "Step was skipped.",
        _ => "No output recorded.",
    }
}

/// A run's key in lists: its id, else the workflow and start.
pub fn key(run: &Value) -> String {
    s(run, "runId").map_or_else(
        || {
            format!(
                "{}-{}",
                s(run, "workflowId").unwrap_or(""),
                s(run, "startedAt").unwrap_or("")
            )
        },
        str::to_owned,
    )
}

/// What a run list's controls do; runs go by [`key`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunAct {
    Toggle(String),
    ToggleStep(String, String),
    Stop(String),
    Rerun(String),
    Retry(String),
    /// "View full output" on a step's logs.
    Output(String, String),
    /// "Resume session" on the agent session a step left.
    Resume(String, String),
    /// A gate's review page, in the browser.
    OpenReview(String, String),
}

/// `toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })`.
pub fn clock_time(iso: &str) -> String {
    let t = crate::tasks::long_date(iso);
    // "Oct 12, 9:05 AM": the time is what follows the comma.
    t.rsplit_once(", ")
        .map_or(t.clone(), |(_, time)| time.to_owned())
}

/// `encodeURIComponent`.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Which runs and steps the lists hold open, past their defaults.
#[derive(Debug, Default)]
pub struct RunView {
    pub open: HashMap<String, bool>,
    pub step: HashMap<String, Option<String>>,
    /// Each gate's draft, by [`crate::gates::key`].
    pub gates: HashMap<String, crate::gates::GateUi>,
    /// The review page open over everything: run and node.
    pub review: Option<(String, String)>,
}

impl RunView {
    /// Open by default while a gate waits.
    pub fn is_open(&self, run: &Value) -> bool {
        self.open
            .get(&key(run))
            .copied()
            .unwrap_or_else(|| states(run).any(|n| s(n, "status") == Some("waiting")))
    }

    /// The open step: the one picked, else [`focus`].
    pub fn step<'a>(&'a self, run: &'a Value) -> Option<&'a str> {
        match self.step.get(&key(run)) {
            Some(picked) => picked.as_deref(),
            None => focus(run),
        }
    }
}

/// `hasFailedStep`.
pub fn has_failed_step(run: &Value) -> bool {
    states(run)
        .any(|n| s(n, "status") == Some("error") && n.get("rejectedAt").is_none_or(Value::is_null))
}

/// `isRunStoppable`.
pub fn stoppable(run: &Value) -> bool {
    matches!(s(run, "status"), Some("running" | "waiting"))
        || states(run).any(|n| matches!(s(n, "status"), Some("running" | "waiting")))
}

/// `formatInputValue`: one line, clipped at 60 characters.
pub fn input_value(v: &Value) -> String {
    let text = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Object(_) | Value::Array(_) => v.to_string(),
        other => other.to_string(),
    };
    if text.chars().count() > 60 {
        format!("{}\u{2026}", text.chars().take(60).collect::<String>())
    } else {
        text
    }
}

/// One entry of a step's trace: the agent's words or an engine line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Agent(String),
    Engine(String),
}

/// `inlineLogTail`: the last 8000 characters, with a note of what went.
pub fn log_tail(logs: &str) -> String {
    const MAX: usize = 8000;
    let n = logs.chars().count();
    if n <= MAX {
        return logs.to_owned();
    }
    let cut = logs.char_indices().nth(n - MAX).map_or(0, |(i, _)| i);
    format!(
        "\u{2026} {} earlier characters hidden \u{2014} use View full output\n\n{}",
        grouped(n - MAX),
        &logs[cut..]
    )
}

/// `toLocaleString` for a count: `12,345`.
fn grouped(n: usize) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, ch) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Whether a diagnostics note is the engine's "First output from the agent".
fn first_output(note: &str) -> bool {
    note.strip_prefix("[+")
        .and_then(|r| r.split_once("s] "))
        .is_some_and(|(n, rest)| {
            !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit() || c == '.')
                && rest
                    .strip_prefix("First output from the agent")
                    .is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
        })
}

/// `stepTimeline`: the engine's notes up to the agent's first output, the
/// agent's log, then the notes on how the step ended.
pub fn timeline(logs: Option<&str>, diagnostics: Option<&str>) -> Vec<Line> {
    let notes: Vec<&str> = diagnostics
        .unwrap_or("")
        .split('\n')
        .filter(|n| !n.is_empty())
        .collect();
    let agent = logs.filter(|l| !l.trim().is_empty()).map(log_tail);
    let engine = |n: &[&str]| {
        n.iter()
            .map(|t| Line::Engine((*t).to_owned()))
            .collect::<Vec<_>>()
    };
    let Some(agent) = agent else {
        return engine(&notes);
    };
    let seam = notes.iter().position(|n| first_output(n));
    let (before, after) = match seam {
        Some(i) => notes.split_at(i + 1),
        None => (&notes[..], &[][..]),
    };
    let mut out = engine(before);
    out.push(Line::Agent(agent));
    out.extend(engine(after));
    out
}

/// What "Resume session" on step `nid` starts (`terminal:create`'s
/// params), as RunEntry gathers it from the step, its node and the tasks.
pub fn resume_params(
    agents: &crate::agents::Agents,
    run: &Value,
    nodes: &Value,
    nid: &str,
    tasks: &[Value],
) -> Option<Value> {
    let ns = states(run).find(|n| s(n, "nodeId") == Some(nid))?;
    let session = s(ns, "agentSessionId")?;
    let config = node(nodes, nid).and_then(|n| n.get("config"));
    let cfg = |k: &str| config.and_then(|c| c.get(k)).filter(|v| !v.is_null());
    let task = |id: Option<&str>| id.and_then(|id| tasks.iter().find(|t| s(t, "id") == Some(id)));
    let (node_task, trigger_task) = (task(s(ns, "taskId")), task(s(run, "triggerTaskId")));
    let from_tasks = |k: &str| {
        node_task
            .and_then(|t| t.get(k))
            .or_else(|| trigger_task.and_then(|t| t.get(k)))
            .filter(|v| !v.is_null())
    };
    let agent = s(ns, "agentType")
        .or_else(|| {
            cfg("agentType")
                .and_then(Value::as_str)
                .filter(|a| *a != "fromTask")
        })
        .filter(|a| crate::tasks::resumable(agents, a))?;
    fn nonempty(v: Option<&Value>) -> Option<&str> {
        v.and_then(Value::as_str).filter(|x| !x.is_empty())
    }
    let project = s(ns, "projectName")
        .filter(|x| !x.is_empty())
        .or_else(|| nonempty(cfg("projectName")))
        .or_else(|| nonempty(from_tasks("projectName")))?;
    let path = s(ns, "projectPath")
        .filter(|x| !x.is_empty())
        .or_else(|| nonempty(cfg("projectPath")))
        .unwrap_or("");
    let mut p = serde_json::json!({
        "agentType": agent,
        "projectName": project,
        "projectPath": path,
        "resumeSessionId": session,
    });
    if let Some(b) = cfg("branch").or_else(|| from_tasks("branch")) {
        p["branch"] = b.clone();
    }
    if let Some(w) = cfg("useWorktree").or_else(|| from_tasks("useWorktree")) {
        p["useWorktree"] = w.clone();
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn durations_read_as_the_renderer_formats_them() {
        let a = "2024-03-05T14:30:00.000Z";
        assert_eq!(duration(a, None), "running...");
        assert_eq!(duration(a, Some("2024-03-05T14:30:00.250Z")), "250ms");
        assert_eq!(duration(a, Some("2024-03-05T14:30:02.500Z")), "2.5s");
        assert_eq!(duration(a, Some("2024-03-05T14:32:05.000Z")), "2m 5s");
        assert_eq!(duration(a, Some("2024-03-05T14:29:00.000Z")), "\u{2014}");
    }

    #[test]
    fn relative_times_count_back_from_now() {
        let t = crate::tasks::ms("2024-03-05T14:30:00.000Z").unwrap();
        let at = "2024-03-05T14:30:00.000Z";
        assert_eq!(relative(at, t + 10_000.0), "Just now");
        assert_eq!(relative(at, t + 5.0 * 60_000.0), "5m ago");
        assert_eq!(relative(at, t + 3.0 * 3_600_000.0), "3h ago");
        assert_eq!(relative(at, t + 2.0 * 86_400_000.0), "2d ago");
        assert_eq!(relative(at, t + 40.0 * 86_400_000.0), "Mar 5");
    }

    #[test]
    fn steps_skip_the_trigger_and_focus_the_live_or_broken_one() {
        let nodes = json!([{ "id": "t0123456789", "type": "trigger" }, { "id": "a", "type": "launchAgent", "label": "Fix" }]);
        let run = json!({
            "status": "error",
            "nodeStates": [
                { "nodeId": "t0123456789", "status": "success" },
                { "nodeId": "a", "status": "error" }
            ]
        });
        let shown = steps(&run, &nodes, false);
        assert_eq!(shown.len(), 1);
        assert_eq!(focus(&run), Some("a"));
        assert_eq!(node_label(node(&nodes, "a"), "a"), "Fix");
        assert_eq!(
            node_label(node(&nodes, "t0123456789"), "t0123456789"),
            "t0123456"
        );
        assert_eq!(run_status(&run), "error");
    }

    #[test]
    fn timelines_split_the_notes_at_the_first_output() {
        let notes = "[+0.1s] Launching\n[+2.5s] First output from the agent\n[+9s] Exited";
        let t = timeline(Some("hi"), Some(notes));
        assert_eq!(
            t,
            vec![
                Line::Engine("[+0.1s] Launching".into()),
                Line::Engine("[+2.5s] First output from the agent".into()),
                Line::Agent("hi".into()),
                Line::Engine("[+9s] Exited".into()),
            ]
        );
        assert_eq!(
            timeline(Some("  "), Some("a")),
            vec![Line::Engine("a".into())]
        );
        assert!(log_tail(&"x".repeat(9000)).starts_with("\u{2026} 1,000 earlier"));
    }

    #[test]
    fn tails_keep_the_last_lines() {
        assert_eq!(tail("a\nb\nc", 2), "b\nc");
        assert_eq!(tail("a", 5), "a");
    }

    #[test]
    fn resume_takes_the_project_from_the_task() {
        let run = serde_json::json!({ "triggerTaskId": "t1", "nodeStates": [
            { "nodeId": "a", "agentSessionId": "s1", "agentType": "acme" },
            { "nodeId": "b", "agentSessionId": "s2", "agentType": "gamma" }
        ]});
        let nodes = serde_json::json!([{ "id": "a", "config": { "agentType": "fromTask" } }]);
        let tasks = [serde_json::json!({ "id": "t1", "projectName": "web", "branch": "x" })];
        let p =
            resume_params(&crate::agents::testing::agents(), &run, &nodes, "a", &tasks).unwrap();
        assert_eq!(p["projectName"], "web");
        assert_eq!(p["branch"], "x");
        assert_eq!(p["resumeSessionId"], "s1");
        assert!(
            resume_params(&crate::agents::testing::agents(), &run, &nodes, "b", &tasks).is_none()
        );
        assert!(resume_params(&crate::agents::testing::agents(), &run, &nodes, "a", &[]).is_none());
    }
}
