//! `RunEntry` and `RunStepsList`: a workflow run's header and its steps,
//! shared by the task panel and the workflow views.

use serde_json::Value;

use super::gates;
use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::gates::Place;
use crate::look;
use crate::runs::{self, s as sv, Line, RunAct};
use crate::ui::{div, text, tooltip, Action, El, Rgba, Role};

fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

fn faint() -> Rgba {
    ink(look::INK_FAINT)
}

/// `NODE_TYPE_ICON`.
pub fn node_icon(kind: &str) -> &'static str {
    match kind {
        "trigger" | "callConnectorAction" => "zap",
        "launchAgent" => "play",
        "script" => "terminal",
        "condition" => "git-fork",
        "approval" => "hand",
        "createTaskFromItem" => "list-plus",
        "httpRequest" => "globe",
        "loop" => "repeat",
        _ => "circle",
    }
}

/// `stepMeta`: what a step is set to do, in one line.
pub fn step_meta(node: Option<&Value>) -> Option<String> {
    let node = node?;
    let str = |k: &str| {
        node.pointer(&format!("/config/{k}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let parts: Vec<Option<&str>> = match sv(node, "type")? {
        "trigger" if str("triggerType") == Some("connectorPoll") => {
            vec![str("connectionId"), str("event")]
        }
        "trigger" => vec![str("triggerType"), str("cron")],
        "script" => vec![str("scriptType"), str("projectName")],
        "launchAgent" => vec![str("projectName"), str("agentType"), str("branch")],
        "condition" => vec![str("variable"), str("operator"), str("value")],
        "createTaskFromItem" => vec![str("project"), str("initialStatus")],
        "callConnectorAction" => vec![str("connectionId"), str("action")],
        "httpRequest" => vec![str("method"), str("url")],
        _ => return None,
    };
    let kept: Vec<&str> = parts.into_iter().flatten().collect();
    (!kept.is_empty()).then(|| kept.join(" \u{b7} "))
}

/// `StatusDot`: eight pixels in the state's colour.
pub fn status_dot(status: &str) -> El {
    let (rgb, a) = runs::dot(status);
    div()
        .size(8.0, 8.0)
        .rounded(4.0)
        .bg(Rgba::hexa(rgb, a))
        .role(Role::Image, runs::label(status))
}

/// `IconButton`: a neutral `p-1` icon button named by its tooltip.
pub fn icon_button(s: &Screen<'_>, place: &str, act: Act, icon: &str, label: &str) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .bg(white(0.06 * hot))
        .role(Role::Button, label)
        .child(ico(icon, 12.0, 2.0, faint().mix(ink(1.0), hot)));
    tooltip(s.ui, b, id, label, None)
}

/// What a run list lets you do besides open it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Controls {
    pub rerun: bool,
    pub retry: bool,
}

/// One run: the header that opens it and, opened, its steps.
pub fn run_entry(
    s: &Screen<'_>,
    run: &Value,
    nodes: &Value,
    name: Option<&str>,
    ctl: Controls,
) -> El {
    // A live update beats the copy the list was loaded with.
    let run = s
        .store
        .runs
        .get(sv(run, "runId").unwrap_or_default())
        .unwrap_or(run);
    let key = runs::key(run);
    let open = s.st.runs.is_open(run);
    let toggle = s
        .acts
        .on("run-toggle", Act::Run(RunAct::Toggle(key.clone())));
    let hot = s.ui.hovered(toggle);
    let start = sv(run, "startedAt").unwrap_or("");
    let mut line = div().row().grow().min_w0().items_center();
    if let Some(n) = name {
        line = line.child(
            div()
                .pr(6.0)
                .child(text(n, 12.0, faint()).line_height(16.0).truncate().silent()),
        );
    }
    line = line.child(
        text(
            runs::relative(start, crate::state::now_ms()),
            12.0,
            ink(1.0),
        )
        .line_height(16.0)
        .truncate()
        .silent(),
    );
    let mut head = div()
        .id(toggle)
        .on(Action::Click)
        .row()
        .grow()
        .min_w0()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(10.0)
        .role(Role::Button, "Run")
        .expanded(open)
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            12.0,
            2.0,
            faint(),
        ))
        .child(status_dot(runs::run_status(run)))
        .child(line);
    if run.get("partial").and_then(Value::as_bool) == Some(true) {
        head = head.child(text("partial", 11.0, faint()).line_height(14.0).silent());
    }
    head = head.child(
        text(
            runs::duration(start, sv(run, "completedAt")),
            11.0,
            secondary(),
        )
        .mono()
        .line_height(14.0)
        .silent(),
    );
    let mut row = div()
        .row()
        .items_center()
        .bg(white(if hot { 0.03 } else { 0.0 }))
        .child(head);
    if ctl.retry && runs::has_failed_step(run) {
        row = row.child(icon_button(
            s,
            "run-retry",
            Act::Run(RunAct::Retry(key.clone())),
            "rotate-ccw",
            "Retry from failed step",
        ));
    }
    if ctl.rerun && sv(run, "status") != Some("running") {
        row = row.child(icon_button(
            s,
            "run-rerun",
            Act::Run(RunAct::Rerun(key.clone())),
            "play",
            "Run again",
        ));
    }
    if runs::stoppable(run) {
        let id = s.acts.on("run-stop", Act::Run(RunAct::Stop(key.clone())));
        let hot = s.ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .p(4.0)
            .rounded(4.0)
            .bg(white(0.06 * hot))
            .role(Role::Button, "Stop run")
            .child(ico("square", 11.0, 2.5, faint().mix(ink(1.0), hot)));
        row = row.child(div().pr(8.0).child(tooltip(s.ui, b, id, "Stop run", None)));
    }
    let mut frame = div()
        .col()
        .rounded(4.0)
        .border(1.0, white(0.08))
        .clip()
        .child(row);
    if open {
        frame = frame.child(steps(s, run, nodes, &key, false));
    }
    frame
}

/// `RunStepsList`: the inputs, then each step, the trigger first when
/// `trigger` (the runs pane reads what fired the run there).
pub fn steps(s: &Screen<'_>, run: &Value, nodes: &Value, key: &str, trigger: bool) -> El {
    let tasks = task_values(s);
    let mut col = div().col().w_full().border_top(1.0, white(0.06));
    let inputs: Vec<(&String, &Value)> = run
        .get("inputs")
        .and_then(Value::as_object)
        .map(|m| m.iter().collect())
        .unwrap_or_default();
    if !inputs.is_empty() {
        let mut wrap = div().row().wrap_children().gap(12.0).min_w0();
        for (k, v) in inputs {
            wrap = wrap.child(
                div()
                    .row()
                    .gap(4.0)
                    .min_w0()
                    .child(text(k, 11.5, faint()).mono().line_height(15.0).silent())
                    .child(
                        text(runs::input_value(v), 11.5, secondary())
                            .mono()
                            .line_height(15.0)
                            .truncate()
                            .silent(),
                    ),
            );
        }
        col = col.child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .px(16.0)
                .py(8.0)
                .border_bottom(1.0, white(0.05))
                .child(
                    text("INPUTS", 10.0, faint())
                        .tracking(0.05)
                        .line_height(15.0)
                        .silent(),
                )
                .child(wrap.grow()),
        );
    }
    let open = s.st.runs.step(run).map(str::to_owned);
    let shown = runs::steps(run, nodes, trigger);
    let last = shown.len().saturating_sub(1);
    for (i, ns) in shown.into_iter().enumerate() {
        let nid = sv(ns, "nodeId").unwrap_or("");
        let node = runs::node(nodes, nid);
        let status = sv(ns, "status").unwrap_or("pending");
        let rejected = ns.get("rejectedAt").is_some_and(|r| !r.is_null());
        let expanded = open.as_deref() == Some(nid);
        let dim = matches!(status, "pending" | "skipped");
        let id = s.acts.on(
            "run-step",
            Act::Run(RunAct::ToggleStep(key.to_owned(), nid.to_owned())),
        );
        let hot = s.ui.hovered(id);
        let glyph = if hot {
            ico("chevron-right", 12.0, 2.5, faint())
        } else {
            ico(
                node_icon(node.and_then(|n| sv(n, "type")).unwrap_or("")),
                12.0,
                1.5,
                faint(),
            )
        };
        let mut label = div()
            .row()
            .grow()
            .min_w0()
            .items_center()
            .gap(6.0)
            .child(div().size(12.0, 12.0).center().child(glyph))
            .child(
                text(
                    runs::node_label(node, nid),
                    12.5,
                    if dim { faint() } else { ink(1.0) },
                )
                .line_height(18.75)
                .truncate()
                .silent(),
            );
        if let Some(meta) = step_meta(node) {
            label = label.child(
                text(&meta, 11.5, faint())
                    .line_height(17.25)
                    .truncate()
                    .silent(),
            );
        }
        let kind = node.and_then(|n| sv(n, "type"));
        let gate_waits = status == "waiting" && kind == Some("approval");
        let config = node.and_then(|n| n.get("config"));
        if let Some(t) =
            sv(ns, "taskId").and_then(|id| tasks.iter().find(|t| sv(t, "id") == Some(id)))
        {
            label = label.child(task_chip(s, t, 80.0));
        }
        let dur = match (sv(ns, "startedAt"), sv(ns, "completedAt")) {
            (Some(a), Some(b)) => Some(text(runs::duration(a, Some(b)), 12.0, secondary())),
            _ if gate_waits => {
                crate::gates::round_label(config, ns).map(|r| text(r, 11.0, faint()))
            }
            _ => None,
        };
        let mut button = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(10.0)
            .px(16.0)
            .py(8.0)
            .bg(white(if hot { 0.02 } else { 0.0 }))
            .role(Role::Button, runs::node_label(node, nid))
            .expanded(expanded)
            .child(status_dot(if rejected { "cancelled" } else { status }))
            .child(label);
        if let Some(d) = dur {
            button = button.child(d.mono().line_height(18.0).silent());
        }
        let mut item = div().col().child(button);
        if i < last {
            item = item.border_bottom(1.0, white(0.05));
        }
        if gate_waits {
            item = item.child(
                under()
                    .col()
                    .gap(8.0)
                    .child(gates::ask(ns, config))
                    .child(gates::actions(s, Place::Trace, key, ns, config, nodes)),
            );
        }
        if status == "waiting" && sv(ns, "waitingFor") == Some("signIn") {
            let msg = sv(ns, "error").unwrap_or("Signed out. Sign in, and this step runs again.");
            item = item.child(
                under().row().gap(8.0).child(
                    div().grow().min_w0().child(
                        text(msg, 11.5, c(look::BRONZO))
                            .line_height(15.0)
                            .wrapping(),
                    ),
                ),
            );
        }
        if expanded {
            let resume = runs::resume_params(run, nodes, nid, &tasks).is_some();
            item = item.child(trace(s, ns, key, nid, resume));
        }
        col = col.child(item);
    }
    col
}

/// `TASK_CHIP`: a task's title, opening it.
fn task_chip(s: &Screen<'_>, t: &Value, max: f32) -> El {
    let tid = sv(t, "id").unwrap_or("").to_owned();
    let title = sv(t, "title").unwrap_or("");
    let id = s
        .acts
        .on("run-task", Act::Task(crate::tasks::TaskAct::Open(tid)));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .max_w(max)
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .border(1.0, white(0.08))
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, title)
        .child(
            text(title, 10.0, secondary())
                .line_height(15.0)
                .truncate()
                .silent(),
        )
}

/// `UNDER_LABEL`: lined up with the step's name, past the dot.
fn under() -> El {
    div().pl(34.0).pr(16.0).pb(10.0)
}

/// An opened step: its error, its trace and what can be done with it.
fn trace(s: &Screen<'_>, ns: &Value, key: &str, nid: &str, can_resume: bool) -> El {
    let status = sv(ns, "status").unwrap_or("pending");
    let rejected = ns.get("rejectedAt").is_some_and(|r| !r.is_null());
    let mut col = under().col().gap(6.0);
    let error = sv(ns, "error").filter(|e| !e.is_empty());
    if let Some(e) = error {
        let color = if rejected {
            secondary()
        } else {
            c(look::DANGER)
        };
        col = col.child(text(e, 12.0, color).line_height(16.0).wrapping());
    }
    let logs = sv(ns, "logs");
    let timeline = runs::timeline(logs, sv(ns, "diagnostics"));
    if !timeline.is_empty() {
        let mut bx = div()
            .id(crate::ui::Id::new("run-trace").child((key, nid)))
            .col()
            .max_h(280.0)
            .scroll_y()
            .rounded(4.0)
            .bg(Rgba::hexa(0x000000, 0.30))
            .border(1.0, white(0.05));
        for l in &timeline {
            // A closing line break opens no new line in a `<pre>`.
            let shown = |t: &str| t.strip_suffix('\n').unwrap_or(t).to_owned();
            bx = bx.child(match l {
                Line::Agent(t) => div().px(10.0).py(6.0).child(
                    text(shown(t), 12.0, secondary())
                        .mono()
                        .line_height(19.5)
                        .wrapping(),
                ),
                Line::Engine(t) => div().px(10.0).py(2.0).bg(white(0.02)).child(
                    text(shown(t), 12.0, faint())
                        .mono()
                        .line_height(19.5)
                        .wrapping(),
                ),
            });
        }
        col = col.child(bx);
    } else if error.is_none() {
        col = col.child(
            text(runs::empty_log(status), 11.5, faint())
                .line_height(15.0)
                .wrapping(),
        );
    }
    let can_full = logs.is_some_and(|l| !l.is_empty()) && !timeline.is_empty();
    if can_full || can_resume {
        let mut acts = div().row().items_center().gap(4.0);
        if can_full {
            acts = acts.child(icon_button(
                s,
                "run-output",
                Act::Run(RunAct::Output(key.to_owned(), nid.to_owned())),
                "maximize-2",
                "View full output",
            ));
        }
        if can_resume {
            acts = acts.child(icon_button(
                s,
                "run-resume",
                Act::Run(RunAct::Resume(key.to_owned(), nid.to_owned())),
                "rotate-ccw",
                "Resume session",
            ));
        }
        col = col.child(acts);
    }
    col
}

/// The tasks as JSON, for what a run's steps look up in them.
pub fn task_values(s: &Screen<'_>) -> Vec<Value> {
    s.store
        .tasks
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn step_meta_joins_what_the_node_sets() {
        let n = json!({ "type": "launchAgent", "config": { "projectName": "web", "agentType": "claude", "branch": " " } });
        assert_eq!(step_meta(Some(&n)).as_deref(), Some("web \u{b7} claude"));
        assert_eq!(step_meta(Some(&json!({ "type": "approval" }))), None);
        assert_eq!(node_icon("approval"), "hand");
    }
}
