//! `WorkflowEditor`: the 40 px bar, the canvas of cards and the right panel.

use std::collections::HashMap;

use serde_json::Value;

use super::{act, wf_icon};
use crate::act::Act;
use crate::look;
use crate::runs::{self, s as sv};
use crate::screen::chrome::{draggable, nav_cluster, window_controls};
use crate::screen::{c, ico, white, Screen};
use crate::state::{now_ms, MenuKind};
use crate::ui::{div, field, text, tooltip, Action, El, Id, Rgba, Role, TextStyle};
use crate::workflows::{self as wf, Anchor, Draft, Panel, Row, WfAct};

/// The editor in place of the top bar and the landing.
pub fn editor(s: &Screen<'_>) -> El {
    let Some(d) = &s.st.wf.editor else {
        return div();
    };
    let body = div()
        .row()
        .grow()
        .min_h(0.0)
        .items_stretch()
        .child(canvas(s, d))
        .child_if(super::panels::panel(s, d));
    div().col().grow().min_w0().child(bar(s, d)).child(body)
}

fn bar_button(s: &Screen<'_>, a: WfAct, icon: &str, label: &str, on: bool, disabled: bool) -> El {
    let id = s.acts.on("wf-editor", act(a));
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let fg = if on {
        c(look::WHITE)
    } else {
        c(look::GRAY_400).mix(c(look::WHITE), hot)
    };
    let mut b = div()
        .id(id)
        .p(6.0)
        .rounded(6.0)
        .bg(if on {
            white(0.08)
        } else {
            white(0.0).mix(white(0.06), hot)
        })
        .opacity(if disabled { 0.4 } else { 1.0 })
        .role(Role::Button, label)
        .disabled(disabled)
        .child(ico(icon, 15.0, 2.0, fg));
    if !disabled {
        b = b.on(Action::Click);
    }
    b
}

fn bar(s: &Screen<'_>, d: &Draft) -> El {
    let mut left = div().row().items_center().gap(8.0).min_w0();
    if !s.st.sidebar {
        left = left.child(nav_cluster(s));
    }
    let name_id = wf::field_id(None, "name");
    let hot = if s.ui.focused(name_id) {
        0.06
    } else {
        0.04 * s.ui.hover(name_id)
    };
    left = left
        .child(wf_icon(Some(&d.wf), 16.0, "workflow", c(look::GRAY_400)))
        .child(
            field(
                name_id,
                "Workflow name",
                "Workflow name",
                TextStyle {
                    weight: 500,
                    line_height: 22.0,
                    ..TextStyle::ui(15.0)
                },
                c(look::WHITE),
            )
            .w(240.0)
            .px(8.0)
            .py(4.0)
            .rounded(6.0)
            .bg(white(hot)),
        );
    let mut right = div().row().items_center().gap(4.0);
    match s.st.wf.running_since(&s.store.runs) {
        Some(since) => {
            let total = ((now_ms() - since) / 1000.0).max(0.0) as u64;
            let clock = format!("{}:{:02}", total / 60, total % 60);
            let id = s.acts.on("wf-editor", act(WfAct::Stop));
            let hot = s.ui.hover(id);
            let stop = div()
                .id(id)
                .on(Action::Click)
                .p(6.0)
                .rounded(6.0)
                .bg(white(0.0).mix(white(0.06), hot))
                .role(Role::Button, "Stop run")
                .child(ico(
                    "square",
                    13.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::DANGER), hot),
                ));
            right = right.child(
                div()
                    .row()
                    .items_center()
                    .gap(2.0)
                    .child(
                        div()
                            .row()
                            .items_center()
                            .gap(6.0)
                            .px(6.0)
                            .child(ico("loader-circle", 13.0, 2.0, c(look::GRAY_300)))
                            .child(
                                text(clock, 11.0, c(look::GRAY_300))
                                    .mono()
                                    .line_height(14.0)
                                    .role(Role::Timer, "Elapsed"),
                            ),
                    )
                    .child(tooltip(s.ui, stop, id, "Stop run", None)),
            );
        }
        None => {
            let ready = wf::trigger(d.nodes()).is_some();
            let b = bar_button(s, WfAct::RunDraft, "play", "Run workflow", false, !ready);
            let id = bar_id(WfAct::RunDraft);
            let tip = if ready {
                "Run workflow"
            } else {
                "Add a trigger before running"
            };
            right = right.child(tooltip(s.ui, b, id, tip, None));
        }
    }
    if d.existing {
        let n = s.st.wf.history.len();
        let label = if n > 0 {
            format!("Run history ({n})")
        } else {
            "Run history".to_owned()
        };
        let on = s.st.wf.panel == Panel::History;
        let b = bar_button(
            s,
            WfAct::History,
            "history",
            &format!("Run history ({n})"),
            on,
            false,
        );
        right = right.child(tooltip(s.ui, b, bar_id(WfAct::History), &label, None));
    }
    let save_id = s.acts.on("wf-editor", act(WfAct::Save));
    let save_hot = s.ui.hover(save_id);
    right = right.child(
        div()
            .id(save_id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .ml(4.0)
            .px(12.0)
            .py(6.0)
            .rounded(6.0)
            .bg(white(0.12).mix(white(0.18), save_hot))
            .role(Role::Button, "Save")
            .child(ico("save", 13.0, 1.5, c(look::WHITE)))
            .child(
                text("Save", 12.0, c(look::WHITE))
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
            ),
    );
    let open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::EditorMore);
    let more_id = s.acts.on("wf-editor", Act::Open(MenuKind::EditorMore));
    let more_hot = s.ui.hover(more_id);
    let more = div()
        .id(more_id)
        .on(Action::Click)
        .ml(2.0)
        .p(6.0)
        .rounded(6.0)
        .bg(if open {
            white(0.08)
        } else {
            white(0.0).mix(white(0.06), more_hot)
        })
        .role(Role::Button, "More options")
        .has_popup()
        .expanded(open)
        .child(ico(
            "ellipsis",
            15.0,
            2.0,
            if open {
                c(look::WHITE)
            } else {
                c(look::GRAY_400).mix(c(look::WHITE), more_hot)
            },
        ));
    right = right
        .child(tooltip(s.ui, more, more_id, "More options", None))
        .child_if(window_controls(s));
    let row = div()
        .row()
        .items_center()
        .justify_between()
        .h(look::TOP_BAR_H - 1.0)
        .pl(12.0)
        .pr(if s.chrome { 0.0 } else { 12.0 })
        .role(Role::Toolbar, "Workflow editor")
        .child(left)
        .child(right);
    draggable(
        s,
        div()
            .col()
            .shrink0()
            .bg(c(look::SURFACE_BASE))
            .child(row)
            .child(div().h(1.0).w_full().bg(white(0.08))),
    )
}

fn bar_id(a: WfAct) -> Id {
    crate::act::Acts::id("wf-editor", &act(a))
}

/// What each node did in the run on show: the one going, else the newest.
fn statuses(s: &Screen<'_>, d: &Draft) -> HashMap<String, String> {
    let run =
        s.st.wf
            .active_run(d.id(), &s.store.runs)
            .or_else(|| s.st.wf.history.first().cloned());
    let mut out = HashMap::new();
    if let Some(run) = run {
        let live = s.store.runs.get(&runs::key(&run)).unwrap_or(&run);
        for n in runs::states(live) {
            if let (Some(id), Some(st)) = (sv(n, "nodeId"), sv(n, "status")) {
                out.insert(id.to_owned(), st.to_owned());
            }
        }
    }
    out
}

fn canvas(s: &Screen<'_>, d: &Draft) -> El {
    let exec = statuses(s, d);
    let rows = wf::flow(d.nodes(), d.edges());
    let mut col = div().col().items_center().pt(48.0).pb(96.0).px(48.0);
    if wf::trigger(d.nodes()).is_none() {
        col = col.child(add_trigger(s));
        if !d.nodes().is_empty() {
            col = col.child(edge(28.0));
        }
    }
    col = col.child(chain(s, d, &rows, &exec));
    div()
        .id(Id::new("wf-canvas"))
        .col()
        .grow()
        .min_w0()
        .min_h(0.0)
        .scroll_y()
        .bg(c(look::SURFACE_BASE))
        .role(Role::Group, "Workflow canvas")
        .child(col)
}

fn edge(h: f32) -> El {
    div().w(1.5).h(h).bg(white(0.16)).shrink0()
}

fn add_trigger(s: &Screen<'_>) -> El {
    let at = Anchor {
        trigger: true,
        ..Anchor::default()
    };
    let active = matches!(&s.st.wf.panel, Panel::Library(a) if a.trigger && a.replace.is_none());
    let id = s.acts.on("wf-canvas", act(WfAct::Library(at)));
    let hot = s.ui.hover(id);
    let (fg, border) = if active {
        (c(look::WHITE), white(0.40))
    } else {
        (
            c(look::GRAY_500).mix(c(look::GRAY_300), hot),
            white(0.15).mix(white(0.30), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .center()
        .gap(8.0)
        .size(280.0, 58.0)
        .rounded(8.0)
        .border(1.0, border)
        .role(Role::Button, "Add a trigger")
        .child(ico("zap", 14.0, 2.0, fg))
        .child(text("Add a trigger", 13.0, fg).line_height(18.0).silent())
}

/// `AddStepNode`: the round "+" that opens the library at `at`.
fn plus(s: &Screen<'_>, at: Anchor) -> El {
    let active = s.st.wf.panel == Panel::Library(at.clone());
    let id = s.acts.on("wf-plus", act(WfAct::Library(at)));
    let hot = s.ui.hover(id);
    let (fg, bg, border) = if active {
        (c(look::WHITE), white(0.08), white(0.40))
    } else {
        (
            c(look::GRAY_500).mix(c(look::WHITE), hot),
            c(look::SURFACE_SUNKEN),
            white(0.10).mix(white(0.20), hot),
        )
    };
    div()
        .id(id)
        .on(Action::Click)
        .size(22.0, 22.0)
        .rounded(11.0)
        .center()
        .bg(bg)
        .border(1.0, border)
        .role(Role::Button, "Add a step")
        .child(ico("plus", 13.0, 2.5, fg))
}

fn outs<'a>(d: &'a Draft, id: &str) -> Vec<&'a str> {
    d.edges()
        .iter()
        .filter(|e| sv(e, "source") == Some(id))
        .filter_map(|e| sv(e, "target"))
        .collect()
}

fn chain(s: &Screen<'_>, d: &Draft, rows: &[Row], exec: &HashMap<String, String>) -> El {
    let mut col = div().col().items_center();
    for (i, row) in rows.iter().enumerate() {
        match row {
            Row::Node { id, next } => {
                col = col.child(card(s, d, id, exec.get(id).map(String::as_str)));
                let forks = matches!(rows.get(i + 1), Some(Row::Fork { .. }));
                let after = outs(d, id);
                if forks {
                    col = col.child(edge(16.0));
                } else if let Some(n) = next
                    .clone()
                    .or_else(|| after.first().map(|t| (*t).to_owned()))
                {
                    let at = Anchor {
                        after: Some(id.clone()),
                        before: Some(n),
                        ..Anchor::default()
                    };
                    col = col.child(edge(12.0)).child(plus(s, at)).child(edge(12.0));
                } else if after.is_empty()
                    && d.node(id).and_then(|n| sv(n, "type")) != Some("condition")
                {
                    let at = Anchor {
                        after: Some(id.clone()),
                        ..Anchor::default()
                    };
                    col = col.child(edge(20.0)).child(plus(s, at));
                }
            }
            Row::Fork { branches, .. } => {
                let mut lanes = div().row().gap(32.0);
                for (tag, rows) in branches {
                    let label = match tag.as_deref() {
                        Some("true") => "True",
                        Some("false") => "False",
                        Some(other) => other,
                        None => "",
                    };
                    let mut lane = div().col().items_center();
                    if !label.is_empty() {
                        lane = lane.child(
                            div()
                                .px(8.0)
                                .py(2.0)
                                .rounded(10.0)
                                .border(1.0, white(0.10))
                                .bg(c(look::SURFACE_SUNKEN))
                                .child(text(label, 11.0, c(look::GRAY_400)).line_height(14.0)),
                        );
                        lane = lane.child(edge(12.0));
                    }
                    lanes = lanes.child(lane.child(chain(s, d, rows, exec)));
                }
                col = col.child(lanes);
                if i + 1 < rows.len() {
                    col = col.child(edge(24.0));
                }
            }
        }
    }
    col
}

fn card(s: &Screen<'_>, d: &Draft, id: &str, status: Option<&str>) -> El {
    let Some(node) = d.node(id) else {
        return div();
    };
    let label = sv(node, "label").unwrap_or("");
    let selected = s.st.wf.node.as_deref() == Some(id) && s.st.wf.panel == Panel::Node;
    let cid = s.acts.on("wf-card", act(WfAct::SelectNode(id.to_owned())));
    let tools = [
        Id::new("wf-card-run").child(id),
        Id::new("wf-card-replace").child(id),
        Id::new("wf-card-delete").child(id),
    ];
    let hovering = s.ui.hovered(cid) || tools.iter().any(|t| s.ui.hovered(*t));
    let hot = if hovering { 1.0 } else { 0.0 };
    let border = if selected {
        white(0.40)
    } else if status == Some("error") {
        Rgba::hexa(look::DANGER, 0.6)
    } else {
        white(0.08)
    };
    let kind = sv(node, "type").unwrap_or("");
    let text_col = div()
        .col()
        .grow()
        .min_w0()
        .child(
            text(label, 13.0, c(look::WHITE))
                .weight(500)
                .line_height(18.0)
                .truncate()
                .silent(),
        )
        .child(
            text(wf::subtitle(node), 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .truncate()
                .silent(),
        );
    let mut el = div()
        .id(cid)
        .on(Action::Click)
        .col()
        .w(280.0)
        .px(12.0)
        .py(10.0)
        .rounded(6.0)
        .border(1.0, border)
        .bg(c(look::SURFACE_SUNKEN).mix(white(0.02), hot * 0.5))
        .role(Role::Button, label)
        .selected(selected)
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(ico(wf::card_icon(node), 16.0, 1.5, c(look::GRAY_400)))
                .child(text_col),
        );
    if let Some(f) = wf::footer(node) {
        el = el.child(
            div().mt(8.0).pt(8.0).border_top(1.0, white(0.06)).child(
                text(f, 11.0, c(look::GRAY_600))
                    .mono()
                    .line_height(14.0)
                    .truncate(),
            ),
        );
    }
    if let Some(rgb) = status.and_then(exec_dot) {
        el = el.child(
            div()
                .absolute_right(8.0, 8.0)
                .size(6.0, 6.0)
                .rounded(3.0)
                .bg(c(rgb))
                .role(Role::Image, status.unwrap_or("")),
        );
    }
    if hovering {
        el = el.child(hover_tools(s, d, id, kind, &tools));
    }
    el
}

/// `WORKFLOW_STATUS_DOT_PULSE`'s colours.
fn exec_dot(status: &str) -> Option<crate::look::Rgb> {
    match status {
        "running" => Some(look::YELLOW_500),
        "success" => Some(look::GREEN_500),
        "error" => Some(look::RED_500),
        "waiting" => Some(look::AMBER_400),
        _ => None,
    }
}

/// `NodeHoverToolbar`: run to here, replace and delete, beside the card.
fn hover_tools(s: &Screen<'_>, d: &Draft, id: &str, kind: &str, ids: &[Id; 3]) -> El {
    let trigger = kind == "trigger";
    let body = d.nodes().iter().any(|n| {
        n.pointer("/config/bodyNodeIds")
            .and_then(Value::as_array)
            .is_some_and(|b| b.iter().any(|x| x.as_str() == Some(id)))
    });
    let replaceable = matches!(
        kind,
        "trigger" | "launchAgent" | "script" | "httpRequest" | "approval"
    );
    let button = |bid: Id, a: WfAct, icon: &str, label: &str| {
        s.acts.bind(bid, act(a));
        let h = s.ui.hover(bid);
        let b = div()
            .id(bid)
            .on(Action::Click)
            .p(6.0)
            .rounded(6.0)
            .bg(c(look::SURFACE_OVERLAY))
            .border(1.0, white(0.12).mix(white(0.20), h))
            .role(Role::Button, label)
            .child(ico(
                icon,
                12.0,
                2.0,
                c(look::GRAY_400).mix(c(look::WHITE), h),
            ));
        tooltip(s.ui, b, bid, label, None)
    };
    let mut col = div().col().gap(4.0).absolute(286.0, 0.0);
    if !trigger && !body {
        col = col.child(button(
            ids[0],
            WfAct::RunTo(id.to_owned()),
            "step-forward",
            "Run to this step",
        ));
    }
    if replaceable {
        let at = if trigger {
            Anchor {
                trigger: true,
                replace: Some(id.to_owned()),
                ..Anchor::default()
            }
        } else {
            Anchor {
                after: Some(id.to_owned()),
                replace: Some(id.to_owned()),
                ..Anchor::default()
            }
        };
        col = col.child(button(
            ids[1],
            WfAct::Library(at),
            "replace",
            "Replace step",
        ));
    }
    col.child(button(
        ids[2],
        WfAct::DeleteNode(id.to_owned()),
        "trash-2",
        "Delete step",
    ))
}
