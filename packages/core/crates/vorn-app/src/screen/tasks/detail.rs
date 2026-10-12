//! `TaskDetailPanel`: the selected task's properties, description, images,
//! session buttons, workflow runs and changes, or the create form.

use vorn_protocol::TaskConfig;

use super::diff;
use super::editor::editor;
use super::{
    agent_picker, branch_id, can_resume, confirm_delete, faint, image_id, live, primary,
    project_picker, secondary, status_picker, title_id,
};
use crate::act::Act;
use crate::look;
use crate::screen::runs::{run_entry, Controls};
use crate::screen::{c, ico, white, Screen};
use crate::tasks::{self, Form, TaskAct};
use crate::ui::{div, field, image, text, tooltip, Action, El, Id, Rgba, Role, TextStyle};

pub const WIDTH: f32 = 420.0;

/// A header icon button: faint, brightening on hover.
fn head_btn(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    icon: &str,
    size: f32,
    label: &str,
    danger: bool,
) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    let to = if danger { c(look::DANGER) } else { primary() };
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, label)
        .child(ico(icon, size, 1.5, faint().mix(to, hot)));
    tooltip(s.ui, b, id, label, None)
}

fn header(s: &Screen<'_>, task: Option<&TaskConfig>) -> El {
    let mut row = div().row().w_full().items_center().gap(8.0);
    row = match task {
        None => row.child(
            div().grow().child(
                text("New Task", 14.0, primary())
                    .weight(500)
                    .line_height(20.0)
                    .silent(),
            ),
        ),
        Some(_) => row.child(div().grow()),
    };
    if let Some(t) = task {
        let status = t.status.to_string();
        if t.archived_at.is_some() {
            row = row.child(head_btn(
                s,
                "task-panel",
                Act::Task(TaskAct::Unarchive(t.id.clone())),
                "archive-restore",
                13.0,
                "Unarchive task",
                false,
            ));
        } else if tasks::is_terminal(&status) {
            row = row.child(head_btn(
                s,
                "task-panel",
                Act::Task(TaskAct::Archive(t.id.clone())),
                "archive",
                13.0,
                "Archive task",
                false,
            ));
        }
        let del = head_btn(
            s,
            "task-panel",
            Act::Task(TaskAct::AskDelete(t.id.clone())),
            "trash-2",
            13.0,
            "Delete task",
            true,
        );
        row = row.child(confirm_delete(s, del, &t.id));
    }
    row = row.child(head_btn(
        s,
        "task-panel",
        Act::Task(TaskAct::CloseDetail),
        "x",
        14.0,
        "Close",
        false,
    ));
    div()
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.06))
        .child(row)
}

/// One property row: the `w-20` label and its control.
fn prop(label: &str, value: El) -> El {
    div()
        .row()
        .items_center()
        .gap(8.0)
        .child(
            div()
                .w(80.0)
                .shrink0()
                .child(text(label, 12.0, faint()).line_height(16.0).silent()),
        )
        .child(value)
}

fn properties(s: &Screen<'_>, task: Option<&TaskConfig>) -> El {
    let form = &s.st.tasks.detail_form;
    let worktree = {
        let on = form.worktree;
        let id = s.acts.on(
            "task-worktree",
            Act::Task(TaskAct::ToggleWorktree(Form::Detail)),
        );
        let hot = s.ui.hover(id);
        let color = if on { secondary() } else { faint() };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(6.0)
            .py(2.0)
            .ml(-6.0)
            .rounded(4.0)
            .bg(white(0.04 * hot))
            .role(Role::Switch, "Worktree")
            .toggled(on)
            .child(ico("folder-git-2", 13.0, 1.5, color))
            .child(
                text(if on { "Enabled" } else { "Disabled" }, 12.0, color)
                    .line_height(16.0)
                    .silent(),
            )
    };
    let branch = div()
        .row()
        .grow()
        .min_w0()
        .items_center()
        .gap(6.0)
        .child(ico("git-branch", 11.0, 2.0, faint()))
        .child(
            div().grow().min_w0().py(2.0).child(
                field(
                    branch_id(),
                    "Branch",
                    "feature/my-task",
                    TextStyle::ui(12.0),
                    secondary(),
                )
                .w_full(),
            ),
        );
    let mut col = div()
        .col()
        .gap(10.0)
        .px(16.0)
        .py(12.0)
        .border_bottom(1.0, white(0.06))
        .child(prop(
            "Status",
            status_picker(s, Form::Detail, task.is_none()),
        ))
        .child(prop("Project", project_picker(s, Form::Detail)))
        .child(prop("Branch", branch))
        .child(prop("Worktree", worktree))
        .child(prop("Agent", agent_picker(s, Form::Detail)));
    let date = |icon: &str, iso: &str| {
        div()
            .row()
            .items_center()
            .gap(4.0)
            .child(ico(icon, 11.0, 2.0, secondary()))
            .child(
                text(tasks::long_date(iso), 12.0, secondary())
                    .line_height(16.0)
                    .silent(),
            )
    };
    if let Some(t) = task {
        col = col.child(prop("Created", date("calendar", &t.created_at)));
        if let Some(done) = &t.completed_at {
            col = col.child(prop("Completed", date("clock", done)));
        }
    }
    col
}

fn source_chip(s: &Screen<'_>, t: &TaskConfig) -> Option<El> {
    let conn = t.source_connector_id.as_deref()?;
    let url = t.source_external_url.as_deref()?;
    let id = s.acts.on("task-source", Act::OpenUrl(url.to_owned()));
    let hot = s.ui.hover(id);
    let label = t
        .source_external_id
        .as_deref()
        .map_or_else(|| conn.to_owned(), |x| format!("#{x}"));
    Some(
        div().px(16.0).pb(8.0).row().child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(6.0)
                .px(8.0)
                .py(4.0)
                .rounded(6.0)
                .bg(white(0.04 + 0.04 * hot))
                .border(1.0, white(0.08))
                .role(Role::Link, &label)
                .child(ico("link", 13.0, 2.0, secondary()))
                .child(
                    text(&label, 12.0, secondary().mix(c(look::WHITE), hot))
                        .line_height(16.0)
                        .silent(),
                )
                .child(text("\u{2197}", 12.0, faint()).line_height(16.0).silent()),
        ),
    )
}

/// The thumbnails, each with its remove button, and the add tile.
pub(super) fn images(s: &Screen<'_>, form: Form, size: f32) -> El {
    let f = s.st.tasks.form_ref(form);
    let mut wrap = div().row().wrap_children().gap(8.0);
    for name in &f.images {
        let tile = Id::new("task-image").child((form, name.as_str()));
        let hot = s.ui.hovered(tile);
        let mut t = div()
            .id(tile)
            .sense(crate::ui::Sense::HOVER)
            .size(size, size)
            .rounded(8.0)
            .border(1.0, white(0.08))
            .bg(white(0.03))
            .clip()
            .role(Role::Image, name);
        if let Some(&img) = s.st.tasks.thumbs.get(name) {
            t = t.child(image(img, size - 2.0, size - 2.0));
        }
        if hot {
            let id = s.acts.on(
                "task-image-remove",
                Act::Task(TaskAct::RemoveImage(form, name.clone())),
            );
            let xhot = s.ui.hover(id);
            t = t.child(
                div()
                    .id(id)
                    .on(Action::Click)
                    .absolute_right(2.0, 2.0)
                    .size(16.0, 16.0)
                    .rounded(8.0)
                    .bg(Rgba::hexa(0x000000, 0.70))
                    .center()
                    .role(Role::Button, "Remove image")
                    .child(ico(
                        "x",
                        10.0,
                        3.0,
                        c(look::WHITE).mix(c(look::DANGER), xhot),
                    )),
            );
        }
        wrap = wrap.child(t);
    }
    let id = s
        .acts
        .on("task-image-add", Act::Task(TaskAct::AttachImages(form)));
    let hot = s.ui.hover(id);
    let add = div()
        .id(id)
        .on(Action::Click)
        .size(size, size)
        .rounded(8.0)
        .border(1.0, white(0.10 + 0.10 * hot))
        .dashed()
        .center()
        .role(Role::Button, "Add images")
        .child(ico(
            "image-plus",
            if size < 60.0 { 16.0 } else { 18.0 },
            1.5,
            faint().mix(secondary(), hot),
        ));
    wrap = wrap.child(tooltip(s.ui, add, id, "Add images", None));
    let mut col = div().col().gap(8.0).child(wrap);
    if f.attaching {
        col = col.child(
            div()
                .px(10.0)
                .py(6.0)
                .rounded(6.0)
                .bg(white(0.03))
                .border(1.0, white(0.10))
                .child(
                    field(
                        image_id(form),
                        "Image path",
                        "Image path, then Enter",
                        TextStyle::ui(12.0),
                        secondary(),
                    )
                    .w_full(),
                ),
        );
    }
    col
}

fn session_buttons(s: &Screen<'_>, t: &TaskConfig) -> Option<El> {
    let live = live(s, t);
    let resume = !live && can_resume(t);
    if !live && !resume {
        return None;
    }
    let btn = |icon: &str, label: &str| {
        let id = s.acts.on(
            "task-panel-session",
            Act::Task(TaskAct::OpenSession(t.id.clone())),
        );
        let hot = s.ui.hover(id);
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(6.0)
            .rounded(6.0)
            .bg(white(0.04 + 0.04 * hot))
            .border(1.0, white(0.08))
            .role(Role::Button, label)
            .child(ico(icon, 12.0, 2.0, secondary().mix(primary(), hot)))
            .child(
                text(label, 12.0, secondary().mix(primary(), hot))
                    .weight(500)
                    .line_height(16.0)
                    .silent(),
            )
    };
    let b = if live {
        btn("terminal", "Focus Session")
    } else {
        btn("play", "Resume Session")
    };
    Some(
        div()
            .row()
            .px(16.0)
            .py(8.0)
            .border_top(1.0, white(0.06))
            .child(b),
    )
}

/// A section toggle: `px-4 py-2.5`, 11 px uppercase, faint.
fn toggle(s: &Screen<'_>, act: TaskAct, open: bool, icon: &str, label: &str) -> (Id, El) {
    let id = s.acts.on("task-section", Act::Task(act));
    let hot = s.ui.hover(id);
    let color = faint().mix(secondary(), hot);
    (
        id,
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(8.0)
            .px(16.0)
            .py(10.0)
            .role(Role::Button, label)
            .expanded(open)
            .child(ico(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                11.0,
                2.0,
                color,
            ))
            .child(ico(icon, 12.0, 2.0, color))
            .child(
                text(label.to_uppercase(), 11.0, color)
                    .weight(500)
                    .line_height(14.0)
                    .silent(),
            ),
    )
}

fn workflow_runs(s: &Screen<'_>) -> Option<El> {
    let runs = &s.st.tasks.runs;
    if runs.is_empty() {
        return None;
    }
    let open = s.st.tasks.runs_open;
    let (_, head) = toggle(
        s,
        TaskAct::ToggleRuns,
        open,
        "workflow",
        &format!("Workflow Runs ({})", runs.len()),
    );
    let mut col = div().col().border_top(1.0, white(0.06)).child(head);
    if open {
        let mut list = div().col().gap(8.0).px(12.0).pb(12.0);
        for run in runs {
            let wid = run.get("workflowId").and_then(|v| v.as_str()).unwrap_or("");
            let wf = s.store.workflows.iter().find(|w| w.id == wid);
            let nodes = wf.map_or(serde_json::Value::Null, |w| {
                serde_json::to_value(&w.nodes).unwrap_or_default()
            });
            let name = run
                .get("workflowName")
                .and_then(|v| v.as_str())
                .or(wf.map(|w| w.name.as_str()));
            list = list.child(run_entry(s, run, &nodes, name, Controls::default()));
        }
        col = col.child(list);
    }
    Some(col)
}

fn changes(s: &Screen<'_>, t: &TaskConfig) -> Option<El> {
    if !matches!(t.status.to_string().as_str(), "in_review" | "in_progress") {
        return None;
    }
    let d = &s.st.tasks.diff;
    let open = s.st.tasks.diff_open;
    let (ins, del) = d
        .files
        .iter()
        .fold((0, 0), |(i, x), f| (i + f.insertions, x + f.deletions));
    let has_stat = !d.files.is_empty() || (!d.loading && !d.cwd.is_empty());
    let (_, mut head) = toggle(s, TaskAct::ToggleDiff, open, "file-code", "Changes");
    if has_stat {
        head = head.child(
            div()
                .row()
                .gap(6.0)
                .child(
                    text(format!("+{ins}"), 11.0, secondary())
                        .mono()
                        .line_height(14.0)
                        .silent(),
                )
                .child(
                    text(format!("-{del}"), 11.0, faint())
                        .mono()
                        .line_height(14.0)
                        .silent(),
                ),
        );
    }
    head = head.child(div().grow());
    if ins > 0 || del > 0 {
        let id = s
            .acts
            .on("task-commit-open", Act::Task(TaskAct::OpenCommit));
        let hot = s.ui.hover(id);
        head = head.child(
            div()
                .id(id)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(4.0)
                .px(8.0)
                .py(2.0)
                .rounded(4.0)
                .bg(white(0.04 + 0.04 * hot))
                .border(1.0, white(0.06))
                .role(Role::Button, "Commit")
                .child(ico(
                    "git-commit-horizontal",
                    11.0,
                    1.5,
                    secondary().mix(primary(), hot),
                ))
                .child(
                    text("Commit", 10.0, secondary().mix(primary(), hot))
                        .weight(500)
                        .line_height(14.0)
                        .silent(),
                ),
        );
    }
    let rid = s
        .acts
        .on("task-diff-refresh", Act::Task(TaskAct::RefreshDiff));
    let rhot = s.ui.hover(rid);
    head = head.child(
        div()
            .id(rid)
            .on(Action::Click)
            .p(2.0)
            .rounded(4.0)
            .role(Role::Button, "Refresh")
            .child(ico(
                "refresh-cw",
                12.0,
                1.5,
                faint().mix(c(look::WHITE), rhot),
            )),
    );
    let mut col = div().col().border_top(1.0, white(0.06)).child(head);
    if open {
        if !d.comments.is_empty() {
            col = col.child(diff::feedback_bar(s, t.agent_session_id.is_some()));
        }
        col = if d.loading && d.files.is_empty() {
            col.child(div().row().justify_center().py(32.0).child(ico(
                "loader-circle",
                18.0,
                2.0,
                faint(),
            )))
        } else if !d.files.is_empty() {
            col.child(diff::file_list(s, &d.files))
                .child(diff::content(s, &d.files))
        } else {
            let msg = if d.cwd.is_empty() {
                "No project path"
            } else {
                "No uncommitted changes"
            };
            col.child(
                div()
                    .row()
                    .justify_center()
                    .py(24.0)
                    .child(text(msg, 12.0, faint()).line_height(16.0).silent()),
            )
        };
    }
    Some(col)
}

fn footer(s: &Screen<'_>) -> El {
    let f = &s.st.tasks.detail_form;
    let ok = !s.ui.edit_text(title_id(Form::Detail)).trim().is_empty()
        && !f.project.is_empty()
        && !s
            .ui
            .edit_text(super::desc_id(Form::Detail))
            .trim()
            .is_empty();
    let cancel = s
        .acts
        .on("task-panel-cancel", Act::Task(TaskAct::CloseDetail));
    let chot = s.ui.hover(cancel);
    let create = s.acts.on("task-panel-create", Act::Task(TaskAct::Create));
    let hot = if ok { s.ui.hover(create) } else { 0.0 };
    let mut create_btn = div()
        .id(create)
        .row()
        .items_center()
        .gap(6.0)
        .px(12.0)
        .py(6.0)
        .rounded(8.0)
        .bg(white(0.10 + 0.05 * hot))
        .opacity(if ok { 1.0 } else { 0.3 })
        .role(Role::Button, "Create Task")
        .disabled(!ok)
        .child(ico("save", 13.0, 2.0, c(look::WHITE)))
        .child(
            text("Create Task", 14.0, c(look::WHITE))
                .weight(500)
                .line_height(20.0)
                .silent(),
        );
    if ok {
        create_btn = create_btn.on(Action::Click);
    }
    div()
        .row()
        .justify_end()
        .gap(8.0)
        .px(16.0)
        .py(12.0)
        .border_top(1.0, white(0.06))
        .child(
            div()
                .id(cancel)
                .on(Action::Click)
                .px(12.0)
                .py(6.0)
                .rounded(8.0)
                .bg(white(0.04 + 0.04 * chot))
                .role(Role::Button, "Cancel")
                .child(
                    text("Cancel", 14.0, secondary().mix(primary(), chot))
                        .line_height(20.0)
                        .silent(),
                ),
        )
        .child(create_btn)
}

/// The panel, when a task (or the create form) is selected.
pub(super) fn panel(s: &Screen<'_>) -> Option<El> {
    let sel = s.st.tasks.selected.as_deref()?;
    let task = s.store.tasks.iter().find(|t| t.id == sel);
    if task.is_none() && sel != "new" {
        return None;
    }
    let title_focus = s.ui.focused(title_id(Form::Detail));
    let title = div().px(16.0).pt(16.0).pb(8.0).child(
        div()
            .w_full()
            .px(4.0)
            .ml(-4.0)
            .py(2.0)
            .rounded(4.0)
            .bg(white(if title_focus { 0.02 } else { 0.0 }))
            .child(
                field(
                    title_id(Form::Detail),
                    "Task title",
                    "Task title",
                    TextStyle {
                        weight: 600,
                        ..TextStyle::ui(16.0)
                    },
                    primary(),
                )
                .w_full(),
            ),
    );
    let cols = ((WIDTH - 64.0) / 7.6) as usize;
    let mut scroll = div()
        .id(Id::new("task-panel-scroll"))
        .col()
        .grow()
        .min_h(0.0)
        .scroll_y()
        .child(properties(s, task))
        .child(title);
    if let Some(t) = task {
        scroll = scroll.child_if(source_chip(s, t));
    }
    scroll = scroll
        .child(div().px(16.0).pb(12.0).child(editor(
            s,
            Form::Detail,
            "Describe the task in detail, or type / for commands...",
            120.0,
            cols,
        )))
        .child(div().px(16.0).pb(12.0).child(images(s, Form::Detail, 64.0)));
    if let Some(t) = task {
        scroll = scroll
            .child_if(session_buttons(s, t))
            .child_if(workflow_runs(s))
            .child_if(changes(s, t));
    }
    let mut col = div()
        .id(Id::new("task-panel"))
        .col()
        .w(WIDTH)
        .shrink0()
        .h_full()
        .bg(c(look::SURFACE_PANEL))
        .border_left(1.0, white(0.08))
        .role(Role::Region, "Task details")
        .child(header(s, task))
        .child(scroll);
    if task.is_none() {
        col = col.child(footer(s));
    }
    Some(col)
}
