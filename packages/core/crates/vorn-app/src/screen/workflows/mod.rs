//! The Workflows view: the runs landing (`WorkflowsLandingView`), the
//! sidebar's section (`WorkflowsSection`), their menus, and the editor.

mod canvas;
mod dialogs;
mod editor;
mod forms;
mod panels;

use serde_json::Value;

use super::chrome::{divider, tool, window_controls};
use super::menus::Item;
use super::runs::{icon_button, status_dot};
use super::sidebar::{control, toggle_section};
use super::{c, ico, white, MainView, Screen, MOD_LABEL};
use crate::act::Act;
use crate::library::Look;
use crate::look;
use crate::runs::{self, s as sv, RunAct};
use crate::state::{now_ms, MenuKind};
use crate::ui::{div, text, tooltip, Action, El, Id, Rgba, Role, Sense};
use crate::workflows::{self as wf, Anchor, Bucket, SideFilter, Tab, WfAct};

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
pub(super) fn hex(v: Option<&str>) -> Option<Rgba> {
    let h = v?.strip_prefix('#')?;
    (h.len() == 6)
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
        .map(Rgba::hex)
}

/// `useConnectorLook`: how a connection's connector is drawn.
fn connection_look(s: &Screen<'_>, connection: &str) -> Option<Look> {
    let src = crate::library::Sources {
        connections: &s.st.wf.connections,
        packs: &s.st.wf.lib.packs,
        ..crate::library::Sources::default()
    };
    crate::library::look_for(&src, connection)
}

/// `parseConnectorWorkflowId`: the connection a connector-seeded workflow
/// (`connector:<connectionId>:<event>`) listens on.
fn seeded_connection(id: &str) -> Option<&str> {
    id.strip_prefix("connector:")?
        .split_once(':')
        .map(|(c, _)| c)
}

/// The look a connector run resolves to, from its item's connection.
fn run_look(s: &Screen<'_>, run: &Value) -> Option<Look> {
    run.pointer("/connectorItem/connectionId")
        .and_then(Value::as_str)
        .and_then(|c| connection_look(s, c))
}

/// `RunIcon`: the workflow's `ICON_MAP` glyph in its colour, else the
/// connector's mark for a connector run, else the source's glyph; the last
/// two in `color`.
fn run_icon(w: Option<&Value>, d: &wf::Described, lk: Option<&Look>, size: f32, color: Rgba) -> El {
    if let Some(n) = w.and_then(|w| sv(w, "icon")).and_then(look::icon_map) {
        let tint = w.and_then(|w| hex(sv(w, "iconColor")));
        return ico(n, size, 1.5, tint.unwrap_or(c(look::ICON_DEFAULT)));
    }
    match &d.connector {
        Some(connector) => {
            let lk = Look {
                connector: connector.clone(),
                icon: lk.and_then(|l| l.icon.clone()),
                packaged: lk.is_some_and(|l| l.packaged),
            };
            panels::mark(Some(&lk), size, color)
        }
        None => ico(d.fallback, size, 1.5, color),
    }
}

/// `ICON_MAP[icon] || Workflow` in the workflow's colour: the sidebar's and
/// the editor's mark.
fn wf_glyph(w: &Value, size: f32) -> El {
    let name = sv(w, "icon").and_then(look::icon_map).unwrap_or("workflow");
    ico(
        name,
        size,
        1.5,
        hex(sv(w, "iconColor")).unwrap_or(c(look::ICON_DEFAULT)),
    )
}

/// A canvas card's glyph: the trigger's kind or its connector, the agent's
/// mark, the script's language, the connector a step calls.
pub(super) fn node_glyph(s: &Screen<'_>, n: &Value, size: f32) -> El {
    let cfg = |k: &str| {
        n.pointer(&format!("/config/{k}"))
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let conn = match sv(n, "type").unwrap_or("") {
        "callConnectorAction" => Some(cfg("connectionId")),
        "trigger" if cfg("triggerType") == "connectorPoll" => Some(cfg("connectionId")),
        _ => None,
    };
    if let Some(look) = conn.filter(|c| !c.is_empty()).and_then(|c| {
        let src = crate::library::Sources {
            connections: &s.st.wf.connections,
            packs: &s.st.wf.lib.packs,
            ..crate::library::Sources::default()
        };
        crate::library::look_for(&src, c)
    }) {
        return panels::mark(Some(&look), size, c(look::INK));
    }
    let glyph = match sv(n, "type").unwrap_or("") {
        "trigger" => wf::trigger_icon(cfg("triggerType")),
        "script" => match cfg("scriptType") {
            "python" | "node" => "code-xml",
            _ => "terminal",
        },
        "launchAgent" if cfg("agentType") == "fromTask" => "clipboard-list",
        "launchAgent" => return super::agent_icon(s, cfg("agentType"), size),
        "callConnectorAction" => "zap",
        k => super::runs::node_icon(k),
    };
    ico(glyph, size, 2.0, secondary())
}

/// The editor's icon picker in its popover.
pub fn icon_picker(s: &Screen<'_>) -> El {
    let Some(d) = &s.st.wf.editor else {
        return div();
    };
    let icon = sv(&d.wf, "icon").unwrap_or("Workflow");
    let color = sv(&d.wf, "iconColor").unwrap_or("#3b82f6");
    div()
        .w(220.0)
        .p(8.0)
        .rounded(8.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Change icon")
        .child(icon_colors(
            s,
            "wf-icon",
            icon,
            color,
            |n| act(WfAct::Icon(n)),
            |h| act(WfAct::IconColor(h)),
        ))
}

/// `IconColorPicker`: `PROJECT_ICON_OPTIONS` eight across, then
/// `ICON_COLOR_PALETTE`, choosing through `on_icon` and `on_color`.
pub fn icon_colors(
    s: &Screen<'_>,
    place: &'static str,
    icon: &str,
    color: &str,
    on_icon: impl Fn(String) -> Act,
    on_color: impl Fn(String) -> Act,
) -> El {
    icon_colors_from(s, place, &look::ICON_OPTIONS, 22.0, icon, color, on_icon, on_color)
}

/// `icon_colors` over `options`, each cell `cell_w` wide.
#[allow(clippy::too_many_arguments)]
pub fn icon_colors_from(
    s: &Screen<'_>,
    place: &'static str,
    options: &[(&str, &str, &str)],
    cell_w: f32,
    icon: &str,
    color: &str,
    on_icon: impl Fn(String) -> Act,
    on_color: impl Fn(String) -> Act,
) -> El {
    let tint = hex(Some(color)).unwrap_or(c(0x3b82f6));
    let mut grid = div().row().wrap_children().gap(4.0);
    for &(name, glyph, label) in options {
        let id = s.acts.on(place, on_icon(name.to_owned()));
        let on = icon == name;
        let hot = s.ui.hover(id);
        let b = div()
            .id(id)
            .on(Action::Click)
            .size(cell_w, 24.0)
            .center()
            .rounded(4.0)
            .bg(if on {
                white(0.10)
            } else {
                white(0.0).mix(white(0.06), hot)
            })
            .border(1.0, if on { white(0.2) } else { white(0.0) })
            .role(Role::Button, label)
            .selected(on)
            .child(ico(glyph, 12.0, 1.5, if on { tint } else { c(0x9ca3af) }));
        grid = grid.child(tooltip(s.ui, b, id, label, None));
    }
    let mut swatches = div().row().gap(6.0);
    for rgb in look::ICON_COLORS {
        let hexs = format!("#{rgb:06x}");
        let id = s.acts.on(place, on_color(hexs.clone()));
        let on = color.eq_ignore_ascii_case(&hexs);
        let hot = s.ui.hover(id);
        let size = if on { 22.0 } else { 20.0 };
        swatches = swatches.child(
            div().size(20.0, 20.0).center().child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .size(size, size)
                    .rounded(size / 2.0)
                    .bg(c(rgb))
                    .border(
                        1.0,
                        if on {
                            c(look::WHITE)
                        } else {
                            white(0.0).mix(white(0.3), hot)
                        },
                    )
                    .role(Role::Button, &hexs)
                    .selected(on),
            ),
        );
    }
    div().col().gap(8.0).child(grid).child(swatches)
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
    let lk = run_look(s, run);
    let d = wf::describe(run, Some(&named), lk.as_ref().map(|l| l.connector.as_str()));
    let nodes = nodes_of(w);
    let start = sv(run, "startedAt").unwrap_or("");
    let progress = (sv(run, "status") != Some("success") && w.is_some())
        .then(|| wf::progress(run, nodes))
        .filter(|(done, _)| *done > 0)
        .map(|(done, total)| format!("{done} of {total} steps"));
    let partial = run.get("partial").and_then(Value::as_bool) == Some(true);
    let details: Vec<String> = [
        Some(wf::status_line(run, nodes)),
        Some(d.subtitle.clone().unwrap_or_else(|| d.source.clone())),
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
                .child(run_icon(w, &d, lk.as_ref(), 13.0, c(look::GRAY_400)))
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
    let lk = run_look(s, run);
    let d = wf::describe(run, Some(&named), lk.as_ref().map(|l| l.connector.as_str()));
    let nodes = nodes_of(w);
    let start = sv(run, "startedAt").unwrap_or("");
    let name = sv(&named, "name").map(str::trim).filter(|n| !n.is_empty());
    let short: String = key.chars().take(8).collect();
    let meta: Vec<String> = [
        name.filter(|n| *n != d.title).map(str::to_owned),
        Some(format!("Run {short}")),
        Some(d.source.clone()),
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
                .child(run_icon(w, &d, lk.as_ref(), 15.0, secondary())),
        )
        .child(
            div().grow().min_w0().child(
                text(d.title.as_str(), 15.0, ink(1.0))
                    .weight(500)
                    .line_height(22.5)
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
                .line_height(18.0)
                .truncate(),
        );
    if let Some(sub) = &d.subtitle {
        head = head.child(text(sub.as_str(), 13.0, secondary()).line_height(19.5));
    }
    head = head.child(
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(status_dot(dot_status(run)))
            .child(text(wf::status_line(run, nodes), 13.0, ink(1.0)).line_height(19.5)),
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
            .w_full()
            .px(20.0)
            .pb(24.0)
            .child(super::runs::steps(s, run, nodes, &key, true)),
    )
}

fn gate_button(
    s: &Screen<'_>,
    a: Act,
    icon: &str,
    label: &str,
    key: Option<&str>,
    approve: bool,
    disabled: bool,
) -> El {
    let id = s.acts.on("wf-gate", a);
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
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
        .row()
        .items_center()
        .gap(8.0)
        .px(16.0)
        .py(10.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(bg)
        .opacity(if disabled { 0.4 } else { 1.0 })
        .role(Role::Button, label)
        .disabled(disabled)
        .child(ico(icon, 14.0, 2.0, fg))
        .child(text(label, 13.0, fg).line_height(19.5).silent())
        .child(div().grow());
    if !disabled {
        b = b.on(Action::Click);
    }
    if let Some(k) = key {
        b = b.shortcut(k).child(
            div()
                .px(4.0)
                .py(2.0)
                .rounded(4.0)
                .border(1.0, white(0.08))
                .child(text(k, 10.0, faint()).mono().line_height(15.0).silent()),
        );
    }
    b
}

/// A `flex-1` neutral button of the pane's Edit, Open review and Request
/// changes row.
fn pane_button(s: &Screen<'_>, a: Act, icon: &str, label: &str) -> El {
    let id = s.acts.on("wf-gate", a);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .flex(1.0)
        .row()
        .items_center()
        .justify_center()
        .gap(8.0)
        .px(16.0)
        .py(8.0)
        .rounded(6.0)
        .border(1.0, white(0.06))
        .bg(white(0.0).mix(white(0.04), hot))
        .role(Role::Button, label)
        .child(ico(icon, 13.0, 1.75, secondary().mix(ink(1.0), hot)))
        .child(
            text(label, 12.5, secondary().mix(ink(1.0), hot))
                .line_height(18.75)
                .silent(),
        )
}

/// `RunDetailPane`'s waiting block: what the gate asks, its editor or
/// comment when open, then Approve & continue and Reject run.
fn gate_box(s: &Screen<'_>, run: &Value, gate: &Value, nodes: &Value) -> El {
    use crate::gates::{self as g, GateOp, Mode, Place};
    let key = runs::key(run);
    let nid = sv(gate, "nodeId").unwrap_or("").to_owned();
    let gk = g::key(Place::Pane, &key, &nid);
    let mut col = div().col().w_full().gap(6.0).px(20.0).pb(16.0).shrink0();
    let sign_in = wf::is_sign_in(gate);
    let node = runs::node(nodes, &nid);
    let approval = node.and_then(|n| sv(n, "type")) == Some("approval");
    let config = node.and_then(|n| n.get("config"));
    if !sign_in && approval {
        col = col.child(super::gates::ask(gate, config));
        let open = s.st.runs.gates.get(&gk);
        match open.and_then(|o| o.mode) {
            Some(Mode::Edit) => {
                if let Some(e) = open.and_then(|o| o.editor.as_ref()) {
                    col = col.child(super::gates::editor(s, &gk, e, true, false));
                }
            }
            Some(Mode::Changes) => {
                col = col.child(super::gates::composer(
                    s,
                    &gk,
                    Mode::Changes,
                    gate,
                    config,
                    nodes,
                    true,
                ));
            }
            _ => {
                let mut row = div().row().gap(6.0);
                let mut any = false;
                if g::editable(gate).is_some() {
                    let edited = open.is_some_and(|o| o.edited.is_some());
                    row = row.child(pane_button(
                        s,
                        Act::Gate(GateOp::Open(gk.clone(), Mode::Edit)),
                        "pencil",
                        if edited { "Edited" } else { "Edit" },
                    ));
                    any = true;
                }
                if sv(gate, "viewToken").is_some() {
                    row = row.child(pane_button(
                        s,
                        Act::Gate(GateOp::Review(Some((key.clone(), nid.clone())))),
                        "eye",
                        "Open review",
                    ));
                    any = true;
                }
                if g::can_request_changes(config, gate) {
                    row = row.child(pane_button(
                        s,
                        Act::Gate(GateOp::Open(gk.clone(), Mode::Changes)),
                        "message-square",
                        "Request changes",
                    ));
                    any = true;
                }
                if any {
                    col = col.child(row);
                }
            }
        }
    }
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
            act(WfAct::SignIn(conn)),
            "log-in",
            &format!("Sign in to {name}"),
            None,
            true,
            false,
        ));
    } else {
        let blocked =
            s.st.runs
                .gates
                .get(&gk)
                .and_then(|o| o.editor.as_ref().filter(|_| o.mode == Some(Mode::Edit)))
                .and_then(|e| e.invalid());
        col = col.child(gate_button(
            s,
            Act::Gate(GateOp::Approve(gk.clone())),
            "check",
            "Approve & continue",
            Some(&format!("{MOD_LABEL}\u{21b5}")),
            true,
            blocked.is_some(),
        ));
    }
    col.child(gate_button(
        s,
        Act::Gate(GateOp::Reject(gk)),
        "x",
        "Reject run",
        (!sign_in).then_some("R"),
        false,
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
    let actions = vec![
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
    let col = div().col().gap(2.0).child(toggle_section(
        s,
        "Workflows",
        !wfs.folded,
        act(WfAct::Fold),
        actions,
    ));
    if wfs.folded {
        return col;
    }
    let mut list = div().col().gap(2.0).child(all_runs(
        s,
        waiting.len(),
        s.st.view == MainView::Workflows && wfs.editor.is_none(),
    ));
    let shown: Vec<&Value> = listed(s)
        .into_iter()
        .filter(|w| wfs.side.keeps(w))
        .collect();
    if shown.is_empty() {
        list = list.child(
            div().px(10.0).py(4.0).child(
                text("No workflows", 13.0, c(look::GRAY_600))
                    .line_height(19.5)
                    .role(Role::Label, "No workflows"),
            ),
        );
    }
    let reorder = wfs.side == SideFilter::All;
    for (i, w) in shown.into_iter().enumerate() {
        let id = sv(w, "id").unwrap_or("");
        let is_waiting = s.store.runs.values().any(|r| {
            sv(r, "workflowId") == Some(id)
                && runs::states(r).any(|n| sv(n, "status") == Some("waiting"))
        });
        let failed = sv(w, "lastRunStatus") == Some("error");
        let over = reorder
            && wfs
                .drag
                .as_ref()
                .is_some_and(|(from, to)| *to == Some(i) && from != id);
        let row = side_row(s, w, wf::side_status(w, is_waiting, failed), reorder);
        list = list.child(if over {
            div().border_top(1.0, white(0.4)).child(row)
        } else {
            row
        });
    }
    col.child(list)
}

/// The `All runs` row: the landing, with the gates waiting on someone.
fn all_runs(s: &Screen<'_>, waiting: usize, on: bool) -> El {
    let id = s.acts.on("sidebar", act(WfAct::AllRuns));
    let hot = !on && s.ui.hovered(id);
    let fg = if on || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .bg(if hot { white(0.04) } else { white(0.0) })
        .role(Role::Button, "All runs")
        .toggled(on)
        .child_if(on.then(selected_bar))
        .child(ico("activity", 14.0, 1.5, c(look::GRAY_400)))
        .child(
            div().row().grow().min_w0().child(
                text("All runs", 13.0, fg)
                    .line_height(19.5)
                    .truncate()
                    .silent(),
            ),
        )
        .child_if((waiting > 0).then(|| {
            text(waiting.to_string(), 10.0, c(look::BRONZO))
                .mono()
                .line_height(15.0)
                .silent()
        }))
}

/// `absolute left-0 top-1 bottom-1 w-px bg-white`: the selected row's mark.
fn selected_bar() -> El {
    div()
        .absolute(0.0, 4.0)
        .w(1.0)
        .h(23.5)
        .rounded(0.5)
        .bg(c(look::WHITE))
}

/// `WorkflowItem`'s glyph: a connector-seeded workflow's connector in
/// gray-400, else the workflow's own.
fn side_glyph(s: &Screen<'_>, w: &Value) -> El {
    match sv(w, "id")
        .and_then(seeded_connection)
        .and_then(|c| connection_look(s, c))
    {
        Some(lk) => panels::mark(Some(&lk), 14.0, c(look::GRAY_400)),
        None => wf_glyph(w, 14.0),
    }
}

/// `WorkflowItem`: the glyph with its status dot, the name, and Run and
/// More on hover, held in place so the name truncates the same way.
fn side_row(s: &Screen<'_>, w: &Value, status: Option<&str>, reorder: bool) -> El {
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
            "waiting" => {
                let (rgb, a) = runs::dot("waiting");
                Rgba::hexa(rgb, a)
            }
            "scheduled" => secondary(),
            "disabled" => faint(),
            _ => {
                let (rgb, a) = runs::dot("error");
                Rgba::hexa(rgb, a)
            }
        };
        div()
            .absolute(size_dot_at(14.0), size_dot_at(14.0))
            .size(8.0, 8.0)
            .rounded(4.0)
            .bg(color)
            .border(1.0, c(look::SURFACE_BASE))
            .role(Role::Image, st)
    });
    let fg = if editing || hot {
        c(look::WHITE)
    } else {
        c(look::GRAY_300)
    };
    let small = |bid: Id, a: Act, icon: &str, size: f32, label: &str| {
        s.acts.bind(bid, a);
        let h = s.ui.hover(bid);
        let b = div()
            .id(bid)
            .on(Action::Click)
            .p(2.0)
            .rounded(4.0)
            .opacity(if hot { 1.0 } else { 0.0 })
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
    let mut row = div()
        .id(row_id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .opacity(if status == Some("disabled") { 0.4 } else { 1.0 })
        .bg(if hot && !editing {
            white(0.04)
        } else {
            white(0.0)
        })
        .role(Role::Button, name)
        .selected(editing)
        .child_if(editing.then(selected_bar))
        .child(div().shrink0().child(side_glyph(s, w)).child_if(dot))
        .child(
            div()
                .row()
                .grow()
                .min_w0()
                .child(text(name, 13.0, fg).line_height(19.5).truncate().silent()),
        )
        .child(small(
            run_id,
            act(WfAct::Run(id.clone())),
            "play",
            11.0,
            &format!("Run workflow {name}"),
        ))
        .child(small(
            more_id,
            Act::Open(menu.clone()),
            "ellipsis",
            12.0,
            &format!("More options for {name}"),
        ));
    if reorder {
        row = row.sense(Sense::MOVE);
    }
    s.acts.context(row, row_id, Act::Open(menu))
}

/// `-bottom-0.5 -right-0.5` on a glyph `size` wide, for an 8 px dot.
fn size_dot_at(size: f32) -> f32 {
    size + 2.0 - 8.0
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
            out.into_iter().map(Item::plain).collect()
        }
        MenuKind::EditorMore => {
            let Some(d) = &s.st.wf.editor else {
                return Vec::new();
            };
            let mut out = vec![Item::new(
                "settings",
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
            out.into_iter().map(Item::plain).collect()
        }
        MenuKind::NodeMore(id) => {
            let Some(n) = s.st.wf.editor.as_ref().and_then(|d| d.node(id)) else {
                return Vec::new();
            };
            let trigger = sv(n, "type") == Some("trigger");
            let anchor = if trigger {
                Anchor {
                    trigger: true,
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
    forms::choice_items(s, path)
}

/// How wide each workflow menu is.
pub fn menu_width(kind: &MenuKind) -> Option<f32> {
    Some(match kind {
        MenuKind::WfChoice(t) if !t.starts_with("prompt.") => forms::choice_width(t),
        MenuKind::RunStatus | MenuKind::WfChoice(_) => 200.0,
        MenuKind::WorkflowFilter | MenuKind::NodeMore(_) => 160.0,
        MenuKind::WorkflowItem(_) | MenuKind::EditorMore => 180.0,
        MenuKind::PromptProject => 240.0,
        MenuKind::WfIcon => 220.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seeded_workflow_names_its_connection() {
        assert_eq!(seeded_connection("connector:c1:issue.opened"), Some("c1"));
        assert_eq!(seeded_connection("connector:c1"), None);
        assert_eq!(seeded_connection("wf-1"), None);
    }

    #[test]
    fn icon_names_map_to_glyphs() {
        assert_eq!(look::icon_map("Workflow"), Some("workflow"));
        assert_eq!(look::icon_map("Play"), Some("play"));
        assert_eq!(look::icon_map("RotateCcw"), None, "not in ICON_MAP");
    }
}
