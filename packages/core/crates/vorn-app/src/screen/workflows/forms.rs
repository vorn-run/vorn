//! `NodeConfigPanel` and every step's form (`TriggerConfigForm`,
//! `LaunchAgentConfigForm`, `ScriptConfigForm`, `ConditionConfigForm`,
//! `LoopConfigForm`, `ApprovalConfigForm`, `HttpRequestConfigForm`,
//! `CallConnectorActionNodeForm`, `ConnectorPollTriggerForm`,
//! `CreateTaskFromItemNodeForm`), `WorkflowInputsEditor`,
//! `WorkflowPropertiesPanel`, and `VariableAutocomplete` under any field.

use serde_json::Value;

use super::{act, panels::mark};
use crate::act::Act;
use crate::app::forms::field_of;
use crate::look;
use crate::runs::{self, s as sv};
use crate::screen::menus::Item;
use crate::screen::{c, ico, white, Screen};
use crate::state::MenuKind;
use crate::ui::{div, field, text, textarea, tooltip, Action, El, Place, Rgba, Role, TextStyle};
use crate::workflows::{self as wf, Bind, Conv, Draft, WfAct};

fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

fn is_ctx(v: Option<&str>) -> bool {
    crate::vars::is_context_ref(v)
}

fn bind(node: Option<&str>, path: &str, conv: Conv, vars: bool) -> Bind {
    Bind {
        node: node.map(str::to_owned),
        path: path.to_owned(),
        conv,
        vars,
    }
}

/// `text-[13px] text-gray-400 font-medium block mb-2`.
fn label(t: &str) -> El {
    div().pb(8.0).child(
        text(t, 13.0, c(look::GRAY_400))
            .weight(500)
            .line_height(19.5),
    )
}

/// A label with a quieter "(optional)" after it.
fn label_opt(t: &str, opt: &str) -> El {
    div()
        .row()
        .gap(4.0)
        .pb(8.0)
        .child(
            text(t, 13.0, c(look::GRAY_400))
                .weight(500)
                .line_height(19.5),
        )
        .child(
            text(opt, 13.0, c(look::GRAY_600))
                .weight(500)
                .line_height(19.5),
        )
}

/// `text-[11px] text-gray-500`, `top` above it.
fn help(t: &str, top: f32) -> El {
    div().pt(top).child(
        text(t, 11.0, c(look::GRAY_500))
            .line_height(16.5)
            .wrapping()
            .min_w0(),
    )
}

/// `text-[11px] text-gray-600 mt-1.5`.
fn help_faint(t: &str) -> El {
    div().pt(6.0).child(
        text(t, 11.0, c(look::GRAY_600))
            .line_height(16.5)
            .wrapping()
            .min_w0(),
    )
}

/// The two field washes: `bg-white/[0.06] border-white/[0.1]` and the
/// quieter `bg-white/[0.03] border-white/[0.08]`.
#[derive(Clone, Copy, PartialEq)]
enum Wash {
    Strong,
    Quiet,
}

fn wash(w: Wash) -> (Rgba, Rgba) {
    match w {
        Wash::Strong => (white(0.06), white(0.10)),
        Wash::Quiet => (white(0.03), white(0.08)),
    }
}

/// A bound one-line field in a framed box.
#[allow(clippy::too_many_arguments)]
fn input(
    s: &Screen<'_>,
    b: Bind,
    value: &str,
    placeholder: &str,
    mono: bool,
    w: Wash,
    size: f32,
    px: f32,
) -> El {
    let id = field_of(&b);
    s.acts.text(id, Act::Workflow(WfAct::Text(b)), value);
    let (bg, border) = wash(w);
    let focus = s.ui.focused(id);
    let style = TextStyle {
        line_height: size * 1.5,
        ..if mono {
            TextStyle::mono(size)
        } else {
            TextStyle::ui(size)
        }
    };
    div()
        .w_full()
        .px(px)
        .py(if size < 13.0 { 6.0 } else { 8.0 })
        .rounded(6.0)
        .bg(bg)
        .border(1.0, if focus { white(0.2) } else { border })
        .child(
            field(id, placeholder, placeholder, style, c(look::WHITE))
                .w_full()
                .min_w0(),
        )
}

/// `VariableAutocomplete`: a template field with the `{{` button, the
/// preview of what its tokens resolve to, and the dropdown.
#[allow(clippy::too_many_arguments)]
fn var_area(
    s: &Screen<'_>,
    d: &Draft,
    b: Bind,
    value: &str,
    placeholder: &str,
    rows: usize,
    mono: bool,
) -> El {
    let id = field_of(&b);
    let node = b.node.clone().unwrap_or_default();
    let (groups, vars) = wf::var_sources(&s.st.wf, d, &node, &b.path);
    let has_vars = !crate::vars::items(&groups, &vars).is_empty();
    s.acts
        .text(id, Act::Workflow(WfAct::Text(b.clone())), value);
    let size = if mono { 12.0 } else { 13.0 };
    let line = size * 1.5;
    let style = TextStyle {
        line_height: line,
        ..if mono {
            TextStyle::mono(size)
        } else {
            TextStyle::ui(size)
        }
    };
    let focus = s.ui.focused(id);
    let open = s.st.wf.ac.as_ref().is_some_and(|a| a.bind == b);
    let mut boxed = div()
        .w_full()
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.06))
        .border(1.0, if focus { white(0.2) } else { white(0.10) })
        .child(
            textarea(id, placeholder, placeholder, style, c(look::WHITE))
                .w_full()
                .min_w0()
                .h(rows as f32 * line),
        );
    if has_vars {
        let bid = s.acts.on("wf-vars", act(WfAct::Vars(b.clone())));
        let hot = s.ui.hover(bid);
        let btn = div()
            .id(bid)
            .on(Action::Click)
            .absolute_right(6.0, 6.0)
            .p(4.0)
            .rounded(4.0)
            .bg(if open {
                white(0.10)
            } else {
                white(0.06).mix(white(0.10), hot)
            })
            .role(Role::Button, "Insert variable")
            .child(ico(
                "braces",
                13.0,
                2.0,
                if open {
                    ink(1.0)
                } else {
                    c(look::GRAY_500).mix(c(look::GRAY_300), hot)
                },
            ));
        boxed = boxed.child(tooltip(
            s.ui,
            btn,
            bid,
            "Insert variable (or type {{ in the editor)",
            None,
        ));
    }
    if open && has_vars {
        // As wide as the field, which the last frame measured.
        let w = s.ui.rect_of(id).map_or(380.0, |r| r.w + 26.0);
        boxed = boxed.dropdown(dropdown(s, &groups).w(w), Place::BelowStart, 4.0);
    }
    // A textarea sits on the text baseline, leaving a few pixels under it.
    let mut col = div().col().w_full().child(boxed).child(div().h(5.0));
    if value.contains("{{") {
        let p = crate::vars::preview(value, &groups);
        if let Some((token, suggestion)) = p.broken {
            let msg = match suggestion {
                Some(sug) => format!("{token} not found \u{2014} did you mean {sug}?"),
                None => format!("{token} not found"),
            };
            col = col.child(
                div().pt(4.0).child(
                    text(msg, 10.5, Rgba::hexa(look::DANGER, 0.9))
                        .mono()
                        .line_height(15.75)
                        .truncate(),
                ),
            );
        } else if let Some(r) = p.resolved {
            let flat: String = r.split_whitespace().collect::<Vec<_>>().join(" ");
            let cut: String = flat.chars().take(160).collect();
            col = col.child(
                div().pt(4.0).child(
                    text(format!("\u{2192} {cut}"), 10.5, ink(look::INK_FAINT))
                        .mono()
                        .line_height(15.75)
                        .truncate(),
                ),
            );
        }
    }
    col
}

/// The `{{` dropdown: the filter, then each group folding its variables.
fn dropdown(s: &Screen<'_>, groups: &[crate::vars::StepGroup]) -> El {
    let Some(ac) = &s.st.wf.ac else {
        return div();
    };
    let Some(d) = &s.st.wf.editor else {
        return div();
    };
    let node = ac.bind.node.clone().unwrap_or_default();
    let (_, vars) = wf::var_sources(&s.st.wf, d, &node, &ac.bind.path);
    let all = crate::vars::filtered(crate::vars::items(groups, &vars), &ac.filter);
    let visible: Vec<&crate::vars::Item> = all
        .iter()
        .filter(|i| !ac.collapsed.contains(&i.group_id))
        .collect();
    let mut list = div().col();
    if all.is_empty() {
        list =
            list.child(
                div().px(12.0).py(12.0).row().justify_center().child(
                    text("No matching variables", 12.0, c(look::GRAY_600)).line_height(18.0),
                ),
            );
    }
    let mut current = String::new();
    for item in &all {
        if item.group_id != current {
            current = item.group_id.clone();
            let collapsed = ac.collapsed.contains(&current);
            let gid = s
                .acts
                .on("wf-var-group", act(WfAct::VarGroup(current.clone())));
            let hot = s.ui.hover(gid);
            let chevron = if collapsed {
                "chevron-right"
            } else {
                "chevron-down"
            };
            let mut head = div()
                .id(gid)
                .on(Action::Click)
                .row()
                .items_center()
                .px(10.0)
                .py(6.0)
                .bg(white(0.0).mix(white(0.03), hot));
            match groups.iter().find(|g| g.slug == current) {
                Some(g) => {
                    let look = g.connection.as_deref().and_then(|cid| {
                        let src = crate::library::Sources {
                            connections: &s.st.wf.connections,
                            packs: &s.st.wf.lib.packs,
                            ..crate::library::Sources::default()
                        };
                        crate::library::look_for(&src, cid)
                    });
                    let glyph = match look {
                        Some(l) => mark(Some(&l), 12.0),
                        None => ico(
                            super::super::runs::node_icon(&g.kind),
                            12.0,
                            2.0,
                            secondary(),
                        ),
                    };
                    head = head
                        .gap(8.0)
                        .child(ico(chevron, 11.0, 2.0, c(look::GRAY_400)))
                        .child(glyph)
                        .child(
                            div().min_w0().child(
                                text(&g.label, 11.5, secondary())
                                    .weight(600)
                                    .line_height(17.25)
                                    .truncate()
                                    .silent(),
                            ),
                        );
                    if let Some(st) = &g.run_status {
                        let (rgb, a) = runs::dot(st);
                        head = head.child(div().size(6.0, 6.0).rounded(3.0).bg(Rgba::hexa(rgb, a)));
                    }
                    if let Some(at) = &g.run_completed {
                        head = head.child(div().grow()).child(
                            text(crate::runs::clock_time(at), 9.5, c(look::GRAY_600))
                                .mono()
                                .line_height(14.25)
                                .silent(),
                        );
                    }
                }
                None => {
                    head = head
                        .gap(6.0)
                        .child(ico(chevron, 11.0, 2.0, c(look::GRAY_500)))
                        .child(
                            text(
                                item.group.to_uppercase(),
                                10.0,
                                c(look::GRAY_500).mix(c(look::GRAY_400), hot),
                            )
                            .weight(600)
                            .tracking(0.05)
                            .line_height(15.0)
                            .silent(),
                        );
                }
            }
            list = list.child(head);
        }
        if ac.collapsed.contains(&item.group_id) {
            continue;
        }
        let i = visible
            .iter()
            .position(|v| *v == item)
            .unwrap_or(usize::MAX);
        let lit = i == ac.highlight;
        let iid = s
            .acts
            .on("wf-var", act(WfAct::Insert(item.pattern.clone())));
        let hot = s.ui.hover(iid);
        let name = if item.key == item.pattern {
            &item.label
        } else {
            &item.key
        };
        let mut row = div()
            .id(iid)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .pl(24.0)
            .pr(12.0)
            .py(6.0)
            .bg(if lit {
                white(0.08)
            } else {
                white(0.0).mix(white(0.04), hot)
            })
            .role(Role::MenuItem, name)
            .child(
                div().min_w(50.0).child(
                    text(name, 12.0, secondary())
                        .mono()
                        .line_height(18.0)
                        .silent(),
                ),
            );
        match &item.value {
            Some(v) => {
                row = row.child(div().grow()).child(
                    div().max_w(150.0).child(
                        text(v, 10.5, c(look::GRAY_500))
                            .mono()
                            .line_height(15.75)
                            .truncate()
                            .silent(),
                    ),
                );
            }
            None if !item.description.is_empty() => {
                row = row.child(
                    div().min_w0().child(
                        text(&item.description, 11.0, c(look::GRAY_600))
                            .line_height(16.5)
                            .truncate()
                            .silent(),
                    ),
                );
            }
            None => {}
        }
        list = list.child(row);
    }
    div()
        .col()
        .rounded(8.0)
        .border(1.0, white(0.12))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .layered()
        .role(Role::Menu, "Variables")
        .child(
            div()
                .px(10.0)
                .py(8.0)
                .border_bottom(1.0, white(0.08))
                .child(
                    div()
                        .w_full()
                        .px(8.0)
                        .py(4.0)
                        .rounded(4.0)
                        .bg(white(0.06))
                        .border(1.0, white(0.08))
                        .child(
                            text(
                                if ac.filter.is_empty() {
                                    "Filter variables..."
                                } else {
                                    ac.filter.as_str()
                                },
                                12.0,
                                if ac.filter.is_empty() {
                                    c(look::GRAY_600)
                                } else {
                                    c(look::GRAY_300)
                                },
                            )
                            .line_height(18.0),
                        ),
                ),
        )
        .child(
            div()
                .id(crate::ui::Id::new("wf-var-list"))
                .col()
                .max_h(230.0)
                .scroll_y()
                .child(list),
        )
}

/// A `SelectPicker` trigger (`variant="form"`), opening the choice menu
/// `token` names.
fn select(
    s: &Screen<'_>,
    token: String,
    label: Option<&str>,
    glyph: Option<El>,
    placeholder: &str,
    w: Wash,
) -> El {
    let kind = MenuKind::WfChoice(token);
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("wf-select", Act::Open(kind));
    let hot = s.ui.hover(id);
    let (bg, border) = wash(w);
    let mut b = div()
        .id(id)
        .on(Action::Click)
        .w_full()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(bg)
        .border(1.0, border.mix(white(0.2), hot))
        .role(Role::Button, label.unwrap_or(placeholder))
        .has_popup()
        .expanded(open);
    if let Some(g) = glyph {
        b = b.child(div().shrink0().child(g));
    }
    b.child(
        div().grow().min_w0().child(
            text(
                label.unwrap_or(placeholder),
                13.0,
                if label.is_some() {
                    if w == Wash::Quiet {
                        c(look::GRAY_200)
                    } else {
                        c(look::WHITE)
                    }
                } else {
                    c(look::GRAY_600)
                },
            )
            .line_height(19.5)
            .truncate()
            .silent(),
        ),
    )
    .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_500)))
}

/// The token a choice menu is named by: its width, node and path.
fn token(w: f32, node: Option<&str>, path: &str) -> String {
    format!("{w}|{}|{path}", node.unwrap_or(""))
}

fn parse_token(t: &str) -> (f32, Option<&str>, &str) {
    let mut it = t.splitn(3, '|');
    let w = it.next().and_then(|w| w.parse().ok()).unwrap_or(200.0);
    let node = it.next().filter(|n| !n.is_empty());
    (w, node, it.next().unwrap_or(""))
}

/// A choice menu's width: the trigger's, at least 180.
pub fn choice_width(t: &str) -> f32 {
    parse_token(t).0.max(180.0)
}

fn project_glyph(s: &Screen<'_>, name: &str, size: f32) -> El {
    match s.store.projects.iter().find(|p| p.name == name) {
        Some(p) => super::super::sidebar_project_icon(p, size),
        None => ico("folder", size, 1.5, c(0x6b7280)),
    }
}

/// `ProjectPicker` (`variant="form"`): the project, or From Context.
fn project(s: &Screen<'_>, node: &str, path: &str, current: &str, from_context: bool) -> El {
    let (label, glyph) = if from_context {
        (Some("From Context"), ico("zap", 13.0, 1.5, c(0x60a5fa)))
    } else if current.is_empty() {
        (None, ico("folder", 13.0, 1.5, c(0x6b7280)))
    } else {
        (Some(current), project_glyph(s, current, 13.0))
    };
    let mut b = select(
        s,
        token(380.0, Some(node), path),
        label,
        Some(glyph),
        "Select project...",
        Wash::Strong,
    );
    if from_context {
        b = b.role(Role::Button, "From Context");
    }
    b
}

/// The choices behind a menu token, as `SelectPicker` lists them.
pub fn choice_items(s: &Screen<'_>, t: &str) -> Vec<Item> {
    let (_, node, path) = parse_token(t);
    let n = node.and_then(|id| s.st.wf.editor.as_ref()?.node(id));
    let cfg = n.and_then(|n| n.get("config")).cloned().unwrap_or_default();
    let get = |k: &str| sv(&cfg, k).unwrap_or("").to_owned();
    let pick = |label: &str, value: &str, on: bool| {
        Item::pick(
            label,
            on,
            act(WfAct::Choose(
                node.map(str::to_owned),
                path.to_owned(),
                value.to_owned(),
            )),
        )
    };
    let opts = |list: &[(&str, &str)], current: &str| -> Vec<Item> {
        list.iter()
            .map(|(v, l)| pick(l, v, *v == current))
            .collect()
    };
    let statuses = [
        ("", "Any status"),
        ("todo", "To Do"),
        ("in_progress", "In Progress"),
        ("in_review", "In Review"),
        ("done", "Done"),
        ("cancelled", "Cancelled"),
    ];
    match path {
        "onError" => {
            let cur = n.and_then(|n| sv(n, "onError")).unwrap_or("stop");
            opts(
                &[("stop", "Stop the run"), ("continue", "Carry on anyway")],
                cur,
            )
        }
        "config.scriptType" => [
            ("bash", "Bash", "terminal"),
            ("powershell", "PowerShell", "terminal"),
            ("python", "Python", "file-code"),
            ("node", "Node", "braces"),
        ]
        .iter()
        .map(|(v, l, i)| {
            pick(l, v, get("scriptType") == *v).mark(ico(i, 12.0, 2.0, c(look::GRAY_400)))
        })
        .collect(),
        "script.project" | "agent.project" | "config.projectFilter" => {
            let contextual =
                s.st.wf
                    .editor
                    .as_ref()
                    .is_some_and(|d| wf::contextual(d.nodes()));
            let allow_none = path != "agent.project";
            let field = if path == "config.projectFilter" {
                "projectFilter"
            } else {
                "projectName"
            };
            let cur = get(field);
            let from_ctx = is_ctx(Some(&cur));
            let mut out = Vec::new();
            if contextual && path != "config.projectFilter" {
                out.push(pick("From Context", "@context", from_ctx).mark(ico(
                    "zap",
                    13.0,
                    1.5,
                    c(0x60a5fa),
                )));
            }
            if allow_none {
                out.push(pick("None", "", cur.is_empty() && !from_ctx).mark(ico(
                    "folder",
                    13.0,
                    1.5,
                    c(look::GRAY_600),
                )));
            }
            for p in &s.store.projects {
                out.push(
                    pick(&p.name, &p.name, !from_ctx && p.name == cur)
                        .mark(super::super::sidebar_project_icon(p, 13.0)),
                );
            }
            out
        }
        "config.model" => {
            let agent = get("agentType");
            let path = get("projectPath");
            let model = get("model");
            let mut out = vec![pick("Agent default", "", model.is_empty())];
            for m in s.st.wf.models.get(&(agent, path)).into_iter().flatten() {
                let mid = sv(m, "id").unwrap_or("");
                let mut it = pick(sv(m, "label").unwrap_or(mid), mid, mid == model);
                if sv(m, "label").is_some_and(|l| l != mid) {
                    it = it.detail(mid.to_owned());
                }
                out.push(it);
            }
            let b = bind(node, "config.model", Conv::Text, false);
            let fid = field_of(&b);
            s.acts
                .text(fid, Act::Workflow(WfAct::Text(b)), model.as_str());
            out.push(Item {
                row: Some(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .px(12.0)
                        .py(8.0)
                        .border_top(1.0, white(0.06))
                        .child(
                            text("ID", 10.0, c(look::GRAY_600))
                                .tracking(0.025)
                                .line_height(15.0),
                        )
                        .child(
                            div()
                                .grow()
                                .min_w0()
                                .px(8.0)
                                .py(2.0)
                                .rounded(4.0)
                                .bg(white(0.04))
                                .border(1.0, white(0.08))
                                .child(
                                    field(
                                        fid,
                                        "Model id",
                                        "model id",
                                        TextStyle {
                                            line_height: 16.5,
                                            ..TextStyle::mono(11.0)
                                        },
                                        c(look::GRAY_200),
                                    )
                                    .w_full(),
                                ),
                        ),
                ),
                ..Item::default()
            });
            out
        }
        "config.agentType" => {
            let mut list: Vec<(&str, &str)> = ["claude", "copilot", "codex", "opencode", "gemini"]
                .iter()
                .map(|a| (*a, agent_label(a)))
                .collect();
            let trig = s.st.wf.editor.as_ref().and_then(|d| {
                wf::trigger(d.nodes())
                    .and_then(|t| t.pointer("/config/triggerType"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            let task_trigger = matches!(trig.as_deref(), Some("taskCreated" | "taskStatusChanged"));
            let source_task = cfg.get("taskId").is_some_and(|t| !t.is_null())
                || cfg.get("taskFromQueue").and_then(Value::as_bool) == Some(true);
            if task_trigger || source_task {
                list.insert(0, ("fromTask", "From Task"));
            }
            list.iter()
                .map(|(v, l)| {
                    let glyph = if *v == "fromTask" {
                        ico("clipboard-list", 14.0, 2.0, c(look::BLUE_400))
                    } else {
                        super::super::agent_icon(s, v, 14.0)
                    };
                    pick(l, v, get("agentType") == *v).mark(glyph)
                })
                .collect()
        }
        "promptSource" => {
            let cur = if cfg.get("taskId").is_some_and(|t| !t.is_null()) {
                "task"
            } else if cfg.get("taskFromQueue").and_then(Value::as_bool) == Some(true) {
                "queue"
            } else {
                "inline"
            };
            opts(
                &[("inline", "Inline"), ("task", "Task"), ("queue", "Queue")],
                cur,
            )
        }
        "config.taskId" => {
            let mut out = vec![pick("Select task...", "", get("taskId").is_empty())];
            for t in s
                .store
                .tasks
                .iter()
                .filter(|t| t.project_name == get("projectName") && t.status.as_str() == "todo")
            {
                out.push(pick(&t.title, &t.id, get("taskId") == t.id));
            }
            out
        }
        "worktreeMode" => {
            let mode = worktree_mode(&cfg);
            let contextual =
                s.st.wf
                    .editor
                    .as_ref()
                    .is_some_and(|d| wf::contextual(d.nodes()));
            let has_branch = sv(&cfg, "branch").is_some_and(|b| !b.trim().is_empty());
            let steps = prior_worktree_steps(s, node.unwrap_or(""));
            let mut out = vec![
                pick("None", "none", mode == "none"),
                pick("New worktree", "new", mode == "new"),
                pick("From step", "fromStep", mode == "fromStep"),
                pick("Existing worktree", "existing", mode == "existing"),
            ];
            if !has_branch {
                out[1] = std::mem::take(&mut out[1]).detail("needs branch");
            }
            if steps.is_empty() {
                out[2] = std::mem::take(&mut out[2]).detail("no steps");
            }
            if contextual {
                out.push(pick("From Context", "fromContext", mode == "fromContext"));
            }
            out
        }
        "config.worktreeFromStepSlug" => {
            let mut out = vec![pick(
                "Select step...",
                "",
                get("worktreeFromStepSlug").is_empty(),
            )];
            for (slug, label) in prior_worktree_steps(s, node.unwrap_or("")) {
                out.push(pick(&label, &slug, get("worktreeFromStepSlug") == slug));
            }
            out
        }
        "config.existingWorktreePath" => vec![pick("Select worktree...", "", true)],
        "config.cron" => [
            ("0 9 * * 1-5", "Weekdays 9am"),
            ("0 9 * * *", "Daily 9am"),
            ("0 * * * *", "Every hour"),
            ("*/30 * * * *", "Every 30min"),
            ("0 9 * * 1", "Weekly (Mon 9am)"),
        ]
        .iter()
        .map(|(v, l)| pick(l, v, get("cron") == *v))
        .collect(),
        "config.fromStatus" | "config.toStatus" => {
            let cur = get(path.trim_start_matches("config."));
            opts(&statuses, &cur)
        }
        "config.method" => {
            let list: &[(&str, &str)] = if sv(&cfg, "triggerType") == Some("webhook") {
                &[("POST", "POST"), ("GET", "GET")]
            } else {
                &[
                    ("GET", "GET"),
                    ("POST", "POST"),
                    ("PUT", "PUT"),
                    ("PATCH", "PATCH"),
                    ("DELETE", "DELETE"),
                ]
            };
            opts(list, &get("method"))
        }
        "config.operator" => opts(
            &[
                ("equals", "equals"),
                ("notEquals", "not equals"),
                ("contains", "contains"),
                ("notContains", "not contains"),
                ("isEmpty", "is empty"),
                ("isNotEmpty", "is not empty"),
            ],
            &get("operator"),
        ),
        "config.until.operator" => opts(
            &[
                ("equals", "equals"),
                ("notEquals", "does not equal"),
                ("contains", "contains"),
                ("notContains", "does not contain"),
                ("isEmpty", "is empty"),
                ("isNotEmpty", "is not empty"),
            ],
            cfg.pointer("/until/operator")
                .and_then(Value::as_str)
                .unwrap_or("equals"),
        ),
        "feedback.from" => {
            let Some(d) = &s.st.wf.editor else {
                return Vec::new();
            };
            let cur = cfg
                .pointer("/feedback/from")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let ups = crate::vars::ancestors(d.nodes(), d.edges(), node.unwrap_or(""));
            let mut out = Vec::new();
            if !ups.iter().any(|u| sv(u, "id") == Some(cur.as_str())) {
                out.push(pick("Pick a step", &cur, true));
            }
            for u in ups {
                let id = sv(u, "id").unwrap_or("");
                out.push(pick(
                    sv(u, "label").filter(|l| !l.is_empty()).unwrap_or(id),
                    id,
                    id == cur,
                ));
            }
            out
        }
        "config.profileConnectionId" => {
            let mut out = vec![pick("None", "", get("profileConnectionId").is_empty())];
            for p in
                s.st.wf
                    .connections
                    .iter()
                    .filter(|c| sv(c, "connectorId") == Some("http"))
            {
                let id = sv(p, "id").unwrap_or("");
                out.push(pick(
                    sv(p, "name").unwrap_or(""),
                    id,
                    get("profileConnectionId") == id,
                ));
            }
            out
        }
        "poll.connection" | "action.connection" => {
            s.st.wf
                .connections
                .iter()
                .map(|cn| {
                    let id = sv(cn, "id").unwrap_or("");
                    let src = crate::library::Sources {
                        connections: &s.st.wf.connections,
                        packs: &s.st.wf.lib.packs,
                        ..crate::library::Sources::default()
                    };
                    let look = crate::library::look_for(&src, id);
                    pick(sv(cn, "name").unwrap_or(""), id, get("connectionId") == id)
                        .mark(mark(look.as_ref(), 14.0))
                })
                .collect()
        }
        "config.event" => {
            let connector =
                s.st.wf
                    .connections
                    .iter()
                    .find(|cn| sv(cn, "id") == Some(get("connectionId").as_str()))
                    .and_then(|cn| sv(cn, "connectorId"))
                    .unwrap_or("");
            s.st.wf
                .lib
                .connectors
                .iter()
                .find(|x| sv(x, "id") == Some(connector))
                .and_then(|x| x.pointer("/manifest/triggers"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|t| {
                    let v = sv(t, "type").unwrap_or("");
                    pick(sv(t, "label").unwrap_or(v), v, get("event") == v)
                })
                .collect()
        }
        "action.action" => {
            s.st.wf
                .lib
                .actions
                .get(&get("connectionId"))
                .into_iter()
                .flatten()
                .map(|a| {
                    let v = sv(a, "type").unwrap_or("");
                    let l = sv(a, "label").unwrap_or(v);
                    let mut it = pick(l, v, get("action") == v);
                    if l != v {
                        it = it.detail(v.to_owned());
                    }
                    it
                })
                .collect()
        }
        "config.project" => {
            let mut out = vec![pick(
                "From connection",
                "fromConnection",
                get("project") == "fromConnection",
            )];
            for p in &s.store.projects {
                out.push(pick(&p.name, &p.name, get("project") == p.name));
            }
            out
        }
        "config.initialStatus" => opts(&statuses[1..], &get("initialStatus")),
        p if p.starts_with("args.") => {
            let key = &p["args.".len()..];
            arg_field(s, node.unwrap_or(""), key)
                .and_then(|f| f.get("options").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .map(|o| {
                    let v = sv(o, "value").unwrap_or("");
                    pick(
                        sv(o, "label").unwrap_or(v),
                        v,
                        cfg.pointer(&format!("/args/{key}")).and_then(Value::as_str) == Some(v),
                    )
                })
                .collect()
        }
        p if p.starts_with("input.") && p.ends_with(".type") => {
            let i: usize = p["input.".len()..]
                .split('.')
                .next()
                .and_then(|i| i.parse().ok())
                .unwrap_or(0);
            let cur = cfg
                .pointer(&format!("/inputs/{i}/type"))
                .and_then(Value::as_str)
                .unwrap_or("text");
            opts(
                &[
                    ("text", "Text"),
                    ("textarea", "Long text"),
                    ("number", "Number"),
                    ("select", "Select"),
                    ("boolean", "Toggle"),
                    ("project", "Project"),
                    ("branch", "Branch"),
                ],
                cur,
            )
        }
        _ => Vec::new(),
    }
}

/// `getWorktreeMode`.
fn worktree_mode(cfg: &Value) -> String {
    if sv(cfg, "useWorktree") == Some("fromContext") {
        return "fromContext".into();
    }
    if let Some(m) = sv(cfg, "worktreeMode") {
        return m.to_owned();
    }
    if cfg.get("useWorktree").and_then(Value::as_bool) == Some(true) {
        "new".into()
    } else {
        "none".into()
    }
}

/// The agent steps whose worktree this one could reuse.
fn prior_worktree_steps(s: &Screen<'_>, current: &str) -> Vec<(String, String)> {
    let Some(d) = &s.st.wf.editor else {
        return Vec::new();
    };
    d.nodes()
        .iter()
        .filter(|n| sv(n, "id") != Some(current) && sv(n, "type") == Some("launchAgent"))
        .filter(|n| {
            let m = worktree_mode(&n.get("config").cloned().unwrap_or_default());
            m == "new" || m == "fromStep"
        })
        .map(|n| {
            let id = sv(n, "id").unwrap_or("");
            let slug = sv(n, "slug").unwrap_or(id);
            let label = sv(n, "label").filter(|l| !l.is_empty()).unwrap_or(slug);
            (slug.to_owned(), label.to_owned())
        })
        .collect()
}

/// The argument field a connector step's action declares for `key`.
fn arg_field(s: &Screen<'_>, node: &str, key: &str) -> Option<Value> {
    arg_fields(s, node)
        .into_iter()
        .find(|f| sv(f, "key") == Some(key))
}

/// `argFields`: the action's config fields, or what the catalog published
/// for a step not bound to a connection yet.
fn arg_fields(s: &Screen<'_>, node: &str) -> Vec<Value> {
    let Some(n) = s.st.wf.editor.as_ref().and_then(|d| d.node(node)) else {
        return Vec::new();
    };
    let cfg = n.get("config").cloned().unwrap_or_default();
    let conn = sv(&cfg, "connectionId").unwrap_or("");
    let action = sv(&cfg, "action").unwrap_or("");
    if let Some(fields) =
        s.st.wf
            .lib
            .actions
            .get(conn)
            .and_then(|l| l.iter().find(|a| sv(a, "type") == Some(action)))
            .and_then(|a| a.get("configFields"))
            .and_then(Value::as_array)
    {
        return fields.clone();
    }
    if !conn.is_empty() {
        return Vec::new();
    }
    let connector = sv(&cfg, "connectorId").unwrap_or("");
    s.st.wf
        .lib
        .catalog
        .iter()
        .find(|e| sv(e, "id") == Some(connector))
        .and_then(|e| e.get("actions"))
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|a| sv(a, "type") == Some(action)))
        .and_then(|a| a.get("inputs"))
        .and_then(Value::as_array)
        .map(|inputs| {
            inputs
                .iter()
                .map(|i| {
                    let kind = match sv(i, "type") {
                        Some("select") => "select",
                        Some("textarea") => "textarea",
                        _ => "text",
                    };
                    serde_json::json!({
                        "key": sv(i, "key").or_else(|| sv(i, "name")).unwrap_or(""),
                        "label": sv(i, "label").or_else(|| sv(i, "key")).unwrap_or(""),
                        "type": kind,
                        "required": i.get("required").cloned().unwrap_or(Value::Bool(false)),
                        "placeholder": i.get("placeholder").cloned().unwrap_or(Value::Null),
                        "description": i.get("description").cloned().unwrap_or(Value::Null),
                        "options": i.get("options").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `ToggleSwitch`: 40×24, blue when on, the knob white.
fn toggle(s: &Screen<'_>, a: WfAct, on: bool, disabled: bool, label: &str) -> El {
    let id = s.acts.on("wf-toggle", act(a));
    let mut el = div()
        .id(id)
        .w(40.0)
        .h(24.0)
        .rounded(12.0)
        .bg(if on { c(0x2b7fff) } else { white(0.10) })
        .role(Role::Switch, label)
        .toggled(on)
        .disabled(disabled)
        .child(
            div()
                .absolute(if on { 20.0 } else { 4.0 }, 4.0)
                .size(16.0, 16.0)
                .rounded(8.0)
                .bg(c(look::WHITE)),
        );
    if !disabled {
        el = el.on(Action::Click);
    }
    el
}

/// The switch cards (Contextual, Headless): a gray switch, a glyph, a
/// title and what it means.
fn switch_card(
    s: &Screen<'_>,
    a: WfAct,
    on: bool,
    icon: &str,
    title: &str,
    desc: &str,
    disabled: bool,
) -> El {
    let id = s.acts.on("wf-switch", act(a));
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let border = if disabled {
        white(0.04)
    } else if on {
        white(0.10)
    } else {
        white(0.04).mix(white(0.10), hot)
    };
    let bg = if disabled {
        white(0.01)
    } else if on {
        white(0.04)
    } else {
        white(0.02)
    };
    let mut el = div()
        .id(id)
        .w_full()
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(10.0)
        .rounded(8.0)
        .border(1.0, border)
        .bg(bg)
        .opacity(if disabled { 0.5 } else { 1.0 })
        .role(Role::Switch, title)
        .toggled(on)
        .child(
            div()
                .w(28.0)
                .h(16.0)
                .rounded(8.0)
                .shrink0()
                .bg(if on { c(look::GRAY_400) } else { white(0.10) })
                .child(
                    div()
                        .absolute(if on { 14.0 } else { 2.0 }, 2.0)
                        .size(12.0, 12.0)
                        .rounded(6.0)
                        .bg(c(look::WHITE)),
                ),
        )
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
                            icon,
                            12.0,
                            2.0,
                            if on {
                                c(look::GRAY_300)
                            } else {
                                c(look::GRAY_500)
                            },
                        ))
                        .child(
                            text(
                                title,
                                12.0,
                                if on {
                                    c(look::GRAY_200)
                                } else {
                                    c(look::GRAY_400)
                                },
                            )
                            .line_height(18.0)
                            .silent(),
                        ),
                )
                .child(
                    div().pt(2.0).child(
                        text(desc, 11.0, c(look::GRAY_500))
                            .line_height(16.5)
                            .wrapping()
                            .min_w0(),
                    ),
                ),
        );
    if !disabled {
        el = el.on(Action::Click);
    }
    el
}

/// `Segmented`: one row of choices.
fn segmented(s: &Screen<'_>, node: &str, path: &str, cur: &str, opts: &[(&str, &str)]) -> El {
    let segments = opts.iter().map(|&(v, l)| {
        let id = s.acts.on(
            "wf-seg",
            act(WfAct::Choose(
                Some(node.to_owned()),
                path.to_owned(),
                v.to_owned(),
            )),
        );
        (id, l, v == cur)
    });
    crate::ui::widgets::segmented(s.ui, segments)
}

/// The Advanced fold: chevron, gear, uppercase label.
fn advanced(s: &Screen<'_>, node: &str, open: bool) -> El {
    let id = s
        .acts
        .on("wf-advanced", act(WfAct::Advanced(node.to_owned())));
    let hot = s.ui.hover(id);
    let fg = c(look::GRAY_500).mix(c(look::GRAY_400), hot);
    div()
        .id(id)
        .on(Action::Click)
        .w_full()
        .row()
        .items_center()
        .gap(6.0)
        .role(Role::Button, "Advanced")
        .expanded(open)
        .child(ico(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            12.0,
            2.0,
            fg,
        ))
        .child(ico("settings-2", 11.0, 2.0, fg))
        .child(
            text("ADVANCED", 11.0, fg)
                .weight(500)
                .tracking(0.05)
                .line_height(16.5)
                .silent(),
        )
}

fn section(children: Vec<El>) -> El {
    div().col().w_full().gap(20.0).children(children)
}

fn block(children: Vec<El>) -> El {
    div().col().w_full().children(children)
}

/// `AGENT_LABELS` of the form's agent picker.
fn agent_label(agent: &str) -> &str {
    match agent {
        "fromTask" => "From Task",
        "claude" => "Claude",
        "copilot" => "Copilot",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "gemini" => "Gemini",
        "" => "Unassigned",
        other => other,
    }
}

/// The panel header's glyph: the connector a step calls, else
/// `NODE_TYPE_ICON`.
fn header_glyph(s: &Screen<'_>, n: &Value) -> El {
    let conn = n
        .pointer("/config/connectionId")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    if sv(n, "type") == Some("callConnectorAction") {
        if let Some(look) = conn.and_then(|cid| {
            let src = crate::library::Sources {
                connections: &s.st.wf.connections,
                packs: &s.st.wf.lib.packs,
                ..crate::library::Sources::default()
            };
            crate::library::look_for(&src, cid)
        }) {
            return mark(Some(&look), 14.0);
        }
    }
    let glyph = match sv(n, "type").unwrap_or("") {
        "trigger" | "callConnectorAction" => "zap",
        "launchAgent" => "play",
        k => super::super::runs::node_icon(k),
    };
    ico(glyph, 14.0, 2.0, secondary())
}

/// `ModelPicker` (`variant="form"`): the chip, the model or "Agent
/// default", its menu of listed models and an id field.
fn model_picker(s: &Screen<'_>, id: &str, n: &Value) -> El {
    let model = cfg_str(n, "model");
    let kind = MenuKind::WfChoice(token(380.0, Some(id), "config.model"));
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let bid = s.acts.on("wf-select", Act::Open(kind));
    let hot = s.ui.hover(bid);
    let shown = (!model.is_empty()).then(|| {
        let agent = cfg_str(n, "agentType").to_owned();
        let path = cfg_str(n, "projectPath").to_owned();
        s.st.wf
            .models
            .get(&(agent, path))
            .and_then(|l| l.iter().find(|m| sv(m, "id") == Some(model)))
            .and_then(|m| sv(m, "label"))
            .unwrap_or(model)
            .to_owned()
    });
    div()
        .id(bid)
        .on(Action::Click)
        .w_full()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.06))
        .border(1.0, white(0.10).mix(white(0.2), hot))
        .role(Role::Button, shown.as_deref().unwrap_or("Agent default"))
        .has_popup()
        .expanded(open)
        .child(ico(
            "cpu",
            14.0,
            2.0,
            if shown.is_some() {
                c(look::GRAY_300)
            } else {
                c(look::GRAY_500)
            },
        ))
        .child(
            div().max_w(170.0).child(
                text(
                    shown.as_deref().unwrap_or("Agent default"),
                    13.0,
                    if shown.is_some() {
                        c(look::WHITE)
                    } else {
                        c(look::GRAY_500)
                    },
                )
                .line_height(19.5)
                .truncate()
                .silent(),
            ),
        )
        .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_500)))
}

/// The panel for `n` (`NodeConfigPanel`).
pub fn node_panel(s: &Screen<'_>, d: &Draft, n: &Value) -> El {
    let id = sv(n, "id").unwrap_or("").to_owned();
    let kind = sv(n, "type").unwrap_or("");
    let label_bind = bind(Some(&id), "label", Conv::Keep, false);
    let lid = field_of(&label_bind);
    s.acts.text(
        lid,
        Act::Workflow(WfAct::Text(label_bind)),
        sv(n, "label").unwrap_or(""),
    );
    let lhot = if s.ui.focused(lid) {
        0.06
    } else {
        0.04 * s.ui.hover(lid)
    };
    let more_kind = MenuKind::NodeMore(id.clone());
    let more_id = s.acts.on("wf-node-more", Act::Open(more_kind));
    let close_id = s.acts.on("wf-panel-close", act(WfAct::ClosePanel));
    let small = |bid: crate::ui::Id, icon: &str, label: &str| {
        let hot = s.ui.hover(bid);
        div()
            .id(bid)
            .on(Action::Click)
            .p(4.0)
            .rounded(6.0)
            .role(Role::Button, label)
            .child(ico(
                icon,
                14.0,
                2.0,
                c(look::GRAY_500).mix(c(look::WHITE), hot),
            ))
    };
    let head = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(20.0)
        .py(12.0)
        .border_bottom(1.0, white(0.08))
        .child(header_glyph(s, n))
        .child(
            div()
                .grow()
                .min_w0()
                .ml(-6.0)
                .px(6.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(lhot))
                .child(
                    field(
                        lid,
                        "Label",
                        "Label",
                        TextStyle {
                            weight: 500,
                            line_height: 19.5,
                            ..TextStyle::ui(13.0)
                        },
                        c(look::WHITE),
                    )
                    .w_full(),
                ),
        )
        .child(
            div()
                .row()
                .gap(2.0)
                .child(small(more_id, "ellipsis", "More node actions"))
                .child(small(close_id, "x", "Close node config")),
        );
    let mut body = div().col().w_full().p(20.0).gap(20.0);
    body = body.child(match kind {
        "trigger" => trigger_form(s, d, &id, n),
        "launchAgent" => agent_form(s, d, &id, n),
        "script" => script_form(s, d, &id, n),
        "condition" => condition_form(s, d, &id, n),
        "loop" => loop_form(s, d, &id, n),
        "approval" => approval_form(s, d, &id, n),
        "createTaskFromItem" => create_task_form(s, &id, n),
        "httpRequest" => http_form(s, d, &id, n),
        "callConnectorAction" => action_form(s, d, &id, n),
        _ => div(),
    });
    if kind != "trigger" {
        let stop = sv(n, "onError").unwrap_or("stop") == "stop";
        let cur = if stop {
            "Stop the run"
        } else {
            "Carry on anyway"
        };
        body = body.child(
            div()
                .col()
                .w_full()
                .pt(16.0)
                .border_top(1.0, white(0.08))
                .child(div().pb(6.0).child(text("If this step fails", 11.0, c(look::GRAY_500)).line_height(16.5)))
                .child(select(s, token(380.0, Some(&id), "onError"), Some(cur), None, "", Wash::Quiet))
                .child(div().pt(6.0).child(
                    text(
                        if stop {
                            "Everything downstream is skipped and the run ends as failed."
                        } else {
                            "Later steps run as if this one had succeeded, and the run still counts as successful \u{2014} for a step whose failure does not invalidate the rest."
                        },
                        10.0,
                        c(look::GRAY_600),
                    )
                    .line_height(15.0)
                    .wrapping()
                    .min_w0(),
                )),
        );
    }
    let mut panel = div()
        .col()
        .w(420.0)
        .shrink0()
        .h_full()
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border_left(1.0, white(0.08))
        .role(Role::Group, "Node configuration")
        .child(head)
        .child(
            div()
                .id(crate::ui::Id::new("wf-node-body"))
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .child(body),
        );
    let body_owned = crate::graph::loop_body_owners(d.nodes()).contains_key(&id);
    if kind != "trigger" && !body_owned {
        let rid = s.acts.on("wf-run-to", act(WfAct::RunTo(id.clone())));
        let hot = s.ui.hover(rid);
        panel = panel.child(
            div().shrink0().p(12.0).border_top(1.0, white(0.08)).child(
                div()
                    .id(rid)
                    .on(Action::Click)
                    .w_full()
                    .row()
                    .center()
                    .gap(8.0)
                    .px(12.0)
                    .py(6.0)
                    .rounded(6.0)
                    .border(1.0, white(0.12).mix(white(0.2), hot))
                    .role(Role::Button, "Run to this step")
                    .child(ico(
                        "step-forward",
                        12.0,
                        2.0,
                        c(look::GRAY_300).mix(c(look::WHITE), hot),
                    ))
                    .child(
                        text(
                            "Run to this step",
                            12.0,
                            c(look::GRAY_300).mix(c(look::WHITE), hot),
                        )
                        .line_height(18.0)
                        .silent(),
                    ),
            ),
        );
    }
    panel
}

fn cfg_str<'a>(n: &'a Value, k: &str) -> &'a str {
    n.pointer(&format!("/config/{k}"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// `TRIGGER_TYPES`: label, glyph and hint.
fn trigger_kind(t: &str) -> (&'static str, &'static str, &'static str) {
    match t {
        "manual" => ("Manual", "zap", "Run manually from the play button"),
        "once" => ("Once", "clock", "Runs once at the scheduled time"),
        "recurring" => ("Recurring", "refresh-cw", "Runs on a repeating schedule"),
        "taskCreated" => (
            "Task Created",
            "list-plus",
            "Fires when a new task is added",
        ),
        "taskStatusChanged" => (
            "Status Change",
            "arrow-right-left",
            "Fires when a task's status changes",
        ),
        "sessionRestored" => (
            "Session Restored",
            "rotate-ccw",
            "Fires when a session comes back after a quit, a crash or a reboot",
        ),
        "connectorPoll" => (
            "Connector Poll",
            "plug",
            "Polls an external connector on cron and fires per new item",
        ),
        "webhook" => (
            "Webhook",
            "globe",
            "Fires when this machine receives an HTTP request at the workflow URL",
        ),
        _ => ("Trigger", "zap", ""),
    }
}

fn trigger_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let t = cfg_str(n, "triggerType");
    let (name, glyph, hint) = trigger_kind(t);
    let link_id = s.acts.on(
        "wf-trigger-lib",
        act(WfAct::Library(wf::Anchor {
            trigger: true,
            ..wf::Anchor::default()
        })),
    );
    let link_hot = s.ui.hover(link_id);
    let mut out = vec![block(vec![
        label("Trigger Type"),
        div()
            .row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .rounded(8.0)
            .border(1.0, white(0.06))
            .bg(white(0.02))
            .child(ico(glyph, 13.0, 2.0, c(look::GRAY_400)))
            .child(text(name, 13.0, c(look::GRAY_200)).line_height(19.5)),
        help(hint, 6.0),
        div().pt(8.0).child(
            div()
                .id(link_id)
                .on(Action::Click)
                .border_bottom(1.0, c(look::GRAY_500).mix(c(look::GRAY_300), link_hot))
                .role(Role::Button, "Change trigger from the library")
                .child(
                    text(
                        "Change trigger from the library",
                        11.0,
                        c(look::GRAY_500).mix(c(look::GRAY_300), link_hot),
                    )
                    .line_height(14.0),
                ),
        ),
    ])];
    let project_filter = |help_text: Option<&str>| {
        let cur = cfg_str(n, "projectFilter");
        let mut b = vec![
            label("Project Filter"),
            project(s, id, "config.projectFilter", cur, false),
        ];
        if let Some(h) = help_text {
            b.push(help(h, 4.0));
        }
        block(b)
    };
    match t {
        "manual" => {
            let on = n.pointer("/config/contextual").and_then(Value::as_bool) == Some(true);
            out.push(switch_card(
                s,
                WfAct::Toggle("config.contextual".into()),
                on,
                "zap",
                "Contextual",
                "Run this workflow directly from any card or terminal, against that session's folder and branch.",
                false,
            ));
            out.push(inputs_editor(s, id, n));
        }
        "once" => out.push(block(vec![
            label("Run At"),
            input(
                s,
                bind(Some(id), "config.runAt", Conv::Text, false),
                cfg_str(n, "runAt"),
                "",
                false,
                Wash::Strong,
                13.0,
                12.0,
            ),
        ])),
        "recurring" => {
            let cron = cfg_str(n, "cron");
            let preset = [
                ("0 9 * * 1-5", "Weekdays 9am"),
                ("0 9 * * *", "Daily 9am"),
                ("0 * * * *", "Every hour"),
                ("*/30 * * * *", "Every 30min"),
                ("0 9 * * 1", "Weekly (Mon 9am)"),
            ]
            .iter()
            .find(|(v, _)| *v == cron)
            .map(|(_, l)| *l);
            out.push(block(vec![
                label("Preset"),
                select(
                    s,
                    token(380.0, Some(id), "config.cron"),
                    preset,
                    None,
                    "Choose preset...",
                    Wash::Strong,
                ),
            ]));
            out.push(block(vec![
                label("Cron Expression"),
                input(
                    s,
                    bind(Some(id), "config.cron", Conv::Keep, false),
                    cron,
                    "* * * * *",
                    true,
                    Wash::Strong,
                    13.0,
                    12.0,
                ),
                help("min hour day month weekday", 4.0),
            ]));
            let tz = n
                .pointer("/config/timezone")
                .and_then(Value::as_str)
                .filter(|z| !z.is_empty())
                .unwrap_or(crate::tasks::local_zone());
            out.push(block(vec![
                label("Timezone"),
                input(
                    s,
                    bind(Some(id), "config.timezone", Conv::Keep, false),
                    tz,
                    "",
                    false,
                    Wash::Strong,
                    13.0,
                    12.0,
                ),
            ]));
        }
        "taskCreated" => out.push(project_filter(Some(
            "Only trigger for tasks in this project",
        ))),
        "connectorPoll" => {
            let conn = cfg_str(n, "connectionId");
            let current =
                s.st.wf
                    .connections
                    .iter()
                    .find(|c| sv(c, "id") == Some(conn));
            let src = crate::library::Sources {
                connections: &s.st.wf.connections,
                packs: &s.st.wf.lib.packs,
                ..crate::library::Sources::default()
            };
            let mut b = vec![
                label("Connection"),
                select(
                    s,
                    token(380.0, Some(id), "poll.connection"),
                    current.and_then(|c| sv(c, "name")),
                    current.map(|_| mark(crate::library::look_for(&src, conn).as_ref(), 14.0)),
                    "Select a connection...",
                    Wash::Strong,
                ),
            ];
            if s.st.wf.connections.is_empty() {
                b.push(help(
                    "No connections yet. Add one from Settings \u{203a} Connectors.",
                    6.0,
                ));
            }
            out.push(block(b));
            if let Some(c) = current {
                let connector = sv(c, "connectorId").unwrap_or("");
                let ev = cfg_str(n, "event");
                let ev_label =
                    s.st.wf
                        .lib
                        .connectors
                        .iter()
                        .find(|x| sv(x, "id") == Some(connector))
                        .and_then(|x| x.pointer("/manifest/triggers"))
                        .and_then(Value::as_array)
                        .and_then(|l| l.iter().find(|t| sv(t, "type") == Some(ev)))
                        .and_then(|t| sv(t, "label"));
                out.push(block(vec![
                    label("Event"),
                    select(
                        s,
                        token(380.0, Some(id), "config.event"),
                        ev_label,
                        None,
                        "Select an event...",
                        Wash::Strong,
                    ),
                ]));
            }
            out.push(block(vec![
                label("Cron"),
                input(
                    s,
                    bind(Some(id), "config.cron", Conv::Keep, false),
                    cfg_str(n, "cron"),
                    "*/5 * * * *",
                    true,
                    Wash::Strong,
                    13.0,
                    12.0,
                ),
                help(
                    "min hour day month weekday \u{b7} default every 5 minutes",
                    4.0,
                ),
            ]));
        }
        "sessionRestored" => {
            out.push(project_filter(Some(
                "Without one, this fires in every project on every start-up",
            )));
            let restore = n
                .pointer("/config/restore")
                .and_then(Value::as_str)
                .unwrap_or("cold");
            out.push(block(vec![
                label("Which restores"),
                segmented(
                    s,
                    id,
                    "config.restore",
                    restore,
                    &[("cold", "Cold only"), ("any", "Warm too")],
                ),
                help(
                    "Cold: a process started again. Warm: a pane attached to one still running.",
                    4.0,
                ),
            ]));
            let conc = n
                .pointer("/config/concurrency")
                .and_then(Value::as_str)
                .unwrap_or("perProject");
            out.push(block(vec![
                label("At a time"),
                segmented(
                    s,
                    id,
                    "config.concurrency",
                    conc,
                    &[
                        ("perProject", "One per project"),
                        ("unbounded", "Any number"),
                    ],
                ),
                help(
                    "Eight sessions in one project make one run, then the next",
                    4.0,
                ),
            ]));
        }
        "webhook" => {
            let url =
                s.st.wf
                    .webhook
                    .as_ref()
                    .filter(|_| d.existing)
                    .map(|base| format!("{base}/wf-hooks/{}/{}", d.id(), cfg_str(n, "token")));
            let mut b = vec![label("URL")];
            match &url {
                Some(u) => {
                    let copied =
                        s.st.wf
                            .copied
                            .is_some_and(|t| crate::state::now_ms() - t < 1500.0);
                    let cid = s.acts.on("wf-copy", act(WfAct::Copy(u.clone())));
                    let hot = s.ui.hover(cid);
                    b.push(
                        div()
                            .row()
                            .items_center()
                            .gap(8.0)
                            .child(
                                div()
                                    .grow()
                                    .min_w0()
                                    .px(12.0)
                                    .py(8.0)
                                    .rounded(6.0)
                                    .bg(white(0.06))
                                    .border(1.0, white(0.10))
                                    .child(
                                        text(u, 11.0, c(look::GRAY_300))
                                            .mono()
                                            .line_height(16.5)
                                            .truncate(),
                                    ),
                            )
                            .child(
                                div()
                                    .id(cid)
                                    .on(Action::Click)
                                    .p(8.0)
                                    .rounded(6.0)
                                    .border(1.0, white(0.10).mix(white(0.2), hot))
                                    .role(Role::Button, "Copy URL")
                                    .child(ico(
                                        if copied { "check" } else { "copy" },
                                        12.0,
                                        2.0,
                                        c(look::GRAY_400).mix(c(look::WHITE), hot),
                                    )),
                            ),
                    );
                }
                None => b.push(
                    text(
                        "Save the workflow first \u{2014} the URL includes its id.",
                        11.0,
                        c(look::GRAY_500),
                    )
                    .line_height(16.5)
                    .wrapping()
                    .min_w0(),
                ),
            }
            b.push(help("Local machine only, and only while the app is open. Exposing it is a deliberate, separate step.", 6.0));
            out.push(block(b));
            out.push(block(vec![
                label("Method"),
                select(s, token(380.0, Some(id), "config.method"), Some(cfg_str(n, "method")), None, "", Wash::Strong),
                help("The request lands as {{trigger.body.*}}, {{trigger.headers.*}}, and {{trigger.query.*}}.", 6.0),
            ]));
        }
        "taskStatusChanged" => {
            out.push(project_filter(None));
            let st = |k: &str| match cfg_str(n, k) {
                "todo" => Some("To Do"),
                "in_progress" => Some("In Progress"),
                "in_review" => Some("In Review"),
                "done" => Some("Done"),
                "cancelled" => Some("Cancelled"),
                _ => Some("Any status"),
            };
            out.push(block(vec![
                label("From Status"),
                select(
                    s,
                    token(380.0, Some(id), "config.fromStatus"),
                    st("fromStatus"),
                    None,
                    "Any status",
                    Wash::Strong,
                ),
            ]));
            out.push(block(vec![
                label("To Status"),
                select(
                    s,
                    token(380.0, Some(id), "config.toStatus"),
                    st("toStatus"),
                    None,
                    "Any status",
                    Wash::Strong,
                ),
            ]));
        }
        _ => {}
    }
    section(out)
}

/// `WorkflowInputsEditor`.
fn inputs_editor(s: &Screen<'_>, id: &str, n: &Value) -> El {
    let defs = n
        .pointer("/config/inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let dups = wf::duplicate_keys(&defs);
    let small =
        |path: String, value: &str, placeholder: &str, mono: bool, conv: Conv, bad: bool| {
            let b = bind(Some(id), &path, conv, false);
            let fid = field_of(&b);
            s.acts.text(fid, Act::Workflow(WfAct::Text(b)), value);
            let focus = s.ui.focused(fid);
            div()
                .w_full()
                .px(10.0)
                .py(6.0)
                .rounded(6.0)
                .bg(white(0.06))
                .border(
                    1.0,
                    if bad {
                        Rgba::hexa(look::DANGER, 0.6)
                    } else if focus {
                        white(0.2)
                    } else {
                        white(0.10)
                    },
                )
                .child(
                    field(
                        fid,
                        placeholder,
                        placeholder,
                        TextStyle {
                            line_height: 18.0,
                            ..if mono {
                                TextStyle::mono(12.0)
                            } else {
                                TextStyle::ui(12.0)
                            }
                        },
                        c(look::WHITE),
                    )
                    .w_full()
                    .min_w0(),
                )
        };
    let mut list = div().col().w_full().gap(8.0);
    for (i, def) in defs.iter().enumerate() {
        let key = sv(def, "key").unwrap_or("");
        let kind = sv(def, "type").unwrap_or("text");
        let dup = !key.is_empty() && dups.iter().any(|d| d == key);
        let rm = s.acts.on("wf-input-remove", act(WfAct::RemoveInput(i)));
        let rhot = s.ui.hover(rm);
        let required = def.get("required").and_then(Value::as_bool) == Some(true);
        let req_id = s.acts.on(
            "wf-input-req",
            act(WfAct::Choose(
                Some(id.to_owned()),
                format!("input.{i}.required"),
                String::new(),
            )),
        );
        let mut box_ = div()
            .col()
            .w_full()
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
                    .child(div().grow().min_w0().child(small(
                        format!("input.{i}.key"),
                        key,
                        "key",
                        true,
                        Conv::Key,
                        dup,
                    )))
                    .child(
                        div()
                            .id(rm)
                            .on(Action::Click)
                            .p(6.0)
                            .rounded(6.0)
                            .bg(white(0.0).mix(white(0.06), rhot))
                            .role(Role::Button, format!("Remove input {key}"))
                            .child(ico(
                                "trash-2",
                                12.0,
                                2.0,
                                c(look::GRAY_500).mix(c(look::DANGER), rhot),
                            )),
                    ),
            );
        if dup {
            box_ = box_.child(
                div()
                    .row()
                    .wrap_children()
                    .child(
                        text(
                            "Duplicate key \u{2014} only one input named ",
                            11.0,
                            c(look::DANGER),
                        )
                        .line_height(16.5),
                    )
                    .child(text(key, 11.0, c(look::DANGER)).mono().line_height(16.5))
                    .child(text(" will reach the run.", 11.0, c(look::DANGER)).line_height(16.5)),
            );
        }
        box_ = box_.child(small(
            format!("input.{i}.label"),
            sv(def, "label").unwrap_or(""),
            "Label shown in the run dialog",
            false,
            Conv::Keep,
            false,
        ));
        let type_label = [
            ("text", "Text"),
            ("textarea", "Long text"),
            ("number", "Number"),
            ("select", "Select"),
            ("boolean", "Toggle"),
            ("project", "Project"),
            ("branch", "Branch"),
        ]
        .iter()
        .find(|(v, _)| *v == kind)
        .map(|(_, l)| *l);
        let mut cbx = div()
            .size(13.0, 13.0)
            .rounded(3.0)
            .center()
            .border(
                1.0,
                if required {
                    white(0.0)
                } else {
                    c(look::GRAY_600)
                },
            )
            .bg(if required { white(0.8) } else { white(0.05) });
        if required {
            cbx = cbx.child(ico("check", 9.0, 3.0, c(look::SURFACE_BASE)));
        }
        box_ = box_.child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(div().grow().min_w0().child(select(
                    s,
                    token(240.0, Some(id), &format!("input.{i}.type")),
                    type_label,
                    None,
                    "",
                    Wash::Strong,
                )))
                .child(
                    div()
                        .id(req_id)
                        .on(Action::Click)
                        .row()
                        .items_center()
                        .gap(6.0)
                        .shrink0()
                        .role(Role::CheckBox, "Required")
                        .toggled(required)
                        .child(cbx)
                        .child(
                            text("Required", 12.0, c(look::GRAY_400))
                                .line_height(18.0)
                                .silent(),
                        ),
                ),
        );
        if kind == "select" {
            let opts: Vec<&str> = def
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| sv(o, "value"))
                .collect();
            box_ = box_.child(small(
                format!("input.{i}.options"),
                &opts.join(", "),
                "Comma-separated choices",
                false,
                Conv::Options,
                false,
            ));
        }
        if kind != "boolean" && kind != "select" {
            let dv = wf::text_of(def.get("defaultValue"));
            box_ = box_.child(small(
                format!("input.{i}.defaultValue"),
                &dv,
                "Default value (optional)",
                false,
                Conv::Text,
                false,
            ));
        }
        list = list.child(box_);
    }
    let add = s.acts.on("wf-input-add", act(WfAct::AddInput));
    let ahot = s.ui.hover(add);
    block(vec![
        label("Run Inputs"),
        div().pb(8.0).child(
            div()
                .row()
                .wrap_children()
                .min_w0()
                .child(
                    text(
                        "Values the user is asked for before the run starts. Use them anywhere as ",
                        11.0,
                        c(look::GRAY_500),
                    )
                    .line_height(16.5),
                )
                .child(
                    text("{{inputs.key}}", 11.0, c(look::GRAY_400))
                        .mono()
                        .line_height(16.5),
                )
                .child(text(".", 11.0, c(look::GRAY_500)).line_height(16.5)),
        ),
        list,
        div().pt(8.0).row().child(
            div()
                .id(add)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(10.0)
                .py(6.0)
                .rounded(6.0)
                .bg(white(0.0).mix(white(0.06), ahot))
                .role(Role::Button, "Add input")
                .child(ico(
                    "plus",
                    12.0,
                    2.0,
                    c(look::GRAY_400).mix(c(look::WHITE), ahot),
                ))
                .child(
                    text(
                        "Add input",
                        12.0,
                        c(look::GRAY_400).mix(c(look::WHITE), ahot),
                    )
                    .line_height(18.0)
                    .silent(),
                ),
        ),
    ])
}

fn script_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let ty = cfg_str(n, "scriptType");
    let (tl, ti) = match ty {
        "powershell" => ("PowerShell", "terminal"),
        "python" => ("Python", "file-code"),
        "node" => ("Node", "braces"),
        _ => ("Bash", "terminal"),
    };
    let cwd_ctx = is_ctx(n.pointer("/config/cwd").and_then(Value::as_str))
        || is_ctx(Some(cfg_str(n, "projectName")))
        || is_ctx(Some(cfg_str(n, "projectPath")));
    let (groups, vars) = wf::var_sources(&s.st.wf, d, id, "config.scriptContent");
    let has_vars = !groups.is_empty() || !vars.is_empty();
    let open = s.st.wf.advanced.contains(id)
        || n.pointer("/config/args")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
            && !s.st.wf.advanced.contains(&format!("{id}:shut"));
    let mut script = vec![
        label("Script"),
        var_area(
            s,
            d,
            bind(Some(id), "config.scriptContent", Conv::Keep, true),
            cfg_str(n, "scriptContent"),
            &format!("Enter {ty} script..."),
            10,
            true,
        ),
    ];
    if has_vars {
        script.push(help(
            "Type {{ to insert step outputs or trigger variables",
            4.0,
        ));
    }
    let mut adv = vec![advanced(s, id, open)];
    if open {
        let args: Vec<&str> = n
            .pointer("/config/args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        adv.push(div().pt(12.0).child(block(vec![
            label("Arguments"),
            input(
                s,
                bind(Some(id), "config.args", Conv::Spaces, false),
                &args.join(" "),
                "arg1 arg2 --flag",
                false,
                Wash::Strong,
                13.0,
                12.0,
            ),
        ])));
    }
    section(vec![
        block(vec![
            label("Type"),
            select(
                s,
                token(380.0, Some(id), "config.scriptType"),
                Some(tl),
                Some(ico(ti, 12.0, 2.0, c(look::GRAY_400))),
                "",
                Wash::Strong,
            ),
        ]),
        block(vec![
            label("Working Directory"),
            project(s, id, "script.project", cfg_str(n, "projectName"), cwd_ctx),
        ]),
        block(script),
        div()
            .col()
            .w_full()
            .pt(16.0)
            .border_top(1.0, white(0.06))
            .children(adv),
    ])
}

fn agent_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let cfg = n.get("config").cloned().unwrap_or_default();
    let agent = cfg_str(n, "agentType");
    let agent_label = agent_label(agent).to_owned();
    let agent_glyph = if agent == "fromTask" {
        ico("clipboard-list", 14.0, 2.0, c(look::BLUE_400))
    } else {
        super::super::agent_icon(s, agent, 14.0)
    };
    let proj_ctx =
        is_ctx(Some(cfg_str(n, "projectName"))) || is_ctx(Some(cfg_str(n, "projectPath")));
    let branch_ctx = is_ctx(Some(cfg_str(n, "branch")));
    let source = if cfg.get("taskId").is_some_and(|t| !t.is_null()) {
        "task"
    } else if cfg.get("taskFromQueue").and_then(Value::as_bool) == Some(true) {
        "queue"
    } else {
        "inline"
    };
    let contextual = wf::contextual(d.nodes());
    let mut prompt = vec![
        label("Prompt"),
        div().pb(8.0).child(select(
            s,
            token(380.0, Some(id), "promptSource"),
            Some(match source {
                "task" => "Task",
                "queue" => "Queue",
                _ => "Inline",
            }),
            None,
            "",
            Wash::Strong,
        )),
    ];
    match source {
        "inline" => prompt.push(var_area(
            s,
            d,
            bind(Some(id), "config.prompt", Conv::Text, true),
            cfg_str(n, "prompt"),
            "Enter prompt...",
            4,
            false,
        )),
        "task" => {
            let tid = cfg_str(n, "taskId");
            let title = s
                .store
                .tasks
                .iter()
                .find(|t| t.id == tid)
                .map(|t| t.title.as_str());
            prompt.push(select(
                s,
                token(380.0, Some(id), "config.taskId"),
                title.or(Some("Select task...")),
                None,
                "Select task...",
                Wash::Strong,
            ));
        }
        _ => prompt.push(
            div()
                .row()
                .wrap_children()
                .child(
                    text(
                        "Auto-picks the next todo task from ",
                        11.0,
                        c(look::GRAY_500),
                    )
                    .line_height(17.9),
                )
                .child(
                    text(
                        sv(&cfg, "projectName")
                            .filter(|p| !p.is_empty())
                            .unwrap_or("the project"),
                        11.0,
                        c(look::GRAY_400),
                    )
                    .line_height(17.9),
                )
                .child(text(" at runtime.", 11.0, c(look::GRAY_500)).line_height(17.9)),
        ),
    }
    let mut out = vec![
        block(vec![
            label("Agent"),
            select(
                s,
                token(380.0, Some(id), "config.agentType"),
                Some(&agent_label),
                Some(agent_glyph),
                "",
                Wash::Strong,
            ),
        ]),
        block(vec![label("Model"), model_picker(s, id, n)]),
        block(vec![
            label("Project"),
            project(s, id, "agent.project", cfg_str(n, "projectName"), proj_ctx),
        ]),
        block(prompt),
    ];
    // Git and branch.
    let mode = worktree_mode(&cfg);
    let branch_box = {
        let mut row = div()
            .row()
            .items_center()
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .rounded(6.0)
            .bg(white(0.06))
            .border(1.0, white(0.10));
        if branch_ctx {
            row = row.child(ico("zap", 12.0, 2.0, ink(1.0))).child(
                div()
                    .grow()
                    .child(text("From Context", 13.0, c(look::GRAY_200)).line_height(19.5)),
            );
        } else {
            let b = bind(Some(id), "config.branch", Conv::Text, false);
            let fid = field_of(&b);
            s.acts
                .text(fid, Act::Workflow(WfAct::Text(b)), cfg_str(n, "branch"));
            row = row
                .child(ico("git-branch", 12.0, 2.0, c(look::GRAY_500)))
                .child(
                    field(
                        fid,
                        "Branch",
                        "feature/my-branch",
                        TextStyle {
                            line_height: 19.5,
                            ..TextStyle::ui(13.0)
                        },
                        c(look::WHITE),
                    )
                    .grow()
                    .min_w0(),
                );
        }
        if contextual {
            let zid = s.acts.on(
                "wf-branch-ctx",
                act(WfAct::Choose(
                    Some(id.to_owned()),
                    "branch.context".into(),
                    String::new(),
                )),
            );
            let zhot = s.ui.hover(zid);
            let zap = div()
                .id(zid)
                .on(Action::Click)
                .p(2.0)
                .rounded(4.0)
                .bg(white(0.0).mix(white(0.08), zhot))
                .role(
                    Role::Button,
                    if branch_ctx {
                        "Use a specific branch"
                    } else {
                        "Use From Context"
                    },
                )
                .child(ico(
                    "zap",
                    11.0,
                    2.0,
                    if branch_ctx {
                        ink(1.0)
                    } else {
                        c(look::GRAY_500).mix(c(look::GRAY_300), zhot)
                    },
                ));
            row = row.child(tooltip(
                s.ui,
                zap,
                zid,
                if branch_ctx {
                    "Use a specific branch"
                } else {
                    "Use From Context"
                },
                None,
            ));
        }
        row
    };
    let mode_help = match mode.as_str() {
        "none" => "Agent runs in the project directory",
        "new" => "Isolated directory \u{2014} won't affect the main working tree",
        "fromStep" => "Reuses the worktree created by a previous step",
        "existing" => "Launches into an existing worktree on disk",
        _ => "Inherits the launching card or terminal's worktree (won't be auto-cleaned)",
    };
    let mode_label = match mode.as_str() {
        "none" => "None",
        "new" => "New worktree",
        "fromStep" => "From step",
        "existing" => "Existing worktree",
        _ => "From Context",
    };
    let mut git = vec![
        div()
            .row()
            .items_center()
            .gap(6.0)
            .child(ico("git-branch", 11.0, 2.0, c(look::GRAY_400)))
            .child(
                text("Git & Branch", 13.0, c(look::GRAY_400))
                    .weight(500)
                    .line_height(19.5),
            ),
        block(vec![
            branch_box,
            help("Checks out this branch before launching", 4.0),
        ]),
        block(vec![
            div()
                .pb(6.0)
                .child(text("Worktree", 11.0, c(look::GRAY_500)).line_height(16.5)),
            select(
                s,
                token(354.0, Some(id), "worktreeMode"),
                Some(mode_label),
                None,
                "",
                Wash::Strong,
            ),
            help(mode_help, 6.0),
        ]),
    ];
    if mode == "fromStep" {
        let slug = cfg_str(n, "worktreeFromStepSlug");
        let l = prior_worktree_steps(s, id)
            .into_iter()
            .find(|(sl, _)| sl == slug)
            .map(|(_, l)| l);
        git.push(select(
            s,
            token(354.0, Some(id), "config.worktreeFromStepSlug"),
            Some(l.as_deref().unwrap_or("Select step...")),
            None,
            "Select step...",
            Wash::Strong,
        ));
    }
    if mode == "existing" {
        git.push(select(
            s,
            token(354.0, Some(id), "config.existingWorktreePath"),
            Some("Select worktree..."),
            None,
            "Select worktree...",
            Wash::Strong,
        ));
    }
    // A project that is not a git repository has no branch to check out.
    let path = sv(&cfg, "projectPath").unwrap_or("");
    if s.st.wf.git_repo.get(path) != Some(&false) {
        out.push(
            div()
                .col()
                .w_full()
                .gap(12.0)
                .p(12.0)
                .rounded(8.0)
                .border(1.0, white(0.06))
                .children(git),
        );
    }
    // Execution.
    let headless = cfg.get("headless").and_then(Value::as_bool) == Some(true);
    let mut exec = vec![
        text("Execution", 13.0, c(look::GRAY_400))
            .weight(500)
            .line_height(19.5),
        switch_card(
            s,
            WfAct::Toggle("config.headless".into()),
            headless,
            "eye-off",
            "Headless",
            if headless {
                "Runs in background. Waits for completion before next step."
            } else {
                "Opens a terminal tab. Step completes immediately."
            },
            false,
        ),
    ];
    if !headless {
        let ph = sv(&cfg, "projectName")
            .filter(|p| !p.is_empty())
            .unwrap_or("Uses project name")
            .to_owned();
        exec.push(block(vec![
            label("Tab Name"),
            input(
                s,
                bind(Some(id), "config.displayName", Conv::Text, false),
                cfg_str(n, "displayName"),
                &ph,
                false,
                Wash::Strong,
                13.0,
                12.0,
            ),
        ]));
    } else {
        let key = (id.to_owned(), "config.outputSchema".to_owned());
        let raw = s.st.wf.raw.get(&key).cloned();
        let shown = raw.clone().unwrap_or_else(|| {
            cfg.get("outputSchema")
                .map(crate::gates::pretty)
                .unwrap_or_default()
        });
        let err = raw
            .as_deref()
            .and_then(|r| wf::convert(Conv::Schema, r).err());
        let b = bind(Some(id), "config.outputSchema", Conv::Schema, false);
        let fid = field_of(&b);
        s.acts.text(fid, Act::Workflow(WfAct::Text(b)), shown);
        let mut schema = vec![
            label("Output Schema"),
            div().w_full().px(12.0).py(8.0).rounded(6.0).bg(white(0.06))
                .border(1.0, if err.is_some() { Rgba::hexa(look::DANGER, 0.5) } else { white(0.10) })
                .child(textarea(fid, "Output Schema", "{\n  \"type\": \"object\",\n  \"properties\": { \"verdict\": { \"type\": \"string\" } },\n  \"required\": [\"verdict\"]\n}", TextStyle { line_height: 18.0, ..TextStyle::mono(12.0) }, c(look::WHITE)).w_full().h(6.0 * 18.0)),
        ];
        schema.push(match err {
            Some(e) => help_colored(&e, c(look::DANGER)),
            None => div().pt(4.0).child(
                div().row().wrap_children().min_w0()
                    .child(text("Agent must return a JSON object matching this schema. Its fields become ", 11.0, c(look::GRAY_500)).line_height(17.9))
                    .child(text("{{steps.<step>.<field>}}", 11.0, c(look::GRAY_400)).mono().line_height(17.9))
                    .child(text(" for later nodes. A run that doesn't match is marked failed.", 11.0, c(look::GRAY_500)).line_height(17.9)),
            ),
        });
        exec.push(block(schema));
        let mins = n
            .pointer("/config/timeoutMs")
            .and_then(Value::as_f64)
            .map(|ms| ((ms / 60_000.0).round() as i64).to_string())
            .unwrap_or_default();
        let default = s
            .store
            .config
            .pointer("/defaults/headlessStepTimeoutMinutes")
            .and_then(Value::as_f64)
            .map_or(30, |m| m as i64);
        let zero = n.pointer("/config/timeoutMs").and_then(Value::as_f64) == Some(0.0);
        exec.push(block(vec![
            label("Timeout"),
            div().row().items_center().gap(8.0)
                .child(div().w(96.0).child(input(s, bind(Some(id), "config.timeoutMs", Conv::Minutes, false), &mins, &default.to_string(), false, Wash::Strong, 13.0, 12.0)))
                .child(text("minutes", 12.0, c(look::GRAY_500)).line_height(18.0)),
            help(&if zero {
                "No limit. A step whose agent never exits keeps its run open indefinitely.".to_owned()
            } else {
                format!("Kills the agent and fails the step if it hasn't exited by then, so one stuck agent can't hold the run open. Blank uses the {default}-minute default; 0 means no limit.")
            }, 6.0),
        ]));
    }
    out.push(
        div()
            .col()
            .w_full()
            .gap(12.0)
            .p(12.0)
            .rounded(8.0)
            .border(1.0, white(0.06))
            .children(exec),
    );
    let open = s.st.wf.advanced.contains(id)
        || cfg
            .get("args")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
    let mut adv = vec![advanced(s, id, open)];
    if open {
        let args: Vec<&str> = cfg
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        adv.push(div().pt(12.0).child(block(vec![
            label("Extra Arguments"),
            input(
                s,
                bind(Some(id), "config.args", Conv::Words, false),
                &args.join(" "),
                "e.g. --dangerously-skip-permissions",
                true,
                Wash::Strong,
                13.0,
                12.0,
            ),
        ])));
    }
    out.push(div().col().w_full().children(adv));
    section(out)
}

fn help_colored(t: &str, color: Rgba) -> El {
    div()
        .pt(4.0)
        .child(text(t, 11.0, color).line_height(16.5).wrapping().min_w0())
}

fn condition_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let op = cfg_str(n, "operator");
    let ops = [
        ("equals", "equals"),
        ("notEquals", "not equals"),
        ("contains", "contains"),
        ("notContains", "not contains"),
        ("isEmpty", "is empty"),
        ("isNotEmpty", "is not empty"),
    ];
    let op_label = ops.iter().find(|(v, _)| *v == op).map(|(_, l)| *l);
    let hide = matches!(op, "isEmpty" | "isNotEmpty");
    let mut out = vec![
        block(vec![
            label("Variable"),
            var_area(
                s,
                d,
                bind(Some(id), "config.variable", Conv::Keep, true),
                cfg_str(n, "variable"),
                "e.g. {{steps.build.status}}",
                1,
                true,
            ),
        ]),
        block(vec![
            label("Operator"),
            select(
                s,
                token(380.0, Some(id), "config.operator"),
                op_label,
                None,
                "",
                Wash::Strong,
            ),
        ]),
    ];
    if !hide {
        out.push(block(vec![
            label("Value"),
            var_area(
                s,
                d,
                bind(Some(id), "config.value", Conv::Keep, true),
                cfg_str(n, "value"),
                "Compare against...",
                1,
                true,
            ),
        ]));
    }
    let var = cfg_str(n, "variable");
    if !var.is_empty() {
        let val = cfg_str(n, "value");
        let tail = if !hide && !val.is_empty() {
            format!(" \"{val}\"")
        } else {
            String::new()
        };
        out.push(
            div()
                .col()
                .w_full()
                .px(12.0)
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.02))
                .border(1.0, white(0.04))
                .child(
                    div().pb(4.0).child(
                        text("EVALUATES", 11.0, c(look::GRAY_500))
                            .tracking(0.05)
                            .line_height(16.5),
                    ),
                )
                .child(
                    text(
                        format!("{var} {}{tail}", op_label.unwrap_or("")),
                        11.0,
                        c(look::GRAY_400),
                    )
                    .mono()
                    .line_height(16.5)
                    .truncate(),
                ),
        );
    }
    section(out)
}

fn loop_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let mode = n
        .pointer("/config/mode")
        .and_then(Value::as_str)
        .unwrap_or("repeat");
    let mut radios = div()
        .row()
        .gap(2.0)
        .p(2.0)
        .rounded(6.0)
        .border(1.0, white(0.08))
        .role(Role::RadioGroup, "How the loop repeats");
    for (v, l) in [
        ("repeat", "A number of times"),
        ("forEach", "For each item in a list"),
    ] {
        let on = mode == v;
        let rid = s.acts.on(
            "wf-loop-mode",
            act(WfAct::Choose(
                Some(id.to_owned()),
                "config.mode".into(),
                v.into(),
            )),
        );
        let hot = s.ui.hover(rid);
        radios = radios.child(
            div()
                .id(rid)
                .on(Action::Click)
                .flex(1.0)
                .row()
                .justify_center()
                .px(10.0)
                .py(6.0)
                .rounded(4.0)
                .bg(if on { white(0.08) } else { white(0.0) })
                .role(Role::RadioButton, l)
                .toggled(on)
                .child(
                    text(
                        l,
                        12.0,
                        if on {
                            c(look::WHITE)
                        } else {
                            c(look::GRAY_400).mix(c(look::GRAY_200), hot)
                        },
                    )
                    .line_height(18.0)
                    .silent(),
                ),
        );
    }
    let mut out = vec![block(vec![label("Repeat"), radios])];
    if mode == "forEach" {
        out.push(block(vec![
            label("Items"),
            var_area(s, d, bind(Some(id), "config.items", Conv::Keep, true), cfg_str(n, "items"), "{{steps.review.items}}", 1, true),
            help("A list from an earlier step: a JSON array, or an object holding one. Each pass reads its item as {{loop.item}}, and every item gets a pass.", 6.0),
        ]));
    } else {
        let max = n
            .pointer("/config/maxIterations")
            .map_or("1".to_owned(), |v| wf::text_of(Some(v)));
        out.push(block(vec![
            label("Maximum passes"),
            input(s, bind(Some(id), "config.maxIterations", Conv::Passes, false), &max, "", false, Wash::Quiet, 13.0, 12.0),
            help("This is what ends the loop, not a safety net: an agent asked whether its own work is good enough tends to say yes. Capped at 10.", 6.0),
        ]));
    }
    let ops = [
        ("equals", "equals"),
        ("notEquals", "does not equal"),
        ("contains", "contains"),
        ("notContains", "does not contain"),
        ("isEmpty", "is empty"),
        ("isNotEmpty", "is not empty"),
    ];
    let uop = n
        .pointer("/config/until/operator")
        .and_then(Value::as_str)
        .unwrap_or("equals");
    let uvar = n
        .pointer("/config/until/variable")
        .and_then(Value::as_str)
        .unwrap_or("");
    let uval = n
        .pointer("/config/until/value")
        .and_then(Value::as_str)
        .unwrap_or("");
    out.push(block(vec![
        label_opt("Stop early when", "(optional)"),
        div()
            .col()
            .w_full()
            .gap(8.0)
            .child(var_area(
                s,
                d,
                bind(Some(id), "config.until.variable", Conv::Keep, true),
                uvar,
                "{{steps.review.approved}}",
                1,
                true,
            ))
            .child(
                div()
                    .row()
                    .gap(8.0)
                    .child(div().w(150.0).child(select(
                        s,
                        token(150.0, Some(id), "config.until.operator"),
                        ops.iter().find(|(v, _)| *v == uop).map(|(_, l)| *l),
                        None,
                        "",
                        Wash::Quiet,
                    )))
                    .child(div().grow().min_w0().child(input(
                        s,
                        bind(Some(id), "config.until.value", Conv::Keep, false),
                        uval,
                        "true",
                        false,
                        Wash::Quiet,
                        13.0,
                        12.0,
                    ))),
            ),
        help(
            &format!(
                "Checked after each pass, against a step's typed output. {}",
                if mode == "forEach" {
                    "Leave blank to go through every item."
                } else {
                    "Leave blank to always run the maximum."
                }
            ),
            6.0,
        ),
    ]));
    section(out)
}

fn approval_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let slug = sv(n, "slug")
        .filter(|x| !x.is_empty())
        .unwrap_or("this_gate");
    let steps = crate::vars::ancestors(d.nodes(), d.edges(), id);
    let feedback = n.pointer("/config/feedback").filter(|f| f.is_object());
    // One run of text, so it wraps the way the paragraph does.
    let code = |before: &str, code: String, after: &str| {
        div().pt(6.0).child(
            text(format!("{before}{code}{after}"), 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .wrapping()
                .min_w0(),
        )
    };
    let help2 = |t: &str| {
        div().pt(6.0).child(
            text(t, 11.0, c(look::GRAY_500))
                .line_height(16.0)
                .wrapping()
                .min_w0(),
        )
    };
    let mut fb = vec![div()
        .row()
        .items_center()
        .justify_between()
        .gap(12.0)
        .child(
            text("Reviewer can request changes", 13.0, c(look::GRAY_400))
                .weight(500)
                .line_height(19.5),
        )
        .child(toggle(
            s,
            WfAct::Toggle("config.feedback".into()),
            feedback.is_some(),
            feedback.is_none() && steps.is_empty(),
            "Reviewer can request changes",
        ))];
    if feedback.is_none() && steps.is_empty() {
        fb.push(help2(
            "Add a step before this gate to send the work back to.",
        ));
    }
    if let Some(f) = feedback {
        let from = sv(f, "from").unwrap_or("");
        let from_label = steps
            .iter()
            .find(|u| sv(u, "id") == Some(from))
            .map(|u| {
                sv(u, "label")
                    .filter(|l| !l.is_empty())
                    .unwrap_or(from)
                    .to_owned()
            })
            .unwrap_or_else(|| "Pick a step".into());
        let rounds = f
            .get("maxRounds")
            .map_or(String::new(), |r| wf::text_of(Some(r)));
        fb.push(
            div()
                .pt(12.0)
                .col()
                .gap(10.0)
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .child(text("Redo from", 12.5, c(look::GRAY_400)).line_height(18.75))
                        .child(div().grow().min_w0().child(select(
                            s,
                            token(280.0, Some(id), "feedback.from"),
                            Some(&from_label),
                            None,
                            "",
                            Wash::Quiet,
                        ))),
                )
                .child(
                    div()
                        .row()
                        .items_center()
                        .gap(8.0)
                        .child(text("up to", 12.5, c(look::GRAY_400)).line_height(18.75))
                        .child(div().w(64.0).child(input(
                            s,
                            bind(Some(id), "config.feedback.maxRounds", Conv::Rounds, false),
                            &rounds,
                            "",
                            true,
                            Wash::Quiet,
                            12.0,
                            8.0,
                        )))
                        .child(text("rounds", 12.5, c(look::GRAY_400)).line_height(18.75)),
                )
                .child(code(
                    "Those steps run again with the comment, read as ",
                    format!("{{{{steps.{slug}.feedback}}}}"),
                    ", then the gate asks again.",
                )),
        );
    }
    let timeout = n
        .pointer("/config/timeoutMs")
        .and_then(Value::as_f64)
        .filter(|ms| *ms > 0.0)
        .map(|ms| ((ms / 1000.0).round() as i64).to_string())
        .unwrap_or_default();
    section(vec![
        block(vec![label("Message"), var_area(s, d, bind(Some(id), "config.message", Conv::Keep, true), cfg_str(n, "message"), "Shown with the answers, e.g. {{steps.draft.output}}", 3, false), help2("Step outputs fill in when the gate opens.")]),
        block(vec![
            label_opt("Editable text", "(optional)"),
            var_area(s, d, bind(Some(id), "config.edit", Conv::Text, true), cfg_str(n, "edit"), "{{steps.draft.output}}", 1, true),
            code("The reviewer may rewrite this before approving. Later steps read ", format!("{{{{steps.{slug}.text}}}}"), ", which is the rewrite when there is one and the original otherwise."),
        ]),
        block(vec![
            label_opt("Review page", "(optional)"),
            var_area(s, d, bind(Some(id), "config.view", Conv::Text, true), cfg_str(n, "view"), "{{steps.review_page.output}}", 1, true),
            help2("HTML, or the path of a .html file. Opens beside the answers, sandboxed, with no network."),
        ]),
        block(fb),
        block(vec![
            label_opt("Timeout", "(seconds, optional)"),
            input(s, bind(Some(id), "config.timeoutMs", Conv::Seconds, false), &timeout, "Leave blank to wait forever", false, Wash::Quiet, 13.0, 12.0),
            help2("Auto-rejects the gate if nobody answers in time. Each round starts a new timeout."),
        ]),
    ])
}

fn create_task_form(s: &Screen<'_>, id: &str, n: &Value) -> El {
    let project = cfg_str(n, "project");
    let plabel = if project == "fromConnection" {
        "From connection"
    } else {
        project
    };
    let st = match cfg_str(n, "initialStatus") {
        "in_progress" => "In Progress",
        "in_review" => "In Review",
        "done" => "Done",
        "cancelled" => "Cancelled",
        _ => "To Do",
    };
    section(vec![
        block(vec![
            label("Project"),
            select(
                s,
                token(380.0, Some(id), "config.project"),
                Some(plabel),
                None,
                "",
                Wash::Strong,
            ),
            help(
                "\"From connection\" uses the executionProject set on the source connection.",
                6.0,
            ),
        ]),
        block(vec![
            label("Initial Status"),
            select(
                s,
                token(380.0, Some(id), "config.initialStatus"),
                Some(st),
                None,
                "",
                Wash::Strong,
            ),
            help(
                "Applied on first import. Local status edits are never overwritten by re-sync.",
                6.0,
            ),
        ]),
    ])
}

fn http_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let prof = cfg_str(n, "profileConnectionId");
    let prof_label = if prof.is_empty() {
        Some("None".to_owned())
    } else {
        s.st.wf
            .connections
            .iter()
            .find(|c| sv(c, "id") == Some(prof))
            .and_then(|c| sv(c, "name"))
            .map(str::to_owned)
    };
    let headers: Vec<(String, String)> = n
        .pointer("/config/headers")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let mut rows = div().col().w_full().gap(6.0);
    for (i, (name, value)) in headers.iter().enumerate() {
        let rm = s.acts.on(
            "wf-header-remove",
            act(WfAct::RemoveHeader(id.to_owned(), i)),
        );
        let hot = s.ui.hover(rm);
        let nb = bind(Some(id), &format!("header.{i}.name"), Conv::Keep, false);
        let nid = field_of(&nb);
        s.acts
            .text(nid, Act::Workflow(WfAct::Text(nb)), name.as_str());
        rows = rows.child(
            div()
                .row()
                .items_center()
                .gap(6.0)
                .child(
                    div()
                        .w(140.0)
                        .px(8.0)
                        .py(6.0)
                        .rounded(4.0)
                        .bg(white(0.03))
                        .border(1.0, white(0.08))
                        .child(
                            field(
                                nid,
                                "Name",
                                "Name",
                                TextStyle {
                                    line_height: 18.0,
                                    ..TextStyle::mono(12.0)
                                },
                                c(look::WHITE),
                            )
                            .w_full(),
                        ),
                )
                .child(div().grow().min_w0().child(var_area(
                    s,
                    d,
                    bind(Some(id), &format!("header.{i}.value"), Conv::Keep, true),
                    value,
                    "Value",
                    1,
                    true,
                )))
                .child(
                    div()
                        .id(rm)
                        .on(Action::Click)
                        .p(4.0)
                        .rounded(4.0)
                        .bg(white(0.0).mix(white(0.06), hot))
                        .role(Role::Button, "Remove header")
                        .child(ico(
                            "x",
                            12.0,
                            2.0,
                            c(look::GRAY_500).mix(c(look::WHITE), hot),
                        )),
                ),
        );
    }
    let add = s
        .acts
        .on("wf-header-add", act(WfAct::AddHeader(id.to_owned())));
    let ahot = s.ui.hover(add);
    rows = rows.child(
        div().row().child(
            div()
                .id(add)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(4.0)
                .role(Role::Button, "Add header")
                .child(ico(
                    "plus",
                    11.0,
                    2.0,
                    c(look::GRAY_500).mix(c(look::GRAY_300), ahot),
                ))
                .child(
                    text(
                        "Add header",
                        11.0,
                        c(look::GRAY_500).mix(c(look::GRAY_300), ahot),
                    )
                    .line_height(16.5)
                    .silent(),
                ),
        ),
    );
    section(vec![
        div().row().gap(8.0).w_full()
            .child(div().w(110.0).child(block(vec![label("Method"), select(s, token(110.0, Some(id), "config.method"), Some(cfg_str(n, "method")), None, "", Wash::Strong)])))
            .child(div().grow().min_w0().child(block(vec![
                label("URL"),
                var_area(s, d, bind(Some(id), "config.url", Conv::Keep, true), cfg_str(n, "url"), if prof.is_empty() { "https://api.example.com/items" } else { "/v1/items" }, 1, true),
            ]))),
        block(vec![
            label("Auth profile"),
            select(s, token(380.0, Some(id), "config.profileConnectionId"), prof_label.as_deref(), None, "", Wash::Strong),
            help_faint("A profile adds its base URL and auth to the request on the server, so its secret never enters this workflow."),
        ]),
        block(vec![label("Headers"), rows]),
        block(vec![
            label("Body"),
            var_area(s, d, bind(Some(id), "config.body", Conv::Keep, true), cfg_str(n, "body"), "{\"name\": \"{{trigger.body.name}}\"}", 6, true),
            help_faint("Sent as-is. GET requests send no body."),
        ]),
    ])
}

fn action_form(s: &Screen<'_>, d: &Draft, id: &str, n: &Value) -> El {
    let conn = cfg_str(n, "connectionId");
    let current =
        s.st.wf
            .connections
            .iter()
            .find(|c| sv(c, "id") == Some(conn));
    let src = crate::library::Sources {
        connections: &s.st.wf.connections,
        packs: &s.st.wf.lib.packs,
        ..crate::library::Sources::default()
    };
    let unbound = conn.is_empty() && !cfg_str(n, "connectorId").is_empty();
    let mut first = vec![
        label("Connection"),
        select(
            s,
            token(380.0, Some(id), "action.connection"),
            current.and_then(|c| sv(c, "name")),
            current.map(|_| mark(crate::library::look_for(&src, conn).as_ref(), 14.0)),
            "Select a connection...",
            Wash::Strong,
        ),
    ];
    if unbound {
        let a = Some(cfg_str(n, "actionLabel"))
            .filter(|x| !x.is_empty())
            .unwrap_or(cfg_str(n, "action"));
        first.push(help(&format!("{a} comes from a connector that is not installed yet \u{2014} the workflow panel offers to install and connect it."), 6.0));
    } else if s.st.wf.connections.is_empty() {
        first.push(help(
            "No connections yet. Add one from Settings \u{203a} Connectors first.",
            6.0,
        ));
    }
    let mut out = vec![block(first)];
    if current.is_some() {
        let actions = s.st.wf.lib.actions.get(conn).cloned().unwrap_or_default();
        let action = cfg_str(n, "action");
        let selected = actions.iter().find(|a| sv(a, "type") == Some(action));
        let mut b = vec![
            label("Action"),
            select(
                s,
                token(380.0, Some(id), "action.action"),
                selected.and_then(|a| sv(a, "label")),
                None,
                if actions.is_empty() {
                    "No actions available for this connection"
                } else {
                    "Select an action..."
                },
                Wash::Strong,
            ),
        ];
        if let Some(desc) = selected.and_then(|a| sv(a, "description")) {
            b.push(help(desc, 6.0));
        }
        out.push(block(b));
    }
    let fields = arg_fields(s, id);
    if !fields.is_empty() {
        let mut args = div().col().w_full().gap(12.0).child(
            div()
                .row()
                .wrap_children()
                .child(
                    text("ARGUMENTS ", 11.0, c(look::GRAY_500))
                        .tracking(0.05)
                        .line_height(16.5),
                )
                .child(text("\u{b7} type ", 11.0, c(look::GRAY_600)).line_height(16.5))
                .child(text("{{", 11.0, c(look::GRAY_500)).mono().line_height(16.5))
                .child(
                    text(" to pick from previous steps", 11.0, c(look::GRAY_600)).line_height(16.5),
                ),
        );
        for f in &fields {
            let key = sv(f, "key").unwrap_or("");
            let value = n
                .pointer(&format!("/config/args/{key}"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let mut lab = div().row().pb(4.0).child(
                text(sv(f, "label").unwrap_or(key), 12.0, c(look::GRAY_500)).line_height(16.0),
            );
            if f.get("required").and_then(Value::as_bool) == Some(true) {
                lab = lab.child(
                    div()
                        .pl(2.0)
                        .child(text("*", 12.0, c(look::DANGER)).line_height(16.0)),
                );
            }
            let choices: Vec<&Value> = f
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .collect();
            let listed = choices.iter().any(|o| sv(o, "value") == Some(value));
            let pickable = sv(f, "type") == Some("select") && !choices.is_empty();
            let free = s.st.wf.free_args.contains(&(id.to_owned(), key.to_owned()))
                || (!value.is_empty() && !listed);
            let mut fb = vec![lab];
            let link = |label: &str, to_free: bool| {
                let lid = s.acts.on(
                    "wf-arg-mode",
                    act(WfAct::ArgFree(id.to_owned(), key.to_owned(), to_free)),
                );
                let hot = s.ui.hover(lid);
                div().pt(4.0).row().child(
                    div()
                        .id(lid)
                        .on(Action::Click)
                        .role(Role::Button, label)
                        .child(
                            text(label, 10.0, c(look::GRAY_500).mix(c(look::GRAY_300), hot))
                                .line_height(15.0)
                                .silent(),
                        ),
                )
            };
            if pickable && !free {
                let cur = choices
                    .iter()
                    .find(|o| sv(o, "value") == Some(value))
                    .and_then(|o| sv(o, "label"));
                fb.push(select(
                    s,
                    token(380.0, Some(id), &format!("args.{key}")),
                    cur,
                    None,
                    sv(f, "placeholder").unwrap_or("\u{2014}"),
                    Wash::Strong,
                ));
                fb.push(link("Use a template instead", true));
            } else {
                let rows = if sv(f, "type") == Some("textarea") {
                    3
                } else {
                    1
                };
                fb.push(var_area(
                    s,
                    d,
                    bind(Some(id), &format!("config.args.{key}"), Conv::Keep, true),
                    value,
                    sv(f, "placeholder").unwrap_or(""),
                    rows,
                    true,
                ));
                if pickable {
                    fb.push(link("Choose from the list", false));
                }
            }
            if let Some(desc) = sv(f, "description") {
                fb.push(
                    div().pt(2.0).child(
                        text(desc, 10.0, c(look::GRAY_600))
                            .line_height(15.0)
                            .wrapping()
                            .min_w0(),
                    ),
                );
            }
            args = args.child(block(fb));
        }
        out.push(args);
    }
    section(out)
}

/// `WorkflowPropertiesPanel`.
pub fn properties(s: &Screen<'_>, d: &Draft) -> El {
    let enabled = d.wf.get("enabled").and_then(Value::as_bool) != Some(false);
    let stagger =
        d.wf.get("staggerDelayMs")
            .map_or(String::new(), |v| wf::text_of(Some(v)));
    let creates = d
        .nodes()
        .iter()
        .filter(|n| sv(n, "type") == Some("launchAgent"))
        .map(|n| worktree_mode(&n.get("config").cloned().unwrap_or_default()))
        .collect::<Vec<_>>();
    let inherited =
        creates.iter().any(|m| m == "fromContext") && !creates.iter().any(|m| m == "new");
    let cleanup = d.wf.get("autoCleanupWorktrees").and_then(Value::as_bool) == Some(true);
    let row = |label: &str, el: El| {
        div()
            .row()
            .items_center()
            .min_h(32.0)
            .py(4.0)
            .child(
                div()
                    .w(110.0)
                    .shrink0()
                    .child(text(label, 12.0, c(look::GRAY_500)).line_height(18.0)),
            )
            .child(div().grow().min_w0().child(el))
    };
    let trig = wf::trigger(d.nodes());
    let summary = match trig
        .and_then(|t| t.pointer("/config/triggerType"))
        .and_then(Value::as_str)
    {
        None if trig.is_none() => "None".to_owned(),
        Some("manual") => "Manual".to_owned(),
        Some("once") => format!(
            "Once \u{2014} {}",
            trig.and_then(|t| t.pointer("/config/runAt"))
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
                .map_or("not set".to_owned(), crate::tasks::long_date)
        ),
        Some("recurring") => format!(
            "Recurring \u{2014} {}",
            trig.and_then(|t| t.pointer("/config/cron"))
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
                .unwrap_or("not set")
        ),
        Some("taskCreated") => "Task created".to_owned(),
        Some("taskStatusChanged") => "Task status changed".to_owned(),
        _ => "Unknown".to_owned(),
    };
    let tid = s.acts.on(
        "wf-props-trigger",
        trig.and_then(|t| sv(t, "id"))
            .map_or(Act::Nothing, |t| act(WfAct::SelectNode(t.to_owned()))),
    );
    let thot = s.ui.hover(tid);
    let sb = bind(None, "staggerDelayMs", Conv::Count, false);
    let sid = field_of(&sb);
    s.acts.text(sid, Act::Workflow(WfAct::Text(sb)), stagger);
    let mut body = div()
        .col()
        .w_full()
        .p(20.0)
        .gap(4.0)
        .child(row(
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
                    .line_height(18.0),
                )
                .child(toggle(
                    s,
                    WfAct::Toggle("enabled".into()),
                    enabled,
                    false,
                    "Status",
                )),
        ))
        .child(row(
            "Stagger delay",
            div()
                .w(80.0)
                .px(8.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.06))
                .border(
                    1.0,
                    if s.ui.focused(sid) {
                        white(0.2)
                    } else {
                        white(0.08)
                    },
                )
                .child(
                    field(
                        sid,
                        "Stagger delay in milliseconds",
                        "0ms",
                        TextStyle {
                            line_height: 18.0,
                            ..TextStyle::ui(12.0)
                        },
                        c(look::GRAY_200),
                    )
                    // The browser's own `darkGray`: this input sets no placeholder colour.
                    .placeholder_color(c(0xa9a9a9))
                    .w_full(),
                ),
        ))
        .child(row(
            "Cleanup worktrees",
            // An inline span: the toggle sits on the text baseline, with a descent under it.
            div()
                .pb(5.0)
                .opacity(if inherited { 0.5 } else { 1.0 })
                .child(toggle(
                    s,
                    WfAct::Toggle("autoCleanupWorktrees".into()),
                    !inherited && cleanup,
                    inherited,
                    "Cleanup worktrees",
                )),
        ))
        .child(div().h(8.0))
        .child(row(
            "Trigger",
            div()
                .id(tid)
                .on(Action::Click)
                .role(Role::Button, &summary)
                .child(
                    text(&summary, 12.0, c(look::GRAY_200).mix(c(look::WHITE), thot))
                        .line_height(18.0)
                        .truncate(),
                ),
        ));
    if let Some(last) = s.st.wf.history.first() {
        let st = runs::run_status(last);
        let (rgb, a) = runs::dot(st);
        let started = sv(last, "startedAt").unwrap_or("");
        body = body.child(row(
            "Last run",
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(div().size(6.0, 6.0).rounded(3.0).bg(Rgba::hexa(rgb, a)))
                .child(
                    text(
                        if started.is_empty() {
                            "Unknown".to_owned()
                        } else {
                            runs::relative(started, crate::state::now_ms())
                        },
                        12.0,
                        c(look::GRAY_300),
                    )
                    .line_height(18.0),
                ),
        ));
    }
    let close = s.acts.on("wf-panel-close", act(WfAct::ClosePanel));
    let chot = s.ui.hover(close);
    div()
        .col()
        .w(340.0)
        .shrink0()
        .h_full()
        .clip()
        .bg(c(look::SURFACE_SUNKEN))
        .border_left(1.0, white(0.08))
        .role(Role::Group, "Properties")
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .px(20.0)
                .py(16.0)
                .border_bottom(1.0, white(0.08))
                .child(
                    text("Properties", 13.0, c(look::WHITE))
                        .weight(500)
                        .line_height(19.5),
                )
                .child(
                    div()
                        .id(close)
                        .on(Action::Click)
                        .p(4.0)
                        .rounded(6.0)
                        .role(Role::Button, "Close properties panel")
                        .child(ico(
                            "x",
                            14.0,
                            2.0,
                            c(look::GRAY_500).mix(c(look::WHITE), chot),
                        )),
                ),
        )
        .child(
            div()
                .id(crate::ui::Id::new("wf-props-body"))
                .col()
                .grow()
                .min_h(0.0)
                .scroll_y()
                .child(body),
        )
}
