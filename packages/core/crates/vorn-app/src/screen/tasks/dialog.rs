//! The Tasks view's dialogs: `AddTaskDialog`, `CommitDialog` and
//! `LogReplayModal`.

use super::detail::images;
use super::editor::editor;
use super::{
    agent_picker, commit_id, faint, primary, project_picker, secondary, status_picker, title_id,
};
use crate::act::Act;
use crate::look;
use crate::screen::dialogs::backdrop;
use crate::screen::{c, ico, white, Screen};
use crate::tasks::{Form, TaskAct};
use crate::ui::{area, div, field, text, Action, El, Id, Rgba, Role, TextStyle};

/// A `p-1` header button that turns white on hover.
#[allow(clippy::too_many_arguments)]
fn head_btn(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    icon: &str,
    size: f32,
    stroke: f32,
    label: &str,
    rest: Rgba,
) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, label)
        .child(ico(icon, size, stroke, rest.mix(c(look::WHITE), hot)))
}

fn rule() -> El {
    div().h(1.0).w_full().shrink0().bg(white(0.06))
}

pub(in crate::screen) fn add_task(s: &Screen<'_>) -> El {
    let f = &s.st.tasks.dialog_form;
    let header = div()
        .row()
        .items_center()
        .justify_between()
        .px(16.0)
        .py(10.0)
        .child(
            text(
                if f.editing { "Edit Task" } else { "New task" },
                13.0,
                faint(),
            )
            .line_height(18.0)
            .role(
                Role::Heading,
                if f.editing { "Edit Task" } else { "New task" },
            ),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .child(head_btn(
                    s,
                    "task-dialog",
                    Act::Task(TaskAct::Expand),
                    "maximize-2",
                    14.0,
                    2.0,
                    "Expand to full panel",
                    faint(),
                ))
                .child(head_btn(
                    s,
                    "task-dialog",
                    Act::Task(TaskAct::CloseDialog),
                    "x",
                    14.0,
                    2.0,
                    "Close",
                    faint(),
                )),
        );
    let title = div()
        .px(16.0)
        .pt(12.0)
        .pb(4.0)
        .frames(title_id(Form::Dialog))
        .child(
            field(
                title_id(Form::Dialog),
                "Task title",
                "Task title",
                TextStyle {
                    weight: 500,
                    ..TextStyle::ui(15.0)
                },
                primary(),
            )
            .w_full(),
        );
    let mut body = div()
        .id(Id::new("task-dialog-body"))
        .col()
        .max_h(s.size.1 * 0.6)
        .scroll_y()
        .child(title)
        .child(div().px(16.0).pb(12.0).child(editor(
            s,
            Form::Dialog,
            "Add description...",
            202.0,
            64,
        )));
    if !f.images.is_empty() || f.attaching {
        body = body.child(div().px(16.0).pb(12.0).child(images(s, Form::Dialog, 56.0)));
    }
    let mut pills = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(16.0)
        .py(8.0)
        .child(status_picker(s, Form::Dialog, false))
        .child(project_picker(s, Form::Dialog))
        .child(agent_picker(s, Form::Dialog));
    if !f.branch.is_empty() || f.editing {
        pills = pills.child(
            div()
                .row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .py(4.0)
                .rounded(999.0)
                .bg(white(0.06))
                .child(ico("folder-git-2", 10.0, 2.0, secondary()))
                .child(
                    text(
                        if f.branch.is_empty() {
                            "branch"
                        } else {
                            &f.branch
                        },
                        12.0,
                        secondary(),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        );
    }
    let attach = s.acts.on(
        "task-dialog-attach",
        Act::Task(TaskAct::AttachImages(Form::Dialog)),
    );
    let ahot = s.ui.hover(attach);
    pills = pills.child(div().grow()).child(
        div()
            .id(attach)
            .on(Action::Click)
            .p(6.0)
            .rounded(4.0)
            .role(Role::Button, "Attach images")
            .child(ico("paperclip", 14.0, 2.0, faint().mix(secondary(), ahot))),
    );
    let ok = !s.ui.edit_text(title_id(Form::Dialog)).trim().is_empty()
        && !f.project.is_empty()
        && !s
            .ui
            .edit_text(super::desc_id(Form::Dialog))
            .trim()
            .is_empty();
    let submit = s.acts.on("task-dialog-submit", Act::Task(TaskAct::Submit));
    let hot = if ok { s.ui.hover(submit) } else { 0.0 };
    let label = if f.editing { "Save" } else { "Create task" };
    let mut btn = div()
        .id(submit)
        .px(12.0)
        .py(6.0)
        .rounded(8.0)
        .bg(white(0.10 + 0.05 * hot))
        .opacity(if ok { 1.0 } else { 0.3 })
        .role(Role::Button, label)
        .disabled(!ok)
        .child(
            text(label, 14.0, c(look::WHITE))
                .weight(500)
                .line_height(20.0)
                .silent(),
        );
    if ok {
        btn = btn.on(Action::Click);
    }
    let footer = div().row().justify_end().px(16.0).py(12.0).child(btn);
    let panel = div()
        .col()
        .w(560.0)
        .rounded(12.0)
        .border(1.0, white(0.10))
        .bg(c(look::SURFACE_OVERLAY))
        .clip()
        .role(Role::Dialog, label)
        .child(header)
        .child(rule())
        .child(body)
        .child(rule())
        .child(pills)
        .child(rule())
        .child(footer);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    backdrop(s, 0.3).center().child(panel)
}

/// `CommitDialog`, for the selected task or the Changes panel's session.
pub(in crate::screen) fn commit(s: &Screen<'_>, scope: super::diff::Scope) -> El {
    use super::diff::Scope;
    use crate::git::CommitOp;
    let session = matches!(scope, Scope::Session);
    let cm = if session {
        &s.st.git.commit
    } else {
        &s.st.tasks.commit
    };
    let act = |op: CommitOp| -> Act {
        if session {
            return Act::Git(crate::git::GitAct::Commit(op));
        }
        match op {
            CommitOp::Open => Act::Task(TaskAct::OpenCommit),
            CommitOp::ToggleUnstaged => Act::Task(TaskAct::ToggleUnstaged),
            CommitOp::Push(p) => Act::Task(TaskAct::CommitPush(p)),
            CommitOp::Commit => Act::Task(TaskAct::Commit),
            CommitOp::Close => Act::CloseDialog,
        }
    };
    let field = if session {
        crate::screen::git::commit_id()
    } else {
        commit_id()
    };
    let place = |p: &str| format!("{}-{p}", if session { "git" } else { "task" });
    let owner = s.st.git.panel.as_deref().and_then(|id| s.store.session(id));
    let task =
        s.st.tasks
            .selected
            .as_deref()
            .and_then(|id| s.store.tasks.iter().find(|t| t.id == id));
    let branch = if session {
        owner.and_then(|o| o.info.branch.as_deref())
    } else {
        task.and_then(|t| t.branch.as_deref())
    }
    .filter(|b| !b.is_empty())
    .unwrap_or("unknown");
    let row = |label: &str, value: El| {
        div()
            .row()
            .items_center()
            .justify_between()
            .child(
                text(label, 14.0, c(look::GRAY_300))
                    .weight(500)
                    .line_height(20.0)
                    .silent(),
            )
            .child(value)
    };
    let header = div()
        .row()
        .items_center()
        .gap(12.0)
        .px(20.0)
        .pt(20.0)
        .pb(12.0)
        .child(div().p(8.0).rounded(8.0).bg(white(0.06)).child(ico(
            "git-commit-horizontal",
            18.0,
            1.5,
            c(look::GRAY_300),
        )))
        .child(
            div().grow().child(
                text("Commit your changes", 18.0, c(look::GRAY_100))
                    .weight(600)
                    .line_height(28.0)
                    .role(Role::Heading, "Commit your changes"),
            ),
        )
        .child(head_btn(
            s,
            &place("commit"),
            act(CommitOp::Close),
            "x",
            18.0,
            1.5,
            "Close",
            c(look::GRAY_400),
        ));
    let branch_row = row(
        "Branch",
        div()
            .row()
            .items_center()
            .gap(6.0)
            .child(ico("git-branch", 14.0, 1.5, c(look::GRAY_500)))
            .child(
                text(branch, 14.0, c(look::GRAY_200))
                    .mono()
                    .line_height(20.0)
                    .silent(),
            ),
    );
    let (n, ins, del) = if session {
        let st = owner
            .and_then(|o| s.st.git.stats.get(&o.info.id))
            .copied()
            .unwrap_or_default();
        (st.files as usize, st.insertions, st.deletions)
    } else {
        let d = &s.st.tasks.diff;
        let (i, x) = d
            .files
            .iter()
            .fold((0, 0), |(i, x), f| (i + f.insertions, x + f.deletions));
        (d.files.len(), i, x)
    };
    let changes = if n > 0 {
        div()
            .row()
            .items_center()
            .gap(8.0)
            .child(
                text(
                    format!("{n} file{}", if n == 1 { "" } else { "s" }),
                    14.0,
                    c(look::GRAY_300),
                )
                .line_height(20.0)
                .silent(),
            )
            .child(
                text(format!("+{ins}"), 14.0, c(look::GREEN_400))
                    .mono()
                    .line_height(20.0)
                    .silent(),
            )
            .child(
                text(format!("-{del}"), 14.0, c(look::RED_400))
                    .mono()
                    .line_height(20.0)
                    .silent(),
            )
    } else {
        text("No changes", 14.0, c(look::GRAY_500))
            .line_height(20.0)
            .silent()
    };
    let tid = s
        .acts
        .on(&place("commit-unstaged"), act(CommitOp::ToggleUnstaged));
    let toggle = div()
        .row()
        .items_center()
        .gap(12.0)
        .child(
            div()
                .id(tid)
                .on(Action::Click)
                .w(40.0)
                .h(22.0)
                .rounded(11.0)
                .bg(if cm.unstaged {
                    c(look::BLUE_500)
                } else {
                    white(0.12)
                })
                .role(Role::Switch, "Include unstaged")
                .toggled(cm.unstaged)
                .child(
                    div()
                        .absolute(if cm.unstaged { 22.0 } else { 3.0 }, 3.0)
                        .size(16.0, 16.0)
                        .rounded(8.0)
                        .bg(c(look::WHITE)),
                ),
        )
        .child(
            text("Include unstaged", 14.0, c(look::GRAY_300))
                .line_height(20.0)
                .silent(),
        );
    let focus = s.ui.focused(field);
    let message = div()
        .col()
        .child(
            div().pb(8.0).child(
                text("Commit message", 14.0, c(look::GRAY_300))
                    .weight(500)
                    .line_height(20.0)
                    .silent(),
            ),
        )
        .child(
            div()
                .w_full()
                .h(3.0 * 21.0 + 16.0)
                .px(12.0)
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.04))
                .border(1.0, if focus { white(0.16) } else { white(0.08) })
                .frames(field)
                .child(
                    area(
                        field,
                        "Commit message",
                        "Enter a commit message...",
                        TextStyle {
                            line_height: 21.0,
                            ..TextStyle::ui(14.0)
                        },
                        c(look::GRAY_200),
                    )
                    .w_full()
                    .h_full(),
                ),
        );
    let mut next = div().col().gap(4.0);
    for (push, label, icon) in [
        (false, "Commit", "git-commit-horizontal"),
        (true, "Commit and push", "arrow-up"),
    ] {
        let on = cm.push == push;
        let id = s.acts.on(&place("commit-next"), act(CommitOp::Push(push)));
        let hot = s.ui.hover(id);
        let color = if on {
            c(look::GRAY_100)
        } else {
            c(look::GRAY_400)
        };
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(12.0)
            .px(12.0)
            .py(10.0)
            .rounded(8.0)
            .bg(if on { white(0.08) } else { white(0.04 * hot) })
            .role(Role::RadioButton, label)
            .selected(on)
            .child(ico(icon, 16.0, 1.5, color))
            .child(
                div()
                    .grow()
                    .child(text(label, 14.0, color).line_height(20.0).silent()),
            );
        if on {
            b = b.child(ico("check", 16.0, 2.0, c(look::GRAY_300)));
        }
        next = next.child(b);
    }
    let next = div()
        .col()
        .child(
            div().pb(8.0).child(
                text("Next steps", 14.0, c(look::GRAY_300))
                    .weight(500)
                    .line_height(20.0)
                    .silent(),
            ),
        )
        .child(next);
    let ok = !s.ui.edit_text(field).trim().is_empty() && !cm.loading;
    let go = s.acts.on(&place("commit-go"), act(CommitOp::Commit));
    let hot = if ok { s.ui.hover(go) } else { 0.0 };
    let mut cont = div()
        .id(go)
        .row()
        .justify_center()
        .items_center()
        .py(10.0)
        .rounded(8.0)
        .bg(if ok {
            c(look::WHITE).mix(c(look::GRAY_200), hot)
        } else {
            white(0.06)
        })
        .role(Role::Button, "Continue")
        .disabled(!ok);
    cont = if cm.loading {
        cont.child(ico("loader-circle", 16.0, 2.0, c(look::GRAY_500)))
    } else {
        cont.child(
            text(
                "Continue",
                14.0,
                if ok {
                    c(look::GRAY_900)
                } else {
                    c(look::GRAY_500)
                },
            )
            .weight(500)
            .line_height(20.0)
            .silent(),
        )
    };
    if ok {
        cont = cont.on(Action::Click);
    }
    let mut body = div()
        .col()
        .gap(16.0)
        .px(20.0)
        .pb(20.0)
        .child(branch_row)
        .child(row("Changes", changes))
        .child(toggle)
        .child(message)
        .child(next);
    if let Some(e) = &cm.error {
        body = body.child(
            div()
                .px(12.0)
                .py(8.0)
                .rounded(8.0)
                .bg(Rgba::hexa(look::RED_500, 0.10))
                .child(text(e, 13.0, c(look::RED_400)).line_height(18.0).wrapping()),
        );
    }
    body = body.child(cont);
    let panel = div()
        .col()
        .w(440.0)
        .rounded(12.0)
        .border(1.0, white(0.08))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Commit your changes")
        .child(header)
        .child(body);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    backdrop(s, 0.5).center().child(panel)
}

pub(in crate::screen) fn output(s: &Screen<'_>) -> El {
    let logs = &s.st.tasks.output;
    let lines = logs.split('\n').count();
    let copied =
        s.st.tasks
            .copied
            .is_some_and(|t| t.elapsed().as_secs_f32() < 2.0);
    let cid = s.acts.on("output-copy", Act::Task(TaskAct::CopyOutput));
    let chot = s.ui.hover(cid);
    let ccol = c(look::GRAY_400).mix(c(look::GRAY_200), chot);
    let (icon, label) = if copied {
        ("circle-check", "Copied")
    } else {
        ("copy", "Copy")
    };
    let header = div()
        .row()
        .items_center()
        .justify_between()
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.08))
        .child(
            div()
                .row()
                .items_center()
                .gap(12.0)
                .child(
                    text("Full Output", 13.0, c(look::WHITE))
                        .weight(500)
                        .line_height(18.0)
                        .role(Role::Heading, "Full Output"),
                )
                .child(
                    text(format!("{lines} lines"), 11.0, c(look::GRAY_600))
                        .line_height(14.0)
                        .silent(),
                ),
        )
        .child(
            div()
                .row()
                .items_center()
                .gap(8.0)
                .child(
                    div()
                        .id(cid)
                        .on(Action::Click)
                        .row()
                        .items_center()
                        .gap(6.0)
                        .px(10.0)
                        .py(4.0)
                        .rounded(6.0)
                        .bg(white(0.04 + 0.04 * chot))
                        .border(1.0, white(0.06))
                        .role(Role::Button, label)
                        .child(ico(icon, 12.0, 2.0, ccol))
                        .child(
                            text(label, 11.0, ccol)
                                .weight(500)
                                .line_height(14.0)
                                .silent(),
                        ),
                )
                .child(head_btn(
                    s,
                    "output",
                    Act::CloseDialog,
                    "x",
                    14.0,
                    2.0,
                    "Close",
                    c(look::GRAY_500),
                )),
        );
    let w = (s.size.0 * 0.9).min(900.0);
    let h = s.size.1 * 0.8;
    let pre = div()
        .id(Id::new("output-log"))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .p(16.0)
        .child(
            text(logs, 12.0, c(look::GRAY_300))
                .mono()
                .line_height(19.5)
                .wrapping(),
        );
    let panel = div()
        .col()
        .w(w)
        .h(h)
        .rounded(12.0)
        .border(1.0, white(0.10))
        .bg(c(look::SURFACE_BASE))
        .clip()
        .role(Role::Dialog, "Full Output")
        .child(header)
        .child(pre);
    let panel = s.acts.click(panel, "dialog", Act::Nothing);
    backdrop(s, 0.6).center().child(panel)
}
