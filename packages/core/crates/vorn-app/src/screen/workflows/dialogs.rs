//! The workflow dialogs: `SourcePromptDialog`, `MissedScheduleDialog` and
//! the export or import path.

use serde_json::Value;

use super::act;
use crate::act::Act;
use crate::look;
use crate::runs::s as sv;
use crate::screen::{c, ico, white, Screen};
use crate::state::{Dialog, MenuKind};
use crate::ui::{area, div, field, text, Action, El, Id, Rgba, Role, TextStyle};
use crate::workflows::{self as wf, Prompt, WfAct};

pub fn dialog(s: &Screen<'_>) -> Option<El> {
    Some(match s.st.dialog.as_ref()? {
        Dialog::RunPrompt => prompt(s, s.st.wf.prompt.as_ref()?),
        Dialog::Missed => missed(s)?,
        Dialog::TurnOn(id) => turn_on(s, id)?,
        _ => return None,
    })
}

fn frame(s: &Screen<'_>, w: f32, name: &str, body: El, closes: bool) -> El {
    let panel = div()
        .col()
        .w(w)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, name)
        .child(body);
    let panel = s.acts.click(panel, "wf-dialog", Act::Nothing);
    let back = if closes {
        super::super::dialogs::backdrop(s, 0.4)
    } else {
        let id = s.acts.on("wf-dialog-backdrop", Act::Nothing);
        div()
            .id(id)
            .on(Action::Click)
            .absolute(0.0, 0.0)
            .size(s.size.0, s.size.1)
            .bg(crate::ui::theme::shadow(0.5))
            .layered()
    };
    back.center().child(panel)
}

fn rule() -> El {
    div().h(1.0).w_full().bg(white(0.06))
}

fn upper(t: &str, required: bool) -> El {
    let mut row = div().row().pb(8.0).child(
        text(t.to_uppercase(), 11.0, c(look::GRAY_500))
            .weight(500)
            .tracking(0.05)
            .line_height(16.5),
    );
    if required {
        row = row.child(
            text("*", 11.0, c(look::DANGER))
                .weight(500)
                .line_height(16.5)
                .ml(4.0),
        );
    }
    row
}

/// `INPUT_CLASS`.
fn boxed(s: &Screen<'_>, id: Id, el: El) -> El {
    div()
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.06))
        .border(
            1.0,
            if s.ui.focused(id) {
                white(0.20)
            } else {
                white(0.10)
            },
        )
        .frames(id)
        .child(el.w_full())
}

fn input(s: &Screen<'_>, id: Id, name: &str, placeholder: &str) -> El {
    boxed(
        s,
        id,
        field(id, name, placeholder, TextStyle::ui(13.0), c(look::WHITE)),
    )
}

fn branch_box(s: &Screen<'_>, id: Id, name: &str, placeholder: &str) -> El {
    div()
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.06))
        .border(
            1.0,
            if s.ui.focused(id) {
                white(0.20)
            } else {
                white(0.10)
            },
        )
        .frames(id)
        .child(ico("git-branch", 12.0, 2.0, c(look::GRAY_500)))
        .child(
            field(id, name, placeholder, TextStyle::ui(13.0), c(look::WHITE))
                .grow()
                .min_w0(),
        )
}

fn picker(s: &Screen<'_>, kind: MenuKind, shown: &str, empty: bool) -> El {
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on("wf-dialog-pick", Act::Open(kind));
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .justify_between()
        .gap(8.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(white(0.06).mix(white(0.08), hot))
        .border(1.0, if open { white(0.20) } else { white(0.10) })
        .role(Role::Button, shown)
        .has_popup()
        .expanded(open)
        .child(
            text(
                shown,
                13.0,
                c(if empty { look::GRAY_600 } else { look::WHITE }),
            )
            .line_height(18.0)
            .truncate()
            .silent(),
        )
        .child(ico("chevron-down", 12.0, 2.0, c(look::GRAY_500)))
}

/// A browser's own `<input type=checkbox>`: 13 px, white with a gray edge,
/// filled with its accent once checked.
fn browser_checkbox(on: bool, accent: Rgba, mark: Rgba) -> El {
    let bx = div().size(13.0, 13.0).rounded(2.0).center().shrink0();
    if on {
        bx.bg(accent).child(ico("check", 10.0, 3.0, mark))
    } else {
        bx.bg(c(look::WHITE)).border(1.0, c(look::GRAY_500))
    }
}

fn checkbox(s: &Screen<'_>, a: Act, on: bool, label: &str) -> El {
    let id = s.acts.on("wf-dialog-check", a);
    let bx = browser_checkbox(on, white(0.8), c(look::SURFACE_BASE));
    div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(8.0)
        .role(Role::CheckBox, label)
        .toggled(on)
        .child(bx)
        .child(
            text(label, 13.0, c(look::GRAY_300))
                .line_height(19.5)
                .silent(),
        )
}

fn footer_button(s: &Screen<'_>, a: Act, label: &str, primary: bool, ready: bool) -> El {
    let id = s.acts.on("wf-dialog-foot", a);
    let hot = if ready { s.ui.hover(id) } else { 0.0 };
    let mut b = div()
        .id(id)
        .py(6.0)
        .px(if primary { 16.0 } else { 12.0 })
        .rounded(6.0)
        .bg(if primary {
            white(0.10).mix(white(0.15), hot)
        } else {
            white(0.0).mix(white(0.06), hot)
        })
        .opacity(if ready { 1.0 } else { 0.4 })
        .role(Role::Button, label)
        .disabled(!ready)
        .child(
            text(
                label,
                13.0,
                if primary {
                    c(look::WHITE)
                } else {
                    c(look::GRAY_300).mix(c(look::WHITE), hot)
                },
            )
            .line_height(19.5)
            .silent(),
        );
    if ready {
        b = b.on(Action::Click);
    }
    b
}

fn prompt_value(s: &Screen<'_>, p: &Prompt, key: &str) -> String {
    p.picked
        .get(key)
        .cloned()
        .unwrap_or_else(|| s.ui.edit_text(wf::prompt_id(key)).to_owned())
}

/// `WorkflowInputFields`.
fn input_field(s: &Screen<'_>, p: &Prompt, def: &Value) -> El {
    let key = sv(def, "key").unwrap_or("");
    let label = sv(def, "label").filter(|l| !l.is_empty()).unwrap_or(key);
    let kind = sv(def, "type").unwrap_or("text");
    let required = def.get("required").and_then(Value::as_bool) == Some(true);
    let placeholder = sv(def, "placeholder").unwrap_or("");
    let id = wf::prompt_id(key);
    let mut col = div().col();
    if kind != "boolean" {
        col = col.child(upper(label, required));
    }
    let value = || {
        p.picked
            .get(key)
            .cloned()
            .unwrap_or_else(|| wf::input_default(def))
    };
    col = col.child(match kind {
        "textarea" => boxed(
            s,
            id,
            area(id, label, placeholder, TextStyle::ui(13.0), c(look::WHITE)).h(60.0),
        ),
        "select" => {
            let cur = value();
            let shown = def
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|o| sv(o, "value") == Some(cur.as_str()))
                .map(|o| {
                    sv(o, "label")
                        .filter(|l| !l.is_empty())
                        .or(sv(o, "value"))
                        .unwrap_or("")
                        .to_owned()
                });
            let empty = shown.is_none();
            let shown = shown.unwrap_or_else(|| {
                if placeholder.is_empty() {
                    "Select...".to_owned()
                } else {
                    placeholder.to_owned()
                }
            });
            picker(
                s,
                MenuKind::WfChoice(format!("prompt.{key}")),
                &shown,
                empty,
            )
        }
        "project" => {
            let cur = value();
            let empty = cur.is_empty();
            let shown = if empty {
                "Select project".to_owned()
            } else {
                cur
            };
            picker(
                s,
                MenuKind::WfChoice(format!("prompt.{key}")),
                &shown,
                empty,
            )
        }
        "boolean" => {
            let on = value() == "true";
            checkbox(
                s,
                act(WfAct::PromptSet(key.to_owned(), (!on).to_string())),
                on,
                label,
            )
        }
        "branch" => branch_box(
            s,
            id,
            label,
            if placeholder.is_empty() {
                "branch name"
            } else {
                placeholder
            },
        ),
        _ => input(s, id, label, placeholder),
    });
    if let Some(d) = sv(def, "description") {
        col = col.child(
            div().mt(4.0).child(
                text(d, 11.0, c(look::GRAY_500))
                    .line_height(15.0)
                    .wrapping()
                    .min_w0(),
            ),
        );
    }
    col
}

/// `SourcePromptDialog`.
fn prompt(s: &Screen<'_>, p: &Prompt) -> El {
    let title = format!("Run \"{}\"", p.name);
    let sub = if p.contextual {
        "This contextual workflow needs a source. Pick the folder it should run against."
    } else {
        "This workflow needs a few details before it runs."
    };
    let head = div()
        .col()
        .px(24.0)
        .py(16.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(ico("zap", 14.0, 2.0, c(look::BLUE_400)))
                .child(
                    text(title.clone(), 15.0, c(look::WHITE))
                        .weight(500)
                        .line_height(22.0)
                        .role(Role::Heading, &title),
                ),
        )
        .child(
            div().mt(4.0).child(
                text(sub, 12.0, c(look::GRAY_500))
                    .line_height(16.0)
                    .wrapping()
                    .min_w0(),
            ),
        );
    let mut body = div().col().gap(16.0).p(24.0);
    if p.needs_project {
        let shown = p.project.clone();
        let empty = shown.is_none();
        body = body.child(div().col().child(upper("Project", false)).child(picker(
            s,
            MenuKind::PromptProject,
            shown.as_deref().unwrap_or("Select project"),
            empty,
        )));
    }
    if p.needs_branch {
        body = body.child(div().col().child(upper("Branch", false)).child(branch_box(
            s,
            wf::prompt_branch(),
            "Branch",
            "leave blank to use the project's current branch",
        )));
    }
    if p.needs_worktree {
        body = body.child(checkbox(
            s,
            act(WfAct::PromptWorktree),
            p.worktree,
            "Run in a new worktree",
        ));
    }
    for def in &p.inputs {
        body = body.child(input_field(s, p, def));
    }
    let ready = !(p.needs_project && p.project.is_none())
        && wf::inputs_valid(&p.inputs, |k| prompt_value(s, p, k));
    let foot = div()
        .row()
        .justify_end()
        .gap(8.0)
        .px(24.0)
        .py(12.0)
        .child(footer_button(s, Act::CloseDialog, "Cancel", false, true))
        .child(footer_button(s, act(WfAct::PromptRun), "Run", true, ready));
    frame(
        s,
        460.0,
        &title,
        div()
            .col()
            .child(head)
            .child(rule())
            .child(body)
            .child(rule())
            .child(foot),
        true,
    )
}

/// `MissedScheduleDialog`, which only its buttons close.
fn missed(s: &Screen<'_>) -> Option<El> {
    if s.st.wf.missed.is_empty() {
        return None;
    }
    let head = div()
        .col()
        .px(20.0)
        .py(16.0)
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(ico("triangle-alert", 16.0, 2.0, c(look::AMBER_400)))
                .child(
                    text("Missed Schedules", 16.0, c(look::WHITE))
                        .weight(500)
                        .line_height(24.0)
                        .role(Role::Heading, "Missed Schedules"),
                ),
        )
        .child(
            div().mt(4.0).child(
                text(
                    "These workflows were scheduled while Vorn was closed.",
                    14.0,
                    c(look::GRAY_500),
                )
                .line_height(20.0)
                .wrapping()
                .min_w0(),
            ),
        );
    let mut list = div().col().gap(8.0).p(16.0);
    for (i, m) in s.st.wf.missed.iter().enumerate() {
        let id = s.acts.on("wf-missed", act(WfAct::MissedToggle(i)));
        let hot = s.ui.hover(id);
        let bx = browser_checkbox(m.on, c(look::BLUE_600), c(look::WHITE));
        let when = crate::tasks::locale_string(&m.at);
        list = list.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(12.0)
                .p(12.0)
                .rounded(8.0)
                .border(1.0, white(0.06))
                .bg(white(0.03).mix(white(0.05), hot))
                .role(Role::CheckBox, &m.name)
                .toggled(m.on)
                .child(bx)
                .child(
                    div()
                        .col()
                        .grow()
                        .min_w0()
                        .child(
                            text(m.name.as_str(), 14.0, c(look::GRAY_200))
                                .line_height(20.0)
                                .truncate()
                                .silent(),
                        )
                        .child(
                            div()
                                .row()
                                .items_center()
                                .gap(4.0)
                                .mt(2.0)
                                .child(ico("clock", 10.0, 2.0, c(look::GRAY_500)))
                                .child(
                                    text(format!("Scheduled for {when}"), 11.0, c(look::GRAY_500))
                                        .line_height(15.0)
                                        .silent(),
                                ),
                        ),
                ),
        );
    }
    let n = s.st.wf.missed.iter().filter(|m| m.on).count();
    let skip = s.acts.on("wf-missed-skip", act(WfAct::MissedSkip));
    let skip_hot = s.ui.hover(skip);
    let run_label = format!("Run Selected ({n})");
    let run = s.acts.on("wf-missed-run", act(WfAct::MissedRun));
    let run_hot = if n > 0 { s.ui.hover(run) } else { 0.0 };
    let mut run_btn = div()
        .id(run)
        .px(16.0)
        .py(8.0)
        .rounded(8.0)
        .bg(white(0.10).mix(white(0.15), run_hot))
        .opacity(if n > 0 { 1.0 } else { 0.3 })
        .role(Role::Button, &run_label)
        .disabled(n == 0)
        .child(
            text(run_label.clone(), 14.0, c(look::WHITE))
                .weight(500)
                .line_height(20.0)
                .silent(),
        );
    if n > 0 {
        run_btn = run_btn.on(Action::Click);
    }
    let foot = div()
        .row()
        .justify_end()
        .gap(12.0)
        .px(20.0)
        .py(12.0)
        .child(
            div()
                .id(skip)
                .on(Action::Click)
                .px(16.0)
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.04).mix(white(0.08), skip_hot))
                .role(Role::Button, "Skip All")
                .child(
                    text(
                        "Skip All",
                        14.0,
                        c(look::GRAY_400).mix(c(look::GRAY_200), skip_hot),
                    )
                    .line_height(20.0)
                    .silent(),
                ),
        )
        .child(run_btn);
    Some(frame(
        s,
        420.0,
        "Missed Schedules",
        div()
            .col()
            .child(head)
            .child(rule())
            .child(list)
            .child(rule())
            .child(foot),
        false,
    ))
}

/// The most lines of what a workflow runs the turn-on ask shows.
const RUNS_SHOWN: usize = 16;

/// What a workflow from a project's file runs, read before it is turned on.
fn turn_on(s: &Screen<'_>, id: &str) -> Option<El> {
    let file = s.st.wf.files.get(id)?;
    let name = super::workflow(s, id)
        .and_then(|w| sv(w, "name"))
        .unwrap_or(id);
    let title = format!("Turn on \u{201c}{name}\u{201d}?");
    let head = div()
        .col()
        .gap(4.0)
        .px(20.0)
        .py(16.0)
        .child(
            text(title.clone(), 16.0, c(look::WHITE))
                .weight(500)
                .line_height(24.0)
                .role(Role::Heading, &title),
        )
        .child(
            text(
                format!(
                    "It comes from {}. Once on, it runs what is listed below. If the file changes outside Vorn, it turns off again.",
                    file.path
                ),
                13.0,
                c(look::GRAY_400),
            )
            .line_height(18.0)
            .wrapping()
            .min_w0(),
        );
    let lines: Vec<&str> = file.runs.iter().flat_map(|r| r.lines()).collect();
    let mut list = div()
        .col()
        .gap(2.0)
        .p(12.0)
        .rounded(4.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_SUNKEN));
    for line in lines.iter().take(RUNS_SHOWN) {
        list = list.child(
            text(*line, 12.0, c(look::GRAY_300))
                .mono()
                .line_height(16.0)
                .truncate(),
        );
    }
    if lines.len() > RUNS_SHOWN {
        list = list.child(
            text(
                format!("and {} more lines in the file", lines.len() - RUNS_SHOWN),
                12.0,
                c(look::GRAY_500),
            )
            .line_height(16.0),
        );
    }
    let button = |place: &str, a: Act, label: &str, strong: bool| {
        let bid = s.acts.on(place, a);
        let hot = s.ui.hover(bid);
        div()
            .id(bid)
            .on(Action::Click)
            .px(16.0)
            .py(8.0)
            .rounded(4.0)
            .bg(if strong {
                white(0.10).mix(white(0.15), hot)
            } else {
                white(0.04).mix(white(0.08), hot)
            })
            .role(Role::Button, label)
            .child(
                text(
                    label,
                    14.0,
                    if strong {
                        c(look::WHITE)
                    } else {
                        c(look::GRAY_400).mix(c(look::GRAY_200), hot)
                    },
                )
                .weight(500)
                .line_height(20.0)
                .silent(),
            )
    };
    let foot = div()
        .row()
        .justify_end()
        .gap(12.0)
        .px(20.0)
        .py(12.0)
        .child(button("wf-turn-on-cancel", Act::CloseDialog, "Cancel", false))
        .child(button(
            "wf-turn-on-ok",
            act(WfAct::ConfirmTurnOn(id.to_owned())),
            "Turn on",
            true,
        ));
    Some(frame(
        s,
        480.0,
        &title,
        div()
            .col()
            .child(head)
            .child(rule())
            .child(div().p(16.0).child(list))
            .child(rule())
            .child(foot),
        true,
    ))
}
