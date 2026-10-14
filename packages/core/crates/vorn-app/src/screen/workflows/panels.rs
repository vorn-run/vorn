//! The editor's right panels: `StepLibrary`, `NodeConfigPanel`,
//! `WorkflowPropertiesPanel`, `StartFromPanel` and `RunHistoryPanel`.

use serde_json::Value;

use super::act;
use crate::look;
use crate::screen::runs::{run_entry, Controls};
use crate::screen::{c, ico, white, Screen};
use crate::ui::{div, field, text, Action, El, Id, Rgba, Role, TextStyle};
use crate::workflows::{self as wf, Anchor, Draft, Panel, WfAct};

/// The panel the editor shows on its right, if any.
pub fn panel(s: &Screen<'_>, d: &Draft) -> Option<El> {
    Some(match &s.st.wf.panel {
        Panel::None => return None,
        Panel::Library(at) => library(s, at),
        Panel::Node => match s.st.wf.node.as_deref().and_then(|n| d.node(n)) {
            Some(n) => super::forms::node_panel(s, d, n),
            None => super::forms::properties(s, d),
        },
        Panel::Properties => super::forms::properties(s, d),
        Panel::StartFrom => start_from(s),
        Panel::History => history(s, d),
    })
}

fn shell(w: f32, label: &str) -> El {
    div()
        .col()
        .w(w)
        .shrink0()
        .h_full()
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border_left(1.0, white(0.08))
        .role(Role::Group, label)
}

fn close(s: &Screen<'_>, place: &str, label: &str) -> El {
    let id = s.acts.on(place, act(WfAct::ClosePanel));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(6.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, label)
        .child(ico(
            "x",
            14.0,
            2.0,
            c(look::GRAY_500).mix(c(look::WHITE), hot),
        ))
}

fn title(label: &str) -> El {
    text(label, 13.0, c(look::WHITE))
        .weight(500)
        .line_height(19.5)
}

fn header(s: &Screen<'_>, label: &str, close_label: &str, px: f32, py: f32) -> El {
    div()
        .row()
        .items_center()
        .justify_between()
        .px(px)
        .py(py)
        .border_bottom(1.0, white(0.08))
        .child(title(label))
        .child(close(s, "wf-panel-close", close_label))
}

/// `ConnectorIcon`: a connector's own glyph or a built-in mark, in the
/// caller's `color`.
pub fn mark(look: Option<&crate::library::Look>, size: f32, color: Rgba) -> El {
    match look.and_then(crate::library::mark_svg) {
        Some(svg) if look.is_some_and(|l| !l.connector.is_empty()) => {
            crate::ui::icon(svg, size, color)
        }
        _ => ico("zap", size, 2.0, Rgba::hexa(look::INK, look::INK_SECONDARY)),
    }
}

/// `StepLibrary`: the steps or triggers, Recent, then each connector's
/// group, the highlighted line answering Enter.
fn library(s: &Screen<'_>, at: &Anchor) -> El {
    let label = if at.trigger {
        "Add a trigger"
    } else if at.replace.is_some() {
        "Replace step"
    } else {
        "Add a step"
    };
    let placeholder = if at.trigger {
        "Search triggers"
    } else {
        "Search steps and actions"
    };
    let search = div()
        .row()
        .items_center()
        .gap(6.0)
        .px(8.0)
        .py(6.0)
        .rounded(6.0)
        .border(1.0, white(0.08))
        .frames(wf::search_id())
        .child(ico("search", 12.0, 2.0, c(look::GRAY_500)))
        .child(
            field(
                wf::search_id(),
                placeholder,
                placeholder,
                TextStyle {
                    line_height: 18.0,
                    ..TextStyle::ui(12.0)
                },
                c(look::WHITE),
            )
            .grow(),
        );
    let top = div()
        .col()
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.08))
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .pb(10.0)
                .child(title(label))
                .child(close(s, "wf-panel-close", "Close")),
        )
        .child(search);
    let lib = &s.st.wf.lib;
    let src = crate::library::Sources {
        connections: &s.st.wf.connections,
        actions: &lib.actions,
        catalog: &lib.catalog,
        packs: &lib.packs,
        connectors: &lib.connectors,
    };
    let entries = crate::library::entries(&src, at.into(), &s.st.wf.search, &lib.recent, &lib.open);
    let reach = entries.iter().filter(|e| e.reachable()).count();
    let lit = lib.highlight.min(reach.saturating_sub(1));
    let secondary = Rgba::hexa(look::INK, look::INK_SECONDARY);
    let mut list = div().col().p(8.0);
    if reach == 0 {
        list = list.child(
            div()
                .px(10.0)
                .py(12.0)
                .child(text("Nothing matches", 11.0, c(look::GRAY_500)).line_height(16.5)),
        );
    }
    let mut index = 0;
    let mut first = true;
    for e in &entries {
        use crate::library::Entry;
        match e {
            Entry::Heading(h) => {
                list = list.child(
                    div()
                        .px(8.0)
                        .pt(if first { 4.0 } else { 12.0 })
                        .pb(4.0)
                        .child(
                            text(h.to_uppercase(), 10.0, c(look::GRAY_600))
                                .mono()
                                .tracking(0.05)
                                .line_height(15.0),
                        ),
                );
            }
            Entry::Group {
                key,
                label,
                look: lk,
                detail,
                count,
                open,
            } => {
                let id = s
                    .acts
                    .on("wf-library-group", act(WfAct::Group(key.clone())));
                let on = index == lit;
                let on = on || s.ui.hovered(id);
                let fg = if on { c(look::WHITE) } else { secondary };
                let mut meta = div().row().items_center().gap(8.0).shrink0();
                if let Some(d) = detail {
                    meta = meta.child(text(*d, 10.0, c(look::GRAY_600)).line_height(15.0).silent());
                }
                meta = meta.child(
                    text(count.to_string(), 10.0, c(look::GRAY_600))
                        .mono()
                        .line_height(15.0)
                        .silent(),
                );
                list =
                    list.child(
                        div()
                            .id(id)
                            .on(Action::Click)
                            .row()
                            .items_center()
                            .gap(8.0)
                            .px(8.0)
                            .py(6.0)
                            .rounded(6.0)
                            .bg(if on { white(0.06) } else { white(0.0) })
                            .role(Role::Button, label)
                            .expanded(*open)
                            .child(ico(
                                if *open {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                12.0,
                                2.0,
                                c(look::GRAY_500),
                            ))
                            .child(mark(lk.as_ref(), 14.0, c(look::INK)))
                            .child(div().grow().min_w0().child(
                                text(label, 12.5, fg).line_height(18.75).truncate().silent(),
                            ))
                            .child(meta),
                    );
                index += 1;
            }
            Entry::Row {
                key,
                label,
                pick,
                icon,
                look: lk,
                owner,
                nested,
                ..
            } => {
                let id = s.acts.on("wf-library", act(WfAct::Pick(pick.clone())));
                let _ = key;
                let on = index == lit || s.ui.hovered(id);
                let fg = if on {
                    c(look::WHITE)
                } else {
                    c(look::GRAY_300)
                };
                let mut r = div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(10.0)
                    .pl(if *nested { 32.0 } else { 8.0 })
                    .pr(8.0)
                    .py(6.0)
                    .rounded(6.0)
                    .bg(if on { white(0.06) } else { white(0.0) })
                    .role(Role::Button, label);
                if let Some(i) = icon {
                    r = r.child(ico(i, 14.0, 2.0, secondary));
                } else if !nested {
                    r = r.child(mark(lk.as_ref(), 14.0, c(look::INK)));
                }
                r = r.child(
                    div()
                        .grow()
                        .min_w0()
                        .child(text(label, 12.5, fg).line_height(18.75).truncate().silent()),
                );
                if let (false, Some(o)) = (*nested, owner) {
                    r = r.child(
                        div().max_w(100.0).shrink0().child(
                            text(o, 10.0, c(look::GRAY_600))
                                .line_height(15.0)
                                .truncate()
                                .silent(),
                        ),
                    );
                }
                list = list.child(r);
                index += 1;
            }
        }
        first = false;
    }
    shell(280.0, label).child(top).child(
        div()
            .col()
            .grow()
            .min_h(0.0)
            .scroll_y()
            .id(Id::new("wf-library-list"))
            .child(list),
    )
}

/// A start-from row: an icon or strip, a name and a gray line.
fn pick_row(
    s: &Screen<'_>,
    place: &str,
    a: WfAct,
    lead: El,
    name: &str,
    sub: &str,
    inner: bool,
) -> El {
    let id = s.acts.on(place, act(a));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .w_full()
        .gap(10.0)
        .px(10.0)
        .py(8.0)
        .rounded(if inner { 0.0 } else { 6.0 })
        .bg(white(0.0).mix(white(0.06), if inner { 0.0 } else { hot }))
        .role(Role::Button, name)
        .child(lead)
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    text(name, 12.0, c(look::GRAY_300).mix(c(look::WHITE), hot))
                        .weight(500)
                        .line_height(18.0)
                        .truncate()
                        .silent(),
                )
                .child(
                    text(sub, 11.0, c(look::GRAY_500))
                        .line_height(16.5)
                        .truncate()
                        .silent(),
                ),
        )
}

fn group_label(label: &str) -> El {
    div().px(8.0).pt(12.0).pb(4.0).child(
        text(label, 10.0, c(look::GRAY_600))
            .mono()
            .tracking(0.05)
            .line_height(15.0)
            .silent(),
    )
}

/// `StepStrip`: the shape of the flow, drawn rather than counted.
fn step_strip(count: usize) -> El {
    let mut strip = div().col().w(22.0).mt(2.0).gap(3.0).shrink0().opacity(0.7);
    for _ in 0..count.min(4) {
        strip = strip.child(div().w_full().h(3.0).rounded(1.0).bg(white(0.18)));
    }
    strip
}

/// `RequirementRow`: what one unmet requirement says, and its fix.
fn requirement_row(s: &Screen<'_>, need: &crate::templates::Need) -> El {
    let l = crate::templates::Listings {
        connectors: &s.st.wf.lib.connectors,
        catalog: &s.st.wf.lib.catalog,
        packs: &s.st.wf.lib.packs,
    };
    let fix = crate::templates::fix(need, l);
    let mut row = div()
        .row()
        .items_center()
        .gap(8.0)
        .pl(38.0)
        .pr(10.0)
        .pb(6.0)
        .child(
            div().grow().min_w0().child(
                text(
                    format!("Needs {}", crate::templates::describe(&need.req)),
                    11.0,
                    c(look::BRONZO),
                )
                .line_height(16.5)
                .truncate(),
            ),
        );
    if let Some(label) = fix.label() {
        let place = format!("wf-fix.{}", s_of(&need.req, "nodeId"));
        let id = s.acts.on(&place, act(WfAct::Fix(fix)));
        let hot = s.ui.hover(id);
        row = row.child(
            div()
                .id(id)
                .on(Action::Click)
                .shrink0()
                .px(8.0)
                .py(2.0)
                .rounded(2.0)
                .border(1.0, white(0.1))
                .bg(white(0.0).mix(white(0.06), hot))
                .role(Role::Button, &label)
                .child(
                    text(&label, 11.0, c(look::GRAY_300).mix(c(look::WHITE), hot))
                        .line_height(16.5)
                        .silent(),
                ),
        );
    }
    row
}

fn s_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `StartFromPanel`: blank first, then the templates and what each still
/// wants, then the workflows this machine's connections ship with.
fn start_from(s: &Screen<'_>) -> El {
    let blank = pick_row(
        s,
        "wf-start-blank",
        WfAct::StartBlank,
        div()
            .mt(2.0)
            .child(ico("file-plus-2", 14.0, 1.5, c(look::GRAY_500))),
        "Blank canvas",
        "Pick your own trigger",
        false,
    );
    let mut list = div().col().p(8.0).child(blank);
    let templates = &s.st.wf.lib.templates;
    if !templates.is_empty() {
        list = list.child(group_label("TEMPLATES"));
    }
    for (i, t) in templates.iter().enumerate() {
        let needs = crate::templates::of_template(t, &s.st.wf.connections);
        let steps: Vec<&str> = t
            .get("steps")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let count = t
            .pointer("/portable/nodes")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let place = format!("wf-template.{}", s_of(t, "id"));
        let mut row = pick_row(
            s,
            &place,
            WfAct::Template(i),
            step_strip(count),
            s_of(t, "name"),
            &steps.join(" \u{b7} "),
            true,
        );
        let pending: Vec<_> = needs.iter().filter(|n| n.bound.is_none()).collect();
        if pending.is_empty() && !needs.is_empty() {
            row = row.child(
                div()
                    .row()
                    .items_center()
                    .gap(4.0)
                    .child(ico("check", 10.0, 2.0, c(crate::tasks::SAGE)))
                    .child(text("Connected", 11.0, c(crate::tasks::SAGE)).line_height(16.5)),
            );
        }
        let hot =
            s.ui.hover(crate::act::Acts::id(&place, &act(WfAct::Template(i))));
        let mut wrap = div()
            .col()
            .rounded(6.0)
            .bg(white(0.0).mix(white(0.06), hot))
            .child(row);
        for need in pending {
            wrap = wrap.child(requirement_row(s, need));
        }
        list = list.child(wrap);
    }
    let suggestions = crate::templates::suggestions(&s.st.wf.connections, &s.st.wf.lib.connectors);
    if !suggestions.is_empty() {
        list = list.child(group_label("FROM YOUR CONNECTIONS"));
    }
    for sg in suggestions {
        list = list.child(pick_row(
            s,
            &format!("wf-suggest.{}", sg.key),
            WfAct::Suggest(sg.key.clone()),
            div()
                .mt(2.0)
                .child(ico("plug", 14.0, 1.5, c(look::GRAY_500))),
            &sg.name,
            &sg.connection_name,
            false,
        ));
    }
    shell(280.0, "Start from")
        .child(header(s, "Start from", "Close", 16.0, 12.0))
        .children(crate::screen::plugins::editor_fix(s))
        .child(
            div()
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .id(Id::new("wf-start-list"))
                .child(list),
        )
}

/// The "Still needs" panel beside the properties, when anything is unbound.
pub fn needs(s: &Screen<'_>) -> Option<El> {
    let needs = s.st.wf.still_needed();
    if needs.is_empty() {
        return None;
    }
    let editing = s.st.wf.editor.as_ref().map(Draft::id);
    let dismissable =
        s.st.wf
            .imported
            .as_ref()
            .is_some_and(|(id, _)| Some(id.as_str()) == editing);
    let mut head = div()
        .row()
        .items_center()
        .justify_between()
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.08))
        .child(title("Still needs"));
    if dismissable {
        let id = s.acts.on("wf-needs-dismiss", act(WfAct::DismissNeeds));
        let hot = s.ui.hover(id);
        head = head.child(
            div()
                .id(id)
                .on(Action::Click)
                .p(4.0)
                .rounded(6.0)
                .bg(white(0.0).mix(white(0.06), hot))
                .role(Role::Button, "Dismiss")
                .child(ico(
                    "x",
                    14.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::WHITE), hot),
                )),
        );
    }
    let mut list = div().col().py(8.0);
    for need in &needs {
        list = list.child(requirement_row(s, need));
    }
    Some(
        shell(280.0, "Still needs")
            .child(head)
            .children(crate::screen::plugins::editor_fix(s))
            .child(
                div()
                    .col()
                    .grow()
                    .min_h(0.0)
                    .scroll_y()
                    .id(Id::new("wf-needs"))
                    .child(list),
            ),
    )
}

/// `RunHistoryPanel`.
fn history(s: &Screen<'_>, d: &Draft) -> El {
    let nodes = d.wf.get("nodes").cloned().unwrap_or(Value::Null);
    let mut list = div().col().gap(8.0).p(12.0);
    if s.st.wf.history.is_empty() {
        list = list.child(
            div()
                .row()
                .justify_center()
                .py(32.0)
                .child(text("No runs yet", 12.0, c(look::GRAY_600)).line_height(16.0)),
        );
    }
    for run in &s.st.wf.history {
        list = list.child(run_entry(
            s,
            run,
            &nodes,
            None,
            Controls {
                rerun: true,
                retry: true,
            },
        ));
    }
    shell(340.0, "Run History")
        .child(header(s, "Run History", "Close", 16.0, 12.0))
        .child(
            div()
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .id(Id::new("wf-history"))
                .child(list),
        )
}
