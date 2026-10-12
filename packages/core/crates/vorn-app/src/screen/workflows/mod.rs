//! The Workflows view: the runs landing (`WorkflowsLandingView`), the
//! sidebar's section (`WorkflowsSection`), their menus, and the editor.

mod dialogs;
mod editor;
mod panels;

use serde_json::Value;

use super::chrome::{divider, tool, window_controls};
use super::menus::Item;
use super::runs::{icon_button, status_dot};
use super::sidebar::{control, nav_item, section};
use super::{c, ico, white, MainView, Screen, MOD_LABEL};
use crate::act::Act;
use crate::look;
use crate::runs::{self, s as sv, RunAct};
use crate::state::{now_ms, MenuKind};
use crate::ui::{div, text, tooltip, Action, El, Id, Rgba, Role, Sense};
use crate::workflows::{self as wf, Anchor, Bucket, Panel, SideFilter, Tab, WfAct, Workflows};

pub use dialogs::dialog;
pub use editor::editor;

fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

fn faint() -> Rgba {
    ink(look::INK_FAINT)
}

fn act(a: WfAct) -> Act {
    Act::Workflow(a)
}

/// The raw workflow `id` from the config.
fn workflow<'a>(s: &'a Screen<'_>, id: &str) -> Option<&'a Value> {
    s.store
        .config
        .get("workflows")?
        .as_array()?
        .iter()
        .find(|w| runs::s(w, "id") == Some(id))
}

/// The active workspace's workflows, in the sidebar's order.
fn listed<'a>(s: &'a Screen<'_>) -> Vec<&'a Value> {
    let ws = s
        .store
        .config
        .pointer("/defaults/activeWorkspace")
        .and_then(Value::as_str)
        .unwrap_or("personal");
    s.store
        .config
        .get("workflows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| runs::s(w, "workspaceId").unwrap_or("personal") == ws)
        .collect()
}

/// `#rrggbb` as a colour.
fn hex(v: Option<&str>) -> Option<Rgba> {
    let h = v?.strip_prefix('#')?;
    (h.len() == 6)
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
        .map(Rgba::hex)
}

/// `ICON_MAP`'s PascalCase name as a lucide glyph, when there is one.
fn icon_name(name: Option<&str>) -> Option<String> {
    let name = name?;
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && (ch.is_ascii_uppercase() || ch.is_ascii_digit()) && !out.ends_with('-') {
            out.push('-');
        }
        out.push(ch.to_ascii_lowercase());
    }
    look::icon(&out, 1.5).is_some().then_some(out)
}

/// A workflow's own glyph in its colour, else `fallback`.
fn wf_icon(w: Option<&Value>, size: f32, fallback: &str, color: Rgba) -> El {
    let name = icon_name(w.and_then(|w| sv(w, "icon")));
    let tint = w.and_then(|w| hex(sv(w, "iconColor")));
    match name {
        Some(n) => ico(&n, size, 1.5, tint.unwrap_or(c(0x6b7280))),
        None => ico(fallback, size, 1.5, color),
    }
}

/// `SOURCE_ICONS`.
fn source_icon(source: &str) -> &'static str {
    match source {
        "scheduled" => "clock",
        "task" => "square-check",
        "restore" => "rotate-ccw",
        _ => "zap",
    }
}

fn nodes_of(w: Option<&Value>) -> &Value {
    static EMPTY: Value = Value::Null;
    w.and_then(|w| w.get("nodes")).unwrap_or(&EMPTY)
}

fn dot_status(run: &Value) -> &str {
    if runs::states(run).any(|n| sv(n, "status") == Some("waiting")) {
        "waiting"
    } else {
        runs::run_status(run)
    }
}

/// The view under the top bar: the run list and the selected run.
pub fn landing(s: &Screen<'_>) -> El {
    let visible = s.st.wf.visible(&s.store.runs);
    let current = s.st.wf.current(&visible);
    div()
        .row()
        .grow()
        .min_h(0.0)
        .items_stretch()
        .child(list(s, &visible, current))
        .child(div().w(1.0).shrink0().bg(white(0.06)))
        .child(detail(s, current))
}

fn list(s: &Screen<'_>, visible: &[Value], current: Option<&Value>) -> El {
    let all = s.st.wf.tab == Tab::All && s.st.wf.bucket == Bucket::All;
    let waiting = visible
        .iter()
        .filter(|r| runs::states(r).any(|n| sv(n, "status") == Some("waiting")))
        .count();
    let mut count = visible.len().to_string();
    if waiting > 0 {
        count.push_str(&format!(" \u{b7} {waiting} waiting"));
    }
    let title = if all { "All runs" } else { "Runs" };
    let head = div()
        .row()
        .items_end()
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .shrink0()
        .child(
            text(title, 15.0, ink(1.0))
                .line_height(22.0)
                .role(Role::Heading, title),
        )
        .child(text(count, 12.0, faint()).line_height(18.0).silent());
    let mut rows = div()
        .id(Id::new("wf-runs"))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .border_top(1.0, white(0.04))
        .role(Role::List, "Runs");
    if visible.is_empty() {
        rows = rows.child(
            div()
                .row()
                .justify_center()
                .py(40.0)
                .child(text("No runs to show", 12.0, faint()).line_height(16.0)),
        );
    }
    let picked = current.map(runs::key);
    for run in visible {
        let on = picked.as_deref() == Some(runs::key(run).as_str());
        rows = rows.child(run_row(s, run, on));
    }
    div()
        .col()
        .w(420.0)
        .shrink0()
        .min_h(0.0)
        .child(head)
        .child(rows)
}

fn run_row(s: &Screen<'_>, run: &Value, selected: bool) -> El {
    let key = runs::key(run);
    let wid = sv(run, "workflowId").unwrap_or("");
    let w = workflow(s, wid);
    let named = w
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "name": sv(run, "workflowName").unwrap_or("") }));
    let d = wf::describe(run, Some(&named));
    let nodes = nodes_of(w);
    let start = sv(run, "startedAt").unwrap_or("");
    let progress = (sv(run, "status") != Some("success") && w.is_some())
        .then(|| wf::progress(run, nodes))
        .filter(|(done, _)| *done > 0)
        .map(|(done, total)| format!("{done} of {total} steps"));
    let partial = run.get("partial").and_then(Value::as_bool) == Some(true);
    let details: Vec<String> = [
        Some(wf::status_line(run, nodes)),
        Some(d.subtitle.clone().unwrap_or_else(|| d.source.to_owned())),
        progress,
        partial.then(|| "partial".to_owned()),
        w.is_none().then(|| "deleted".to_owned()),
        Some(runs::relative(start, now_ms())),
    ]
    .into_iter()
    .flatten()
    .filter(|p| !p.is_empty())
    .collect();
    let id = s.acts.on("wf-run", act(WfAct::SelectRun(key.clone())));
    if w.is_some() {
        s.acts.double(id, act(WfAct::OpenRun(key)));
    }
    let hot = s.ui.hover(id);
    let line1 = div()
        .row()
        .items_center()
        .gap(10.0)
        .child(status_dot(dot_status(run)).size(6.0, 6.0).rounded(3.0))
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .items_center()
                .gap(8.0)
                .child(wf_icon(w, 13.0, source_icon(d.source), c(look::GRAY_400)))
                .child(
                    text(d.title.as_str(), 13.0, ink(1.0))
                        .line_height(18.0)
                        .truncate()
                        .silent(),
                ),
        )
        .child(
            text(
                runs::duration(start, sv(run, "completedAt")),
                12.0,
                secondary(),
            )
            .mono()
            .line_height(16.0)
            .silent(),
        );
    let line2 = div().pl(16.0).child(
        text(details.join(" \u{b7} "), 12.0, faint())
            .line_height(16.0)
            .truncate()
            .silent(),
    );
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .sense(Sense::DOUBLE)
        .col()
        .gap(2.0)
        .px(16.0)
        .py(10.0)
        .border_bottom(1.0, white(0.04))
        .bg(if selected {
            white(0.04)
        } else {
            white(0.0).mix(white(0.02), hot)
        })
        .role(Role::Button, d.title.as_str())
        .toggled(selected)
        .child(line1)
        .child(line2);
    if selected {
        row = row.child(
            div()
                .absolute(0.0, 4.0)
                .w(1.0)
                .h(48.0)
                .rounded(1.0)
                .bg(c(look::WHITE)),
        );
    }
    row
}

fn detail(s: &Screen<'_>, run: Option<&Value>) -> El {
    let Some(run) = run else {
        return div()
            .col()
            .grow()
            .min_w0()
            .center()
            .gap(8.0)
            .child(ico("inbox", 22.0, 1.5, faint()))
            .child(text("Select a run to see its trace", 13.0, faint()).line_height(18.0));
    };
    let key = runs::key(run);
    let wid = sv(run, "workflowId").unwrap_or("");
    let w = workflow(s, wid);
    let named = w
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "name": sv(run, "workflowName").unwrap_or("") }));
    let d = wf::describe(run, Some(&named));
    let nodes = nodes_of(w);
    let start = sv(run, "startedAt").unwrap_or("");
    let name = sv(&named, "name").map(str::trim).filter(|n| !n.is_empty());
    let short: String = key.chars().take(8).collect();
    let meta: Vec<String> = [
        name.filter(|n| *n != d.title).map(str::to_owned),
        Some(format!("Run {short}")),
        Some(d.source.to_owned()),
        Some(runs::relative(start, now_ms())),
        Some(runs::duration(start, sv(run, "completedAt"))),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut title = div()
        .row()
        .items_center()
        .gap(4.0)
        .min_w0()
        .child(
            div()
                .pr(4.0)
                .child(wf_icon(w, 15.0, source_icon(d.source), secondary())),
        )
        .child(
            div().grow().min_w0().child(
                text(d.title.as_str(), 15.0, ink(1.0))
                    .weight(500)
                    .line_height(22.0)
                    .truncate()
                    .role(Role::Heading, d.title.as_str()),
            ),
        );
    let button = |a: Act, icon: &str, label: &str| icon_button(s, "wf-detail", a, icon, label);
    if runs::has_failed_step(run) && w.is_some() {
        title = title.child(button(
            Act::Run(RunAct::Retry(key.clone())),
            "rotate-ccw",
            "Retry from failed step",
        ));
    }
    if sv(run, "status") != Some("running") && w.is_some() {
        title = title.child(button(
            Act::Run(RunAct::Rerun(key.clone())),
            "play",
            "Run again",
        ));
    }
    if w.is_some() {
        title = title.child(button(
            act(WfAct::OpenRun(key.clone())),
            "external-link",
            "Open workflow",
        ));
    }
    if runs::stoppable(run) {
        title = title.child(button(
            Act::Run(RunAct::Stop(key.clone())),
            "square",
            "Stop run",
        ));
    }
    let mut head = div()
        .col()
        .gap(8.0)
        .px(20.0)
        .pt(16.0)
        .pb(16.0)
        .shrink0()
        .child(title)
        .child(
            text(meta.join(" \u{b7} "), 12.0, faint())
                .line_height(16.0)
                .truncate(),
        );
    if let Some(sub) = &d.subtitle {
        head = head.child(text(sub.as_str(), 13.0, secondary()).line_height(18.0));
    }
    head = head.child(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(status_dot(dot_status(run)))
            .child(text(wf::status_line(run, nodes), 13.0, ink(1.0)).line_height(18.0)),
    );
    let mut col = div()
        .id(Id::new("wf-detail"))
        .col()
        .grow()
        .min_w0()
        .min_h(0.0)
        .scroll_y()
        .child(head);
    if let Some(gate) = runs::states(run).find(|n| sv(n, "status") == Some("waiting")) {
        col = col.child(gate_box(s, run, gate, nodes));
    }
    col.child(
        div()
            .px(20.0)
            .pb(24.0)
            .child(super::runs::steps(s, run, nodes, &key)),
    )
}

fn gate_button(
    s: &Screen<'_>,
    a: WfAct,
    icon: &str,
    label: &str,
    key: Option<&str>,
    approve: bool,
) -> El {
    let id = s.acts.on("wf-gate", act(a));
    let hot = s.ui.hover(id);
    let bronzo = |a: f32| Rgba::hexa(look::BRONZO, a);
    let (fg, bg, border) = if approve {
        (
            bronzo(1.0),
            bronzo(0.10).mix(bronzo(0.20), hot),
            bronzo(0.4),
        )
    } else {
        (
            secondary().mix(c(look::DANGER), hot),
            white(0.0).mix(Rgba::hexa(look::DANGER, 0.10), hot),
            white(0.06).mix(Rgba::hexa(look::DANGER, 0.30), hot),
        )
    };
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(16.0)
        .py(10.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(bg)
        .role(Role::Button, label)
        .child(ico(icon, 14.0, 2.0, fg))
        .child(text(label, 13.0, fg).line_height(18.0).silent())
        .child(div().grow());
    if let Some(k) = key {
        b = b.shortcut(k).child(
            div()
                .px(4.0)
                .py(2.0)
                .rounded(4.0)
                .border(1.0, white(0.08))
                .child(text(k, 10.0, faint()).mono().line_height(12.0).silent()),
        );
    }
    b
}

fn gate_box(s: &Screen<'_>, run: &Value, gate: &Value, nodes: &Value) -> El {
    let key = runs::key(run);
    let nid = sv(gate, "nodeId").unwrap_or("").to_owned();
    let mut col = div().col().gap(6.0).px(20.0).pb(16.0).shrink0();
    let sign_in = wf::is_sign_in(gate);
    let node = runs::node(nodes, &nid);
    if sign_in {
        let conn = sv(gate, "connectionId")
            .or_else(|| {
                node.and_then(|n| n.pointer("/config/connectionId"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("")
            .to_owned();
        let name =
            s.st.wf
                .connections
                .iter()
                .find(|c| runs::s(c, "id") == Some(conn.as_str()))
                .and_then(|c| runs::s(c, "name"))
                .unwrap_or("the connection")
                .to_owned();
        col = col.child(gate_button(
            s,
            WfAct::SignIn(conn),
            "log-in",
            &format!("Sign in to {name}"),
            None,
            true,
        ));
    } else {
        let message = sv(gate, "message")
            .or_else(|| {
                node.and_then(|n| n.pointer("/config/message"))
                    .and_then(Value::as_str)
            })
            .filter(|m| !m.is_empty());
        col = col.child(match message {
            Some(m) => div()
                .id(Id::new("wf-gate-message").child(&key))
                .max_h(192.0)
                .scroll_y()
                .rounded(6.0)
                .border(1.0, white(0.07))
                .bg(c(look::SURFACE_BASE))
                .px(10.0)
                .py(8.0)
                .child(
                    text(m, 12.0, ink(1.0))
                        .wrapping()
                        .min_w0()
                        .line_height(18.0),
                ),
            None => text("Waiting for approval.", 11.5, c(look::BRONZO)).line_height(16.0),
        });
        let asked = gate
            .get("feedback")
            .and_then(Value::as_array)
            .and_then(|f| {
                f.iter()
                    .rev()
                    .find(|f| sv(f, "decision") == Some("changes"))
            })
            .and_then(|f| sv(f, "comment"));
        if let Some(a) = asked {
            col = col.child(
                div()
                    .row()
                    .gap(6.0)
                    .child(text("You asked", 11.5, faint()).line_height(16.0).silent())
                    .child(text(a, 11.5, secondary()).line_height(16.0).truncate()),
            );
        }
        let approve = WfAct::Gate {
            run: key.clone(),
            node: nid.clone(),
            approve: true,
        };
        col = col.child(gate_button(
            s,
            approve,
            "check",
            "Approve & continue",
            Some(&format!("{MOD_LABEL}\u{21b5}")),
            true,
        ));
    }
    let reject = WfAct::Gate {
        run: key,
        node: nid,
        approve: false,
    };
    col.child(gate_button(
        s,
        reject,
        "x",
        "Reject run",
        (!sign_in).then_some("R"),
        false,
    ))
}

/// The landing's tools on the right of the top bar.
pub fn header_tools(s: &Screen<'_>) -> El {
    let waiting = s.st.wf.waiting(&s.store.runs).len();
    let tab = |t: Tab, label: &str, count: Option<usize>| {
        let on = s.st.wf.tab == t;
        let id = s.acts.on("wf-tab", act(WfAct::Tab(t)));
        let hot = s.ui.hover(id);
        let fg = if on {
            ink(1.0)
        } else {
            faint().mix(ink(1.0), hot)
        };
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .bg(if on {
                white(0.06)
            } else {
                white(0.0).mix(white(0.03), hot)
            })
            .role(Role::Tab, label)
            .selected(on)
            .child(text(label, 12.0, fg).line_height(16.0).silent());
        if let Some(n) = count.filter(|n| *n > 0) {
            b = b.child(
                text(n.to_string(), 10.0, c(look::BRONZO))
                    .mono()
                    .line_height(14.0)
                    .silent(),
            );
        }
        b
    };
    let mut right = div()
        .row()
        .items_center()
        .gap(4.0)
        .child(
            div()
                .row()
                .gap(2.0)
                .role(Role::TabList, "Runs")
                .child(tab(Tab::All, "All runs", None))
                .child(tab(Tab::Review, "Needs review", Some(waiting))),
        )
        .child(divider());
    if s.st.wf.tab == Tab::All {
        let open =
            s.st.menu
                .as_ref()
                .is_some_and(|m| m.kind == MenuKind::RunStatus);
        let mut opts = tool(
            s,
            "toolbar",
            Act::Open(MenuKind::RunStatus),
            "sliders-horizontal",
            1.5,
            "View options",
            None,
            open,
        );
        if s.st.wf.bucket != Bucket::All && !open {
            opts = div().child(opts).child(
                div()
                    .size(6.0, 6.0)
                    .rounded(3.0)
                    .bg(secondary())
                    .absolute_right(2.0, 2.0),
            );
        }
        right = right.child(opts);
    }
    right
        .child(tool(
            s,
            "toolbar",
            act(WfAct::Refresh),
            "refresh-cw",
            1.5,
            "Refresh",
            None,
            false,
        ))
        .child_if(window_controls(s))
}

/// `WorkflowsSection`.
pub fn sidebar(s: &Screen<'_>) -> El {
    let wfs = &s.st.wf;
    let mut filter = control(
        s,
        Act::Open(MenuKind::WorkflowFilter),
        "list-filter",
        "Filter workflows",
        13.0,
        1.5,
    );
    if wfs.side != SideFilter::All {
        filter = div().child(filter).child(
            div()
                .size(5.0, 5.0)
                .rounded(2.5)
                .bg(c(look::WHITE))
                .absolute_right(0.0, 0.0),
        );
    }
    let controls = vec![
        filter,
        control(
            s,
            act(WfAct::Import),
            "download",
            "Import workflow file",
            13.0,
            1.5,
        ),
        control(s, act(WfAct::New), "workflow", "New workflow", 13.0, 1.5),
    ];
    let waiting = wfs.waiting(&s.store.runs);
    let mut col = div()
        .col()
        .gap(2.0)
        .child(section(s, "Workflows", controls))
        .child(nav_item(
            s,
            act(WfAct::AllRuns),
            ico("activity", 14.0, 1.5, c(look::GRAY_400)),
            "All runs",
            Some(waiting.len()).filter(|n| *n > 0),
            s.st.view == MainView::Workflows && wfs.editor.is_none(),
        ));
    let list: Vec<&Value> = listed(s)
        .into_iter()
        .filter(|w| wfs.side.keeps(w))
        .collect();
    if list.is_empty() {
        col = col.child(
            div().px(10.0).py(4.0).child(
                text("No workflows", 13.0, c(look::GRAY_600))
                    .line_height(18.0)
                    .role(Role::Label, "No workflows"),
            ),
        );
    }
    for w in list {
        let id = sv(w, "id").unwrap_or("");
        let is_waiting = waiting.iter().any(|r| sv(r, "workflowId") == Some(id));
        let failed = sv(w, "lastRunStatus") == Some("error");
        col = col.child(side_row(s, w, wf::side_status(w, is_waiting, failed)));
    }
    col
}

fn side_row(s: &Screen<'_>, w: &Value, status: Option<&str>) -> El {
    let id = sv(w, "id").unwrap_or("").to_owned();
    let name = sv(w, "name").unwrap_or("");
    let editing =
        s.st.view == MainView::Workflows && s.st.wf.editor.as_ref().is_some_and(|d| d.id() == id);
    let menu = MenuKind::WorkflowItem(id.clone());
    let row_id = s.acts.on("wf-side", act(WfAct::Edit(id.clone())));
    let run_id = Id::new("wf-side-run").child(&id);
    let more_id = Id::new("wf-side-more").child(&id);
    let hot = s.ui.hovered(row_id)
        || s.ui.hovered(run_id)
        || s.ui.hovered(more_id)
        || s.st.menu.as_ref().is_some_and(|m| m.kind == menu);
    let dot = status.map(|st| {
        let color = match st {
            "waiting" => c(look::AMBER_400),
            "scheduled" => secondary(),
            "disabled" => faint(),
            _ => c(look::DANGER),
        };
        div()
            .size(6.0, 6.0)
            .rounded(3.0)
            .bg(color)
            .shrink0()
            .role(Role::Image, st)
    });
    let dim = if status == Some("disabled") { 0.4 } else { 1.0 };
    let fg = if editing || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    let mut row = div()
        .id(row_id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if editing {
            white(0.08)
        } else if hot {
            white(0.04)
        } else {
            white(0.0)
        })
        .role(Role::Button, name)
        .selected(editing)
        .child(
            div()
                .opacity(dim)
                .child(wf_icon(Some(w), 14.0, "workflow", c(look::GRAY_400))),
        )
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .opacity(dim)
                .child(text(name, 13.0, fg).line_height(18.0).truncate().silent()),
        )
        .child_if(dot);
    row = s.acts.context(row, row_id, Act::Open(menu.clone()));
    if hot {
        let small = |bid: Id, a: Act, icon: &str, size: f32, label: &str| {
            s.acts.bind(bid, a);
            let h = s.ui.hover(bid);
            let b = div()
                .id(bid)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), h))
                .role(Role::Button, label)
                .child(ico(
                    icon,
                    size,
                    2.0,
                    c(look::GRAY_500).mix(c(look::WHITE), h),
                ));
            tooltip(s.ui, b, bid, label, None)
        };
        row = row.child(
            div()
                .row()
                .items_center()
                .gap(2.0)
                .child(small(
                    run_id,
                    act(WfAct::Run(id.clone())),
                    "play",
                    11.0,
                    "Run",
                ))
                .child(small(more_id, Act::Open(menu), "ellipsis", 12.0, "More")),
        );
    }
    row
}

/// The step library's rows for the open anchor, through the search.
pub fn library_rows(wf: &Workflows) -> Vec<(&'static str, &'static str, &'static str)> {
    let trigger = matches!(&wf.panel, Panel::Library(Anchor { trigger: true, .. }));
    let q = wf.search.trim().to_lowercase();
    let pool: &[(&'static str, &'static str, &'static str)] =
        if trigger { &wf::TRIGGERS } else { &wf::STEPS };
    pool.iter()
        .filter(|(_, label, _)| q.is_empty() || label.to_lowercase().contains(&q))
        .copied()
        .collect()
}

/// The workflow menus' entries.
pub fn menu_items(s: &Screen<'_>, kind: &MenuKind) -> Vec<Item> {
    match kind {
        MenuKind::RunStatus => {
            let mut out = vec![Item::header("Status")];
            for b in Bucket::ALL {
                out.push(Item::option(
                    b.label(),
                    s.st.wf.bucket == b,
                    act(WfAct::Bucket(b)),
                ));
            }
            out
        }
        MenuKind::WorkflowFilter => {
            let mut out = vec![Item::header("Filter")];
            for f in SideFilter::ALL {
                out.push(Item::option(
                    f.label(),
                    s.st.wf.side == f,
                    act(WfAct::Side(f)),
                ));
            }
            out
        }
        MenuKind::WorkflowItem(id) => {
            let Some(w) = workflow(s, id) else {
                return Vec::new();
            };
            let mut out = vec![Item::new(
                "pencil",
                "Edit Workflow",
                act(WfAct::Edit(id.clone())),
            )];
            if wf::is_scheduled(w) {
                let on = w.get("enabled").and_then(Value::as_bool) != Some(false);
                let label = if on {
                    "Disable Schedule"
                } else {
                    "Enable Schedule"
                };
                out.push(Item::new(
                    "power",
                    label,
                    act(WfAct::ToggleSchedule(id.clone())),
                ));
            }
            out.push(Item::new(
                "upload",
                "Export as file\u{2026}",
                act(WfAct::Export(id.clone())),
            ));
            out.push(danger(Item::new(
                "trash-2",
                "Delete Workflow",
                act(WfAct::Delete(id.clone())),
            )));
            out
        }
        MenuKind::EditorMore => {
            let Some(d) = &s.st.wf.editor else {
                return Vec::new();
            };
            let mut out = vec![Item::new(
                "settings-2",
                "Workflow settings",
                act(WfAct::Settings),
            )];
            if d.existing {
                let id = d.id().to_owned();
                out.push(Item::new(
                    "upload",
                    "Export as file\u{2026}",
                    act(WfAct::Export(id.clone())),
                ));
                out.push(
                    danger(Item::new(
                        "trash-2",
                        "Delete workflow",
                        act(WfAct::Delete(id)),
                    ))
                    .sep(),
                );
            }
            out
        }
        MenuKind::NodeMore(id) => {
            let Some(n) = s.st.wf.editor.as_ref().and_then(|d| d.node(id)) else {
                return Vec::new();
            };
            let trigger = sv(n, "type") == Some("trigger");
            let anchor = if trigger {
                Anchor {
                    trigger: true,
                    replace: Some(id.clone()),
                    ..Anchor::default()
                }
            } else {
                Anchor {
                    after: Some(id.clone()),
                    replace: Some(id.clone()),
                    ..Anchor::default()
                }
            };
            let remove = if trigger {
                "Remove trigger"
            } else {
                "Remove action"
            };
            vec![
                Item::new("replace", "Replace step", act(WfAct::Library(anchor))),
                danger(Item::new(
                    "trash-2",
                    remove,
                    act(WfAct::DeleteNode(id.clone())),
                )),
            ]
        }
        MenuKind::WfChoice(path) => choice_items(s, path),
        MenuKind::PromptProject => {
            let cur = s.st.wf.prompt.as_ref().and_then(|p| p.project.as_deref());
            s.store
                .projects
                .iter()
                .map(|p| {
                    Item::option(
                        p.name.as_str(),
                        cur == Some(p.name.as_str()),
                        act(WfAct::PromptProject(p.name.clone())),
                    )
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn danger(mut i: Item) -> Item {
    i.danger = true;
    i.tint(look::DANGER)
}

fn choice_items(s: &Screen<'_>, path: &str) -> Vec<Item> {
    if let Some(key) = path.strip_prefix("prompt.") {
        let Some(p) = &s.st.wf.prompt else {
            return Vec::new();
        };
        let def = p.inputs.iter().find(|d| sv(d, "key") == Some(key));
        let cur = p
            .picked
            .get(key)
            .cloned()
            .or_else(|| def.map(wf::input_default))
            .unwrap_or_default();
        if def.and_then(|d| sv(d, "type")) == Some("project") {
            return s
                .store
                .projects
                .iter()
                .map(|x| {
                    Item::option(
                        x.name.as_str(),
                        cur == x.name,
                        act(WfAct::PromptSet(key.to_owned(), x.name.clone())),
                    )
                })
                .collect();
        }
        return def
            .and_then(|d| d.get("options"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let v = sv(o, "value")?;
                let label = sv(o, "label").filter(|l| !l.is_empty()).unwrap_or(v);
                Some(Item::option(
                    label,
                    cur == v,
                    act(WfAct::PromptSet(key.to_owned(), v.to_owned())),
                ))
            })
            .collect();
    }
    let cur = current(s, path);
    let set = |v: &str| act(WfAct::Set(path.to_owned(), v.to_owned()));
    if path == "config.projectName" || path == "config.projectFilter" {
        let mut out = Vec::new();
        if path == "config.projectFilter" {
            out.push(Item::option("All projects", cur.is_empty(), set("")));
        }
        for p in &s.store.projects {
            out.push(Item::option(p.name.as_str(), cur == p.name, set(&p.name)));
        }
        return out;
    }
    wf::choices(path)
        .into_iter()
        .map(|(v, label)| Item::option(label, cur == v, set(v)))
        .collect()
}

/// The open form's value at `path`.
fn current(s: &Screen<'_>, path: &str) -> String {
    let Some(d) = &s.st.wf.editor else {
        return String::new();
    };
    match s.st.wf.node.as_deref().and_then(|n| d.node(n)) {
        Some(n) if s.st.wf.panel == Panel::Node => wf::field_text(n, path),
        _ => wf::text_of(d.wf.get(path)),
    }
}

/// How wide each workflow menu is.
pub fn menu_width(kind: &MenuKind) -> Option<f32> {
    Some(match kind {
        MenuKind::RunStatus | MenuKind::WfChoice(_) => 200.0,
        MenuKind::WorkflowFilter | MenuKind::NodeMore(_) => 160.0,
        MenuKind::WorkflowItem(_) | MenuKind::EditorMore => 180.0,
        MenuKind::PromptProject => 240.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_names_map_to_glyphs() {
        assert_eq!(icon_name(Some("Workflow")).as_deref(), Some("workflow"));
        assert_eq!(icon_name(Some("GitBranch")).as_deref(), Some("git-branch"));
        assert_eq!(icon_name(Some("NoSuchGlyph")), None);
    }

    #[test]
    fn the_library_filters_by_label() {
        let mut w = Workflows {
            panel: Panel::Library(Anchor::default()),
            ..Workflows::default()
        };
        assert_eq!(library_rows(&w).len(), wf::STEPS.len());
        w.search = "scr".into();
        assert_eq!(library_rows(&w)[0].0, "script");
        w.panel = Panel::Library(Anchor {
            trigger: true,
            ..Anchor::default()
        });
        w.search = "task".into();
        assert_eq!(library_rows(&w).len(), 2);
    }
}
