//! `WorkflowEditor`: the 40 px bar, the canvas of cards and the right panel.

use super::{act, wf_glyph};
use crate::act::Act;
use crate::look;
use crate::screen::chrome::{draggable, nav_cluster, window_controls};
use crate::screen::{c, ico, white, Screen};
use crate::state::{now_ms, MenuKind};
use crate::ui::{div, field, text, tooltip, Action, El, Id, Role, TextStyle};
use crate::workflows::{self as wf, Draft, Panel, WfAct};

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
        .child(super::canvas::canvas(s, d))
        .child_if(super::panels::needs(s))
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
    let mut left = div().row().items_center().gap(4.0).min_w0();
    if !s.st.sidebar {
        left = left.child(nav_cluster(s));
    }
    let name_id = wf::field_id(None, "name");
    let hot = if s.ui.focused(name_id) {
        0.06
    } else {
        0.04 * s.ui.hover(name_id)
    };
    let picker_id = s.acts.on("wf-editor", Act::Open(MenuKind::WfIcon));
    let picker_open =
        s.st.menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::WfIcon);
    let picker = div()
        .id(picker_id)
        .on(Action::Click)
        .p(6.0)
        .rounded(6.0)
        .bg(white(0.0).mix(
            white(0.08),
            if picker_open {
                1.0
            } else {
                s.ui.hover(picker_id)
            },
        ))
        .role(Role::Button, "Change icon")
        .has_popup()
        .expanded(picker_open)
        .child(wf_glyph(&d.wf, 16.0));
    left = left
        .child(tooltip(s.ui, picker, picker_id, "Change icon", None))
        .child(
            field(
                name_id,
                "Workflow name",
                "Workflow name",
                TextStyle {
                    weight: 500,
                    line_height: 22.5,
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
        None if s.st.wf.held_off(d.id()).is_some() => {
            let id = s.acts.on("wf-editor", act(WfAct::TurnOn(d.id().to_owned())));
            let hot = s.ui.hover(id);
            let b = div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(10.0)
                .py(6.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.06), hot))
                .role(Role::Button, "Turn on")
                .child(ico(
                    "power",
                    13.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::WHITE), hot),
                ))
                .child(
                    text("Turn on", 12.0, c(look::GRAY_300).mix(c(look::WHITE), hot))
                        .weight(500)
                        .line_height(16.0)
                        .silent(),
                );
            let tip = "This workflow comes from a project file and runs nothing until you turn it on";
            right = right.child(tooltip(s.ui, b, id, tip, None));
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
        .pl(s.lead())
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
