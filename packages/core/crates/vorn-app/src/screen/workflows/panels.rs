//! The editor's right panels: `StepLibrary`, `NodeConfigPanel`,
//! `WorkflowPropertiesPanel`, `StartFromPanel` and `RunHistoryPanel`.

use serde_json::Value;

use super::act;
use crate::act::Act;
use crate::look;
use crate::runs::s as sv;
use crate::screen::runs::{run_entry, status_dot, Controls};
use crate::screen::tasks::switch;
use crate::screen::{c, ico, white, Screen};
use crate::state::MenuKind;
use crate::ui::{area, div, field, text, Action, El, Id, Rgba, Role, TextStyle};
use crate::workflows::{self as wf, Anchor, Draft, Field, Panel, WfAct};

/// The panel the editor shows on its right, if any.
pub fn panel(s: &Screen<'_>, d: &Draft) -> Option<El> {
    Some(match &s.st.wf.panel {
        Panel::None => return None,
        Panel::Library(at) => library(s, at),
        Panel::Node => match s.st.wf.node.as_deref().and_then(|n| d.node(n)) {
            Some(n) => node(s, n),
            None => properties(s, d),
        },
        Panel::Properties => properties(s, d),
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

fn heading(label: &str) -> El {
    div().px(8.0).pt(12.0).pb(4.0).child(
        text(label.to_uppercase(), 10.0, c(look::GRAY_600))
            .mono()
            .line_height(14.0),
    )
}

/// `StepLibrary`.
fn library(s: &Screen<'_>, at: &Anchor) -> El {
    let label = if at.trigger && at.replace.is_none() {
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
                TextStyle::ui(12.0),
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
    let rows = super::library_rows(&s.st.wf);
    let mut list = div().col().p(8.0);
    if rows.is_empty() {
        list = list.child(
            div()
                .px(10.0)
                .py(12.0)
                .child(text("Nothing matches", 11.0, c(look::GRAY_500)).line_height(14.0)),
        );
    } else {
        list = list.child(heading(if at.trigger { "Triggers" } else { "Steps" }));
        for (kind, name, icon) in rows {
            let id = s.acts.on("wf-library", act(WfAct::Pick(kind.to_owned())));
            let hot = s.ui.hover(id);
            list = list.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(10.0)
                    .px(8.0)
                    .py(6.0)
                    .rounded(6.0)
                    .bg(white(0.0).mix(white(0.06), hot))
                    .role(Role::Button, name)
                    .child(ico(icon, 14.0, 1.5, c(look::GRAY_400)))
                    .child(
                        text(name, 12.5, c(look::GRAY_300).mix(c(look::WHITE), hot))
                            .line_height(18.0)
                            .truncate()
                            .silent(),
                    ),
            );
        }
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

fn label(t: &str) -> El {
    div().pb(8.0).child(
        text(t, 13.0, c(look::GRAY_400))
            .weight(500)
            .line_height(18.0),
    )
}

fn help(t: &str) -> El {
    div().mt(6.0).child(
        text(t, 11.0, c(look::GRAY_500))
            .line_height(15.0)
            .wrapping()
            .min_w0(),
    )
}

/// `FIELD_CLASS`: the boxed text field.
fn boxed(s: &Screen<'_>, id: Id, el: El, bad: bool) -> El {
    let focused = s.ui.focused(id);
    let border = if bad {
        Rgba::hexa(look::DANGER, 0.6)
    } else if focused {
        white(0.20)
    } else {
        white(0.10)
    };
    div()
        .px(10.0)
        .py(6.0)
        .rounded(6.0)
        .bg(white(0.06))
        .border(1.0, border)
        .child(el.w_full())
}

fn text_box(s: &Screen<'_>, id: Id, placeholder: &str, mono: bool, bad: bool) -> El {
    let style = if mono {
        TextStyle::mono(12.0)
    } else {
        TextStyle::ui(12.0)
    };
    boxed(
        s,
        id,
        field(id, placeholder, placeholder, style, c(look::WHITE)),
        bad,
    )
}

/// `SelectPicker`'s form variant: the current label and a chevron.
fn select(s: &Screen<'_>, path: &str, shown: &str) -> El {
    let kind = MenuKind::WfChoice(path.to_owned());
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("wf-select", Act::Open(kind));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .justify_between()
        .gap(8.0)
        .px(10.0)
        .py(6.0)
        .rounded(6.0)
        .bg(white(0.06).mix(white(0.08), hot))
        .border(1.0, if open { white(0.20) } else { white(0.10) })
        .role(Role::Button, shown)
        .has_popup()
        .expanded(open)
        .child(
            text(shown, 12.0, c(look::GRAY_200))
                .line_height(16.0)
                .truncate()
                .silent(),
        )
        .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_500)))
}

fn shown_choice(s: &Screen<'_>, node: &Value, path: &str) -> String {
    let cur = wf::field_text(node, path);
    match path {
        "config.projectFilter" if cur.is_empty() => "All projects".into(),
        "config.projectName" if cur.is_empty() => "Select project".into(),
        "config.projectName" | "config.projectFilter" => cur,
        "onError" if cur.is_empty() => wf::choice_label(path, "stop"),
        "config.cron" => wf::choices(path)
            .into_iter()
            .find(|(v, _)| *v == cur)
            .map_or_else(|| "Custom".into(), |(_, l)| l.to_owned()),
        "config.agentType" => {
            let v = if cur.is_empty() {
                s.store
                    .config
                    .pointer("/defaults/defaultAgent")
                    .and_then(Value::as_str)
                    .unwrap_or("claude")
                    .to_owned()
            } else {
                cur
            };
            wf::choice_label(path, &v)
        }
        _ => wf::choice_label(path, &cur),
    }
}

fn toggle_row(s: &Screen<'_>, a: WfAct, on: bool, name: &str, hint: &str) -> El {
    let id = s.acts.on("wf-switch", act(a));
    let hot = s.ui.hover(id);
    let border = if on {
        white(0.10)
    } else {
        white(0.04).mix(white(0.10), hot)
    };
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .w_full()
        .px(12.0)
        .py(10.0)
        .rounded(8.0)
        .border(1.0, border)
        .bg(if on { white(0.04) } else { white(0.02) })
        .role(Role::Switch, name)
        .toggled(on)
        .child(switch(on))
        .child(
            div()
                .col()
                .grow()
                .min_w0()
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(6.0)
                        .child(ico(
                            "zap",
                            12.0,
                            2.0,
                            c(if on { look::GRAY_300 } else { look::GRAY_500 }),
                        ))
                        .child(
                            text(
                                name,
                                12.0,
                                c(if on { look::GRAY_200 } else { look::GRAY_400 }),
                            )
                            .line_height(16.0)
                            .silent(),
                        ),
                )
                .child(
                    div().mt(2.0).child(
                        text(hint, 11.0, c(look::GRAY_500))
                            .line_height(15.0)
                            .wrapping()
                            .min_w0()
                            .silent(),
                    ),
                ),
        )
}

/// `TriggerConfigForm`'s kind box, hint and library link.
fn trigger_kind(s: &Screen<'_>, node: &Value) -> El {
    let kind = node
        .pointer("/config/triggerType")
        .and_then(Value::as_str)
        .unwrap_or("manual");
    let (name, hint) = wf::trigger_info(kind);
    let id = Some(sv(node, "id").unwrap_or("").to_owned());
    let at = Anchor {
        trigger: true,
        replace: id,
        ..Anchor::default()
    };
    let link = s.acts.on("wf-trigger-link", act(WfAct::Library(at)));
    let hot = s.ui.hover(link);
    div()
        .col()
        .child(label("Trigger Type"))
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(8.0)
                .rounded(8.0)
                .border(1.0, white(0.06))
                .bg(white(0.02))
                .child(ico(wf::trigger_icon(kind), 13.0, 2.0, c(look::GRAY_400)))
                .child(text(name, 13.0, c(look::GRAY_200)).line_height(18.0)),
        )
        .child(help(hint))
        .child(
            div()
                .id(link)
                .on(Action::Click)
                .mt(8.0)
                .role(Role::Link, "Change trigger from the library")
                .child(
                    text(
                        "Change trigger from the library",
                        11.0,
                        c(look::GRAY_500).mix(c(look::GRAY_300), hot),
                    )
                    .line_height(15.0)
                    .silent(),
                ),
        )
}

/// `WorkflowInputsEditor`.
fn inputs(s: &Screen<'_>, node: &Value) -> El {
    let nid = sv(node, "id");
    let defs: Vec<Value> = node
        .pointer("/config/inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let dup = wf::duplicate_keys(&defs);
    let mut list = div().col().gap(8.0);
    for (i, def) in defs.iter().enumerate() {
        let key = sv(def, "key").unwrap_or("");
        let bad = dup.iter().any(|k| k == key);
        let kind = sv(def, "type").unwrap_or("text");
        let rm = s.acts.on("wf-input-rm", act(WfAct::RemoveInput(i)));
        let rm_hot = s.ui.hover(rm);
        let mut card = div()
            .col()
            .gap(8.0)
            .p(10.0)
            .rounded(8.0)
            .border(1.0, white(0.06))
            .bg(white(0.02))
            .child(
                div()
                    .row()
                    .items_center()
                    .gap(8.0)
                    .child(
                        text_box(
                            s,
                            wf::field_id(nid, &format!("input.{i}.key")),
                            "key",
                            true,
                            bad,
                        )
                        .grow(),
                    )
                    .child(
                        div()
                            .id(rm)
                            .on(Action::Click)
                            .p(6.0)
                            .rounded(6.0)
                            .bg(white(0.0).mix(white(0.06), rm_hot))
                            .role(Role::Button, format!("Remove input {key}"))
                            .child(ico(
                                "trash-2",
                                12.0,
                                2.0,
                                c(look::GRAY_500).mix(c(look::DANGER), rm_hot),
                            )),
                    ),
            );
        if bad {
            card = card.child(
                text(
                    format!(
                        "Duplicate key \u{2014} only one input named {key} will reach the run."
                    ),
                    11.0,
                    c(look::DANGER),
                )
                .line_height(15.0)
                .wrapping()
                .min_w0(),
            );
        }
        card = card
            .child(text_box(
                s,
                wf::field_id(nid, &format!("input.{i}.label")),
                "Label shown in the run dialog",
                false,
                false,
            ))
            .child(select(
                s,
                &format!("input.{i}.type"),
                &wf::choice_label("input.type", kind),
            ));
        if kind == "select" {
            card = card.child(text_box(
                s,
                wf::field_id(nid, &format!("input.{i}.options")),
                "Comma-separated choices",
                false,
                false,
            ));
        }
        if kind != "boolean" && kind != "select" {
            card = card.child(text_box(
                s,
                wf::field_id(nid, &format!("input.{i}.defaultValue")),
                "Default value (optional)",
                false,
                false,
            ));
        }
        list = list.child(card);
    }
    let add = s.acts.on("wf-input-add", act(WfAct::AddInput));
    let add_hot = s.ui.hover(add);
    let fg = c(look::GRAY_400).mix(c(look::WHITE), add_hot);
    div()
        .col()
        .child(label("Run Inputs"))
        .child(
            div().pb(8.0).child(
                text(
                    "Values the user is asked for before the run starts. Use them anywhere as {{inputs.key}}.",
                    11.0,
                    c(look::GRAY_500),
                )
                .line_height(15.0)
                .wrapping()
.min_w0(),
            ),
        )
        .child(list)
        .child(
            div().row().mt(8.0).child(
                div()
                    .id(add)
                    .on(Action::Click)
                    .row()
                    .items_center()
                    .gap(6.0)
                    .px(10.0)
                    .py(6.0)
                    .rounded(6.0)
                    .bg(white(0.0).mix(white(0.06), add_hot))
                    .role(Role::Button, "Add input")
                    .child(ico("plus", 12.0, 2.0, fg))
                    .child(text("Add input", 12.0, fg).line_height(16.0).silent()),
            ),
        )
}

fn form_field(s: &Screen<'_>, node: &Value, f: &Field) -> El {
    let nid = sv(node, "id");
    match f {
        Field::Kind => trigger_kind(s, node),
        Field::Inputs => inputs(s, node),
        Field::Switch { path, label, help } => {
            let on = node
                .pointer(&format!("/{}", path.replace('.', "/")))
                .and_then(Value::as_bool)
                == Some(true);
            toggle_row(s, WfAct::Toggle(path.clone()), on, label, help)
        }
        Field::Choice {
            path,
            label: l,
            help: h,
        } => {
            let mut col =
                div()
                    .col()
                    .child(label(l))
                    .child(select(s, path, &shown_choice(s, node, path)));
            if !h.is_empty() {
                col = col.child(help(h));
            }
            col
        }
        Field::Text {
            path,
            label: l,
            placeholder,
            mono,
            lines,
            help: h,
        } => {
            let id = wf::field_id(nid, path);
            let input = if *lines {
                let style = if *mono {
                    TextStyle {
                        line_height: 18.0,
                        ..TextStyle::mono(12.0)
                    }
                } else {
                    TextStyle::ui(12.0)
                };
                boxed(
                    s,
                    id,
                    area(id, l, placeholder, style, c(look::WHITE)).h(if *mono {
                        180.0
                    } else {
                        120.0
                    }),
                    false,
                )
            } else {
                text_box(s, id, placeholder, *mono, false)
            };
            let mut col = div().col().child(label(l)).child(input);
            if !h.is_empty() {
                col = col.child(help(h));
            }
            col
        }
    }
}

/// `NodeConfigPanel`.
fn node(s: &Screen<'_>, node: &Value) -> El {
    let id = sv(node, "id").unwrap_or("").to_owned();
    let name_id = wf::field_id(Some(&id), "label");
    let hot = if s.ui.focused(name_id) {
        0.06
    } else {
        0.04 * s.ui.hover(name_id)
    };
    let kind = MenuKind::NodeMore(id.clone());
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let more = s.acts.on("wf-node-more", Act::Open(kind));
    let more_hot = s.ui.hover(more);
    let top = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(20.0)
        .py(12.0)
        .border_bottom(1.0, white(0.08))
        .child(ico(wf::card_icon(node), 14.0, 1.5, c(look::GRAY_400)))
        .child(
            field(
                name_id,
                "Label",
                "Label",
                TextStyle {
                    weight: 500,
                    ..TextStyle::ui(13.0)
                },
                c(look::WHITE),
            )
            .grow()
            .min_w0()
            .px(6.0)
            .py(2.0)
            .rounded(4.0)
            .bg(white(hot)),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(2.0)
                .child(
                    div()
                        .id(more)
                        .on(Action::Click)
                        .p(4.0)
                        .rounded(6.0)
                        .role(Role::Button, "More node actions")
                        .has_popup()
                        .expanded(open)
                        .child(ico(
                            "ellipsis",
                            14.0,
                            2.0,
                            c(look::GRAY_500)
                                .mix(c(look::WHITE), more_hot.max(if open { 1.0 } else { 0.0 })),
                        )),
                )
                .child(close(s, "wf-node-close", "Close node config")),
        );
    let mut body = div().col().gap(20.0).p(20.0);
    let fields = wf::form(node);
    let trigger = sv(node, "type") == Some("trigger");
    for (i, f) in fields.iter().enumerate() {
        let last = !trigger && i + 1 == fields.len();
        let el = form_field(s, node, f);
        body = body.child(if last {
            div()
                .col()
                .w_full()
                .pt(16.0)
                .border_top(1.0, white(0.08))
                .child(el)
        } else {
            el
        });
    }
    let label = sv(node, "label").unwrap_or("Step");
    shell(420.0, label).child(top).child(
        div()
            .id(Id::new("wf-node-body").child(id.as_str()))
            .col()
            .grow()
            .min_h(0.0)
            .scroll_y()
            .child(body),
    )
}

fn prop_row(name: &str, value: El) -> El {
    div()
        .row()
        .items_center()
        .min_h(32.0)
        .py(4.0)
        .child(
            div()
                .w(110.0)
                .shrink0()
                .child(text(name, 12.0, c(look::GRAY_500)).line_height(16.0)),
        )
        .child(div().row().items_center().grow().min_w0().child(value))
}

/// `formatTriggerSummary`.
fn trigger_summary(node: Option<&Value>) -> String {
    let Some(n) = node else {
        return "None".into();
    };
    let cfg = |k: &str| {
        n.pointer(&format!("/config/{k}"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    match cfg("triggerType").unwrap_or("") {
        "manual" => "Manual".into(),
        "once" => format!("Once \u{2014} {}", cfg("runAt").unwrap_or("not set")),
        "recurring" => format!("Recurring \u{2014} {}", cfg("cron").unwrap_or("not set")),
        "taskCreated" => "Task created".into(),
        "taskStatusChanged" => "Task status changed".into(),
        _ => "Unknown".into(),
    }
}

fn switch_button(s: &Screen<'_>, path: &str, on: bool, name: &str, disabled: bool) -> El {
    let mut b = div()
        .role(Role::Switch, name)
        .toggled(on)
        .opacity(if disabled { 0.5 } else { 1.0 })
        .child(switch(on));
    if !disabled {
        let id = s.acts.on("wf-prop", act(WfAct::Toggle(path.to_owned())));
        b = b.id(id).on(Action::Click);
    }
    b
}

/// `WorkflowPropertiesPanel`.
fn properties(s: &Screen<'_>, d: &Draft) -> El {
    let enabled = d.wf.get("enabled").and_then(Value::as_bool) != Some(false);
    let cleanup = d.wf.get("autoCleanupWorktrees").and_then(Value::as_bool) == Some(true);
    let agents: Vec<&Value> = d
        .nodes()
        .iter()
        .filter(|n| sv(n, "type") == Some("launchAgent"))
        .collect();
    let inherited = !agents.is_empty()
        && agents.iter().all(|n| {
            n.pointer("/config/inheritWorktree")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    let trigger = wf::trigger(d.nodes());
    let stagger_id = wf::field_id(None, "staggerDelayMs");
    let stagger = div()
        .w(80.0)
        .px(8.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.06))
        .border(
            1.0,
            if s.ui.focused(stagger_id) {
                white(0.20)
            } else {
                white(0.08)
            },
        )
        .child(
            field(
                stagger_id,
                "Stagger delay in milliseconds",
                "0ms",
                TextStyle::ui(12.0),
                c(look::GRAY_200),
            )
            .w_full(),
        );
    let trigger_btn = {
        let summary = trigger_summary(trigger);
        let a = match trigger.and_then(|t| sv(t, "id")) {
            Some(id) => act(WfAct::SelectNode(id.to_owned())),
            None => act(WfAct::Library(Anchor {
                trigger: true,
                ..Anchor::default()
            })),
        };
        let id = s.acts.on("wf-prop-trigger", a);
        let hot = s.ui.hover(id);
        div()
            .id(id)
            .on(Action::Click)
            .role(Role::Button, &summary)
            .child(
                text(
                    summary.clone(),
                    12.0,
                    c(look::GRAY_200).mix(c(look::WHITE), hot),
                )
                .line_height(16.0)
                .truncate()
                .silent(),
            )
    };
    let mut body = div()
        .col()
        .gap(4.0)
        .p(20.0)
        .child(prop_row(
            "Status",
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(
                    text(
                        if enabled { "Enabled" } else { "Disabled" },
                        12.0,
                        c(look::GRAY_200),
                    )
                    .line_height(16.0),
                )
                .child(switch_button(s, "enabled", enabled, "Enabled", false)),
        ))
        .child(prop_row("Stagger delay", stagger))
        .child(prop_row(
            "Cleanup worktrees",
            switch_button(
                s,
                "autoCleanupWorktrees",
                cleanup && !inherited,
                "Cleanup worktrees",
                inherited,
            ),
        ))
        .child(div().h(8.0))
        .child(prop_row("Trigger", trigger_btn));
    let last =
        s.st.wf
            .merged(&s.store.runs)
            .into_iter()
            .find(|r| sv(r, "workflowId") == Some(d.id()));
    if let Some(run) = last {
        let when = sv(&run, "startedAt")
            .map(|t| crate::runs::relative(t, crate::state::now_ms()))
            .unwrap_or_else(|| "Unknown".into());
        body = body.child(prop_row(
            "Last run",
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(status_dot(super::dot_status(&run)))
                .child(text(when, 12.0, c(look::GRAY_300)).line_height(16.0)),
        ));
    }
    shell(340.0, "Properties")
        .child(header(
            s,
            "Properties",
            "Close properties panel",
            20.0,
            16.0,
        ))
        .child(
            div()
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .id(Id::new("wf-props"))
                .child(body),
        )
}

/// `StartFromPanel`: the blank canvas; templates are not offered here.
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
