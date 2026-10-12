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
        Panel::Needs => super::forms::properties(s, d),
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
        .line_height(18.0)
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

/// A connector's mark at `size`: its own glyph in full ink, or a built-in
/// mark.
pub fn mark(look: Option<&crate::library::Look>, size: f32) -> El {
    match look.and_then(crate::library::mark_svg) {
        Some(svg) if look.is_some_and(|l| !l.connector.is_empty()) => {
            crate::ui::icon(svg, size, Rgba::hex(look::INK))
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
                            .child(mark(lk.as_ref(), 14.0))
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
                    r = r.child(mark(lk.as_ref(), 14.0));
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

fn start_from(s: &Screen<'_>) -> El {
    let id = s.acts.on("wf-start-blank", act(WfAct::StartBlank));
    let hot = s.ui.hover(id);
    let blank = div()
        .id(id)
        .on(Action::Click)
        .row()
        .gap(10.0)
        .px(10.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.0).mix(white(0.06), hot))
        .role(Role::Button, "Blank canvas")
        .child(
            div()
                .mt(2.0)
                .child(ico("file-plus-2", 14.0, 1.5, c(look::GRAY_500))),
        )
        .child(
            div()
                .col()
                .min_w0()
                .child(
                    text(
                        "Blank canvas",
                        12.0,
                        c(look::GRAY_300).mix(c(look::WHITE), hot),
                    )
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
                )
                .child(
                    text("Pick your own trigger", 11.0, c(look::GRAY_500))
                        .line_height(15.0)
                        .silent(),
                ),
        );
    shell(280.0, "Start from")
        .child(header(s, "Start from", "Close", 16.0, 12.0))
        .child(div().col().p(8.0).child(blank))
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
