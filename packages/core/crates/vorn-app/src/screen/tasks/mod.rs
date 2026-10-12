//! The Tasks view (`TaskBoardView`): the list and the board of the
//! project's tasks, the detail panel beside them, the toolbar's popover and
//! the pickers the forms open.

mod detail;
mod dialog;
mod diff;
mod editor;

pub(super) use dialog::{add_task, commit, output};
pub(super) use editor::overlay;
pub use editor::{form_of, slash_open};

use vorn_protocol::TaskConfig;

use super::{c, ico, white, Screen};
use crate::act::Act;
use crate::look::{self, Rgb};
use crate::screen::menus::{Item, AGENTS};
use crate::state::MenuKind;
use crate::tasks::{self, Form, Mode, Source, TaskAct, STATUSES};
use crate::ui::{div, text, tooltip, Action, El, Id, Place, Rgba, Role, Sense};

pub fn title_id(form: Form) -> Id {
    Id::new("task-title").child(form)
}

pub fn desc_id(form: Form) -> Id {
    Id::new("task-desc").child(form)
}

/// The image path field each form shows to attach an image.
pub fn image_id(form: Form) -> Id {
    Id::new("task-image").child(form)
}

pub fn branch_id() -> Id {
    Id::new("task-branch")
}

pub fn link_id() -> Id {
    Id::new("task-link-url")
}

pub fn comment_id() -> Id {
    Id::new("task-comment")
}

pub fn commit_id() -> Id {
    Id::new("task-commit-message")
}

/// The form whose description field is `id`.
pub fn desc_form(id: Id) -> Option<Form> {
    [Form::Dialog, Form::Detail]
        .into_iter()
        .find(|&f| desc_id(f) == id)
}

pub fn column_id(status: &str) -> Id {
    Id::new("task-column").child(status)
}

pub fn card_id(task: &str) -> Id {
    Id::new("task-card").child(task)
}

pub(super) fn ink(a: f32) -> Rgba {
    Rgba::hexa(look::INK, a)
}

pub(super) fn primary() -> Rgba {
    ink(1.0)
}

pub(super) fn secondary() -> Rgba {
    ink(look::INK_SECONDARY)
}

pub(super) fn faint() -> Rgba {
    ink(look::INK_FAINT)
}

/// `TASK_STATUS_TEXT`.
pub(super) fn tone(status: &str) -> Rgba {
    tasks::status(status).color.map_or_else(faint, c)
}

/// `TASK_STATUS_DOT`: the status filter's dots.
fn dot(status: &str) -> Rgb {
    tasks::status(status).color.unwrap_or(0x5a5a5e)
}

/// The task's assigned session is open.
pub(super) fn live(s: &Screen<'_>, t: &TaskConfig) -> bool {
    t.assigned_session_id.as_deref().is_some_and(|id| {
        s.store
            .sessions
            .iter()
            .any(|x| x.info.id == id && x.exited.is_none())
    })
}

/// `getOpenSessionHandler`: a live session to focus or an agent session to
/// resume.
pub(super) fn can_open(s: &Screen<'_>, t: &TaskConfig) -> bool {
    live(s, t) || can_resume(t)
}

pub(super) fn can_resume(t: &TaskConfig) -> bool {
    t.agent_session_id.is_some()
        && t.assigned_agent
            .as_ref()
            .is_some_and(|a| tasks::resumable(&a.to_string()))
        && matches!(
            t.status.to_string().as_str(),
            "in_progress" | "in_review" | "done"
        )
}

/// A `p-1` icon button in faint ink that brightens on hover.
pub(super) fn icon_btn(
    s: &Screen<'_>,
    place: &str,
    act: Act,
    icon: &str,
    size: f32,
    label: &str,
) -> El {
    let id = s.acts.on(place, act);
    let hot = s.ui.hover(id);
    let b = div()
        .id(id)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, label)
        .child(ico(icon, size, 2.0, faint().mix(primary(), hot)));
    tooltip(s.ui, b, id, label, None)
}

/// `ConfirmPopover` under `anchor`: the question, Cancel and Delete.
pub(super) fn confirm_delete(s: &Screen<'_>, anchor: El, task: &str) -> El {
    if s.st.tasks.confirm_delete.as_deref() != Some(task) {
        return anchor;
    }
    let btn = |place: &str, act: Act, label: &str, danger: bool| {
        let id = s.acts.on(place, act);
        let hot = s.ui.hover(id);
        let (fg, bg) = if danger {
            (c(look::DANGER), Rgba::hexa(look::DANGER, 0.10 + 0.10 * hot))
        } else {
            (secondary().mix(primary(), hot), white(0.04 + 0.04 * hot))
        };
        let mut b = div()
            .id(id)
            .on(Action::Click)
            .px(8.0)
            .py(4.0)
            .rounded(6.0)
            .bg(bg)
            .role(Role::Button, label)
            .child(
                text(label, 11.0, fg)
                    .weight(if danger { 500 } else { 400 })
                    .line_height(14.0)
                    .silent(),
            );
        if danger {
            b = b.border(1.0, Rgba::hexa(look::DANGER, 0.20));
        }
        b
    };
    let panel = div()
        .col()
        .w(200.0)
        .px(12.0)
        .py(8.0)
        .rounded(8.0)
        .border(1.0, white(0.10))
        .bg(c(look::SURFACE_OVERLAY))
        .role(Role::Dialog, "Delete this task permanently?")
        .child(
            div().pb(10.0).child(
                text("Delete this task permanently?", 12.0, secondary())
                    .line_height(16.0)
                    .wrapping(),
            ),
        )
        .child(
            div()
                .row()
                .justify_end()
                .gap(8.0)
                .child(btn(
                    "task-delete-cancel",
                    Act::Task(TaskAct::AskDelete(String::new())),
                    "Cancel",
                    false,
                ))
                .child(btn(
                    "task-delete-confirm",
                    Act::Task(TaskAct::Delete(task.to_owned())),
                    "Delete",
                    true,
                )),
        );
    anchor.overlay(panel, Place::Below, 4.0)
}

/// A list row (`TaskCard`): agent, id, status, title, date and the hover
/// actions.
fn list_row(s: &Screen<'_>, t: &TaskConfig) -> El {
    let id = card_id(&t.id);
    s.acts.bind(id, Act::Task(TaskAct::Open(t.id.clone())));
    let status = t.status.to_string();
    let st = tasks::status(&status);
    let archived = t.archived_at.is_some();
    let dimmed = status == "cancelled" || archived;
    let hot = s.ui.hovered(id) || s.st.tasks.confirm_delete.as_deref() == Some(t.id.as_str());
    let selected = s.st.tasks.selected.as_deref() == Some(t.id.as_str());
    let bg = if hot || selected {
        white(0.04)
    } else {
        white(0.0)
    };
    let agent = match &t.assigned_agent {
        Some(_) => ico("bot", 14.0, 1.5, secondary()),
        None => text("---", 10.0, faint()).line_height(14.0).silent(),
    };
    let mut short = div().row().items_center().gap(6.0).shrink0().child(
        text(tasks::short_id(&t.project_name, &t.id), 11.0, faint())
            .weight(500)
            .line_height(16.0)
            .silent(),
    );
    if let Some(ext) = t
        .source_external_id
        .as_deref()
        .filter(|_| t.source_connector_id.is_some())
    {
        short = short.child(source_badge(ext));
    }
    if archived {
        short = short.child(ico("archive", 10.0, 2.0, faint()));
    }
    let title_color = if status == "cancelled" {
        faint()
    } else {
        primary()
    };
    let mut row = div()
        .id(id)
        .on(Action::Click)
        .row()
        .items_center()
        .gap(12.0)
        .px(12.0)
        .py(8.0)
        .rounded(6.0)
        .bg(bg)
        .role(Role::Button, &t.title)
        .selected(selected)
        .child(div().w(16.0).shrink0().center().child(agent))
        .child(short)
        .child(ico(st.icon, 14.0, 2.0, tone(&status)).shrink0())
        .child(
            div().grow().min_w0().child(
                text(&t.title, 14.0, title_color)
                    .line_height(20.0)
                    .truncate()
                    .silent(),
            ),
        )
        .child(
            text(tasks::short_date(&t.created_at), 11.0, faint())
                .line_height(16.0)
                .silent(),
        );
    let key = &t.id;
    let place = |p: &str| format!("task-row-{p}");
    let mut actions = div().row().items_center().gap(2.0).shrink0();
    if status == "in_review" {
        actions = actions
            .child(icon_btn(
                s,
                &place("diff"),
                Act::Task(TaskAct::ReviewDiff(key.clone())),
                "file-code",
                12.0,
                "Review diff",
            ))
            .child(icon_btn(
                s,
                &place("done"),
                Act::Task(TaskAct::Complete(key.clone())),
                "circle-check",
                12.0,
                "Mark as done",
            ));
    }
    if status == "cancelled" {
        actions = actions.child(icon_btn(
            s,
            &place("reopen"),
            Act::Task(TaskAct::Reopen(key.clone())),
            "rotate-ccw",
            12.0,
            "Reopen task",
        ));
    }
    if can_open(s, t) {
        let l = live(s, t);
        actions = actions.child(icon_btn(
            s,
            &place("session"),
            Act::Task(TaskAct::OpenSession(key.clone())),
            if l { "terminal" } else { "play" },
            12.0,
            if l { "Focus session" } else { "Resume session" },
        ));
    }
    actions = actions.child(icon_btn(
        s,
        &place("edit"),
        Act::Task(TaskAct::Edit(key.clone())),
        "pencil",
        12.0,
        "Edit task",
    ));
    if !tasks::is_terminal(&status) {
        actions = actions.child(icon_btn(
            s,
            &place("cancel"),
            Act::Task(TaskAct::Cancel(key.clone())),
            "circle-x",
            12.0,
            "Cancel task",
        ));
    }
    if !archived && tasks::is_terminal(&status) {
        actions = actions.child(icon_btn(
            s,
            &place("archive"),
            Act::Task(TaskAct::Archive(key.clone())),
            "archive",
            12.0,
            "Archive task",
        ));
    }
    if archived {
        actions = actions.child(icon_btn(
            s,
            &place("unarchive"),
            Act::Task(TaskAct::Unarchive(key.clone())),
            "archive-restore",
            12.0,
            "Unarchive task",
        ));
    }
    let del = icon_btn(
        s,
        &place("delete"),
        Act::Task(TaskAct::AskDelete(key.clone())),
        "trash-2",
        12.0,
        "Delete task",
    );
    actions = actions.child(confirm_delete(s, del, key));
    row = row.child(actions.opacity(if hot { 1.0 } else { 0.0 }));
    if dimmed {
        row = row.opacity(0.6);
    }
    row
}

/// `SourceBadge`: the external id the task came from.
fn source_badge(ext: &str) -> El {
    div()
        .row()
        .items_center()
        .gap(2.0)
        .child(ico("link", 10.0, 2.0, faint()))
        .child(
            text(format!("#{ext}"), 10.0, faint())
                .line_height(14.0)
                .silent(),
        )
}

/// A section's header bar or a column's header: chevron (list only),
/// status icon, label, count and add.
fn header(s: &Screen<'_>, status: &'static tasks::Status, count: usize, list: bool) -> El {
    let add = s.acts.on(
        "task-section-add",
        Act::Task(TaskAct::New(status.key.to_owned())),
    );
    let add_hot = s.ui.hover(add);
    let toggle = list.then(|| {
        s.acts.on(
            "task-section",
            Act::Task(TaskAct::ToggleSection(status.key.to_owned())),
        )
    });
    let col_hot = !list && s.ui.hovered(column_id(status.key));
    let mut h = div()
        .row()
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(if list { 8.0 } else { 12.0 });
    if let Some(id) = toggle {
        let folded = s.st.tasks.collapsed.contains(status.key);
        h = h
            .id(id)
            .on(Action::Click)
            .rounded(8.0)
            .bg(white(0.03).mix(white(0.06), s.ui.hover(id)))
            .role(Role::Button, status.label)
            .expanded(!folded)
            .child(ico(
                if folded {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                14.0,
                2.0,
                secondary(),
            ));
    }
    let add_btn = div()
        .id(add)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, format!("Add task to {}", status.label))
        .opacity(if list || col_hot || add_hot > 0.0 {
            1.0
        } else {
            0.0
        })
        .child(ico("plus", 14.0, 2.0, faint().mix(secondary(), add_hot)));
    h.child(ico(status.icon, 14.0, 2.0, tone(status.key)))
        .child(
            text(status.label, 13.0, secondary())
                .weight(500)
                .line_height(18.0)
                .silent(),
        )
        .child(
            div().pl(if list { 0.0 } else { 2.0 }).child(
                text(count.to_string(), 11.0, faint())
                    .line_height(16.0)
                    .silent(),
            ),
        )
        .child(div().grow())
        .child(tooltip(s.ui, add_btn, add, "Add task", None))
}

fn list(s: &Screen<'_>, shown: &[&TaskConfig]) -> El {
    let mut col = div().col().gap(4.0).w_full();
    for st in &STATUSES {
        let in_status = tasks::in_status(shown, st.key);
        let mut section = div().col().child(header(s, st, in_status.len(), true));
        if !s.st.tasks.collapsed.contains(st.key) {
            let mut body = div().col().py(4.0);
            if in_status.is_empty() {
                body = body.child(
                    div()
                        .py(8.0)
                        .pl(40.0)
                        .child(text(st.empty, 12.0, faint()).line_height(16.0).silent()),
                );
            }
            for t in in_status {
                body = body.child(list_row(s, t));
            }
            section = section.child(body);
        }
        col = col.child(section);
    }
    col
}

/// A board card (`TaskCard` kanban): id, menu, status and title, agent and
/// date.
fn kanban_card(s: &Screen<'_>, t: &TaskConfig, ghost: bool) -> El {
    let id = card_id(&t.id);
    let status = t.status.to_string();
    let archived = t.archived_at.is_some();
    let menu_kind = MenuKind::TaskCard(t.id.clone());
    let menu_open = s.st.menu.as_ref().is_some_and(|m| m.kind == menu_kind);
    let hot = !ghost && (s.ui.hovered(id) || menu_open);
    let mut top = div().row().items_center().gap(6.0).child(
        text(tasks::short_id(&t.project_name, &t.id), 10.0, faint())
            .weight(500)
            .line_height(14.0)
            .silent(),
    );
    if let Some(ext) = t
        .source_external_id
        .as_deref()
        .filter(|_| t.source_connector_id.is_some())
    {
        top = top
            .child(text("\u{b7}", 10.0, faint()).line_height(14.0).silent())
            .child(source_badge(ext));
    }
    if archived {
        top = top.child(ico("archive", 10.0, 2.0, faint()));
    }
    let more = s.acts.on("task-card-menu", Act::Open(menu_kind));
    let more_btn = div()
        .id(more)
        .on(Action::Click)
        .p(4.0)
        .rounded(4.0)
        .role(Role::Button, "More actions")
        .has_popup()
        .opacity(if hot { 1.0 } else { 0.0 })
        .child(ico(
            "ellipsis",
            14.0,
            2.0,
            faint().mix(secondary(), s.ui.hover(more)),
        ));
    let title_color = if status == "cancelled" {
        faint()
    } else {
        primary()
    };
    let mut foot = div().row().items_center().gap(6.0);
    if t.assigned_agent.is_some() {
        foot = foot.child(ico("bot", 13.0, 1.5, secondary()));
        if live(s, t) {
            foot = foot.child(div().size(6.0, 6.0).rounded(3.0).bg(c(tasks::SAGE)));
        }
    }
    let mut card = div()
        .col()
        .px(14.0)
        .py(12.0)
        .rounded(6.0)
        .bg(c(look::SURFACE_RAISED))
        .border(1.0, if hot { white(0.16) } else { white(0.08) })
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .pb(6.0)
                .child(top)
                .child(tooltip(s.ui, more_btn, more, "More actions", None)),
        )
        .child(
            div()
                .row()
                .gap(6.0)
                .child(div().pt(2.0).child(ico(
                    tasks::status(&status).icon,
                    14.0,
                    2.0,
                    tone(&status),
                )))
                .child(
                    div().grow().min_w0().child(
                        text(&t.title, 13.0, title_color)
                            .weight(500)
                            .line_height(18.0)
                            .wrapping()
                            .silent(),
                    ),
                ),
        )
        .child(
            div()
                .row()
                .items_center()
                .justify_between()
                .pt(10.0)
                .child(foot)
                .child(
                    text(
                        format!("Created {}", tasks::short_date(&t.created_at)),
                        11.0,
                        faint(),
                    )
                    .line_height(16.0)
                    .silent(),
                ),
        );
    if status == "cancelled" || archived {
        card = card.opacity(0.6);
    }
    if ghost {
        return card;
    }
    s.acts.bind(id, Act::Task(TaskAct::Open(t.id.clone())));
    let dragging =
        s.st.tasks
            .dragging
            .as_ref()
            .is_some_and(|(d, _)| d == &t.id);
    card.id(id)
        .sense(Sense::CLICK | Sense::MOVE | Sense::HOVER)
        .on(Action::Click)
        .role(Role::Button, &t.title)
        .opacity(if dragging {
            0.4
        } else if status == "cancelled" || archived {
            0.6
        } else {
            1.0
        })
}

fn kanban(s: &Screen<'_>, shown: &[&TaskConfig]) -> El {
    let mut row = div().row().gap(8.0).grow().min_h(0.0).w_full();
    for st in &STATUSES {
        let in_status = tasks::in_status(shown, st.key);
        let over =
            s.st.tasks
                .dragging
                .as_ref()
                .is_some_and(|(_, col)| col.as_deref() == Some(st.key));
        let mut cards = div()
            .id(Id::new("task-column-scroll").child(st.key))
            .col()
            .gap(8.0)
            .grow()
            .min_h(0.0)
            .scroll_y()
            .px(8.0)
            .pb(8.0);
        if in_status.is_empty() {
            cards = cards.child(
                div().min_h(80.0).center().child(
                    div()
                        .w_full()
                        .px(16.0)
                        .py(20.0)
                        .rounded(8.0)
                        .border(1.0, white(0.08))
                        .dashed()
                        .center()
                        .child(
                            text("Drop tasks here", 12.0, faint())
                                .line_height(16.0)
                                .silent(),
                        ),
                ),
            );
        }
        for t in &in_status {
            cards = cards.child(kanban_card(s, t, false));
        }
        let add = s.acts.on(
            "task-column-add",
            Act::Task(TaskAct::New(st.key.to_owned())),
        );
        let add_hot = s.ui.hover(add);
        let bottom = div().px(8.0).pb(8.0).child(
            div()
                .id(add)
                .on(Action::Click)
                .w_full()
                .py(8.0)
                .rounded(8.0)
                .bg(white(0.0).mix(white(0.04), add_hot))
                .center()
                .role(Role::Button, "Add task")
                .child(ico("plus", 12.0, 2.0, faint().mix(secondary(), add_hot))),
        );
        let mut column = div()
            .id(column_id(st.key))
            .sense(Sense::HOVER)
            .col()
            .flex(1.0)
            .min_w0()
            .min_h(0.0)
            .rounded(8.0)
            .role(Role::Group, st.label)
            .child(header(s, st, in_status.len(), false))
            .child(cards)
            .child(bottom);
        if over {
            column = column.bg(white(0.04)).border(1.0, white(0.10));
        }
        row = row.child(column);
    }
    row
}

fn empty(s: &Screen<'_>) -> El {
    let filtered = s.st.tasks.filter.is_some();
    let head = if filtered {
        "No matching tasks"
    } else {
        "No tasks yet"
    };
    let sub = if filtered {
        "Try changing the status filter".to_owned()
    } else if let Some(p) = &s.st.project {
        format!("Create a task for {p} to get started")
    } else {
        "Select a project or create a task to get started".to_owned()
    };
    div()
        .col()
        .grow()
        .w_full()
        .items_center()
        .justify_center()
        .child(div().pb(12.0).child(ico("list-todo", 40.0, 1.0, faint())))
        .child(
            div()
                .pb(4.0)
                .child(text(head, 14.0, faint()).line_height(20.0).silent()),
        )
        .child(text(sub, 12.0, faint()).line_height(16.0).silent())
}

/// The tasks the board shows.
pub(super) fn shown<'a>(s: &Screen<'a>) -> Vec<&'a TaskConfig> {
    let names: Vec<&str> = s.store.projects.iter().map(|p| p.name.as_str()).collect();
    s.st.tasks
        .visible(&s.store.tasks, s.st.project.as_deref(), &names)
}

/// The whole view: the board and, with a task selected, its panel.
pub(super) fn view(s: &Screen<'_>) -> El {
    let shown = shown(s);
    let board = s.st.tasks.mode == Mode::Board;
    let mut content = div()
        .id(Id::new("task-content"))
        .col()
        .grow()
        .min_w0()
        .min_h(0.0)
        .p(16.0);
    content = if shown.is_empty() {
        content.child(empty(s))
    } else if board {
        content.child(kanban(s, &shown))
    } else {
        content.scroll_y().child(list(s, &shown))
    };
    let mut row = div().row().grow().min_h(0.0).child(content);
    if let Some(panel) = detail::panel(s) {
        row = row.child(panel);
    }
    row
}

/// The dragged card, drawn at the pointer over everything.
pub(super) fn ghost(s: &Screen<'_>) -> Option<El> {
    let (id, _) = s.st.tasks.dragging.as_ref()?;
    let (mid, dx, dy) = s.ui.moving()?;
    if mid != card_id(id) {
        return None;
    }
    let r = s.ui.rect_of(mid)?;
    let t = s.store.tasks.iter().find(|t| &t.id == id)?;
    Some(
        div()
            .absolute(r.x + dx, r.y + dy)
            .w(r.w)
            .opacity(0.9)
            .silent()
            .child(kanban_card(s, t, true)),
    )
}

/// The toolbar popover's entries (`TaskToolbar`).
pub(in crate::screen) fn options_items(s: &Screen<'_>) -> Vec<Item> {
    let t = &s.st.tasks;
    let mut items = vec![Item::header("Status")];
    let mut all = Item::option("All", t.filter.is_none(), Act::Task(TaskAct::Filter(None)));
    all.dot = Some(0x8b8a88);
    items.push(all);
    for st in &STATUSES {
        let mut it = Item::option(
            st.label,
            t.filter.as_deref() == Some(st.key),
            Act::Task(TaskAct::Filter(Some(st.key.to_owned()))),
        );
        it.dot = Some(dot(st.key));
        items.push(it);
    }
    let connectors = tasks::connectors(&s.store.tasks);
    if !connectors.is_empty() {
        items.push(Item::header("Source").sep());
        let mut all = Item::option(
            "All Sources",
            t.source == Source::All,
            Act::Task(TaskAct::Source(Source::All)),
        );
        all.dot = Some(0x8b8a88);
        items.push(all);
        let mut local = Item::option(
            "Local Only",
            t.source == Source::Local,
            Act::Task(TaskAct::Source(Source::Local)),
        );
        local.dot = Some(0x5a5a5e);
        items.push(local);
        for cid in connectors {
            let mut name = cid.clone();
            if let Some(f) = name.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            let on = t.source == Source::Connector(cid.clone());
            items.push(Item::option(
                name,
                on,
                Act::Task(TaskAct::Source(Source::Connector(cid))),
            ));
        }
    }
    let sw = s
        .acts
        .on("task-archived", Act::Task(TaskAct::ToggleArchived));
    items.push(Item {
        separator: true,
        row: Some(
            div()
                .id(sw)
                .on(Action::Click)
                .row()
                .items_center()
                .gap(8.0)
                .px(12.0)
                .py(6.0)
                .role(Role::Switch, "Include archived")
                .toggled(t.archived)
                .child(ico("archive", 12.0, 2.0, faint()))
                .child(
                    div().grow().child(
                        text("Include archived", 12.0, secondary())
                            .line_height(18.0)
                            .silent(),
                    ),
                )
                .child(crate::ui::widgets::switch_track(t.archived)),
        ),
        ..Item::default()
    });
    items.push(Item::header("View").sep());
    let seg = |mode: Mode, label: &str, icon: &str| {
        let on = t.mode == mode;
        let id = s.acts.on("task-mode", Act::Task(TaskAct::Mode(mode)));
        let hot = s.ui.hover(id);
        let fg = if on {
            c(look::WHITE)
        } else {
            faint().mix(secondary(), hot)
        };
        div()
            .id(id)
            .on(Action::Click)
            .row()
            .items_center()
            .gap(6.0)
            .px(10.0)
            .py(4.0)
            .rounded(6.0)
            .bg(if on { white(0.10) } else { white(0.04) })
            .role(Role::Button, label)
            .selected(on)
            .child(ico(icon, 12.0, 2.0, fg))
            .child(text(label, 11.0, fg).line_height(16.5).silent())
    };
    items.push(Item {
        row: Some(
            div()
                .row()
                .gap(4.0)
                .px(12.0)
                .py(4.0)
                .child(seg(Mode::List, "List", "list"))
                .child(seg(Mode::Board, "Board", "columns-3")),
        ),
        ..Item::default()
    });
    items
}

/// `KanbanCardMenu`'s entries.
pub(in crate::screen) fn card_items(s: &Screen<'_>, id: &str) -> Vec<Item> {
    let Some(t) = s.store.tasks.iter().find(|t| t.id == id) else {
        return Vec::new();
    };
    let status = t.status.to_string();
    let archived = t.archived_at.is_some();
    let key = || id.to_owned();
    let mut items = Vec::new();
    let faint_rgb = 0x5a5a5e;
    let sec_rgb = 0x8b8a88;
    if status == "in_progress" && can_open(s, t) {
        let l = live(s, t);
        items.push(
            Item::new(
                if l { "terminal" } else { "play" },
                if l { "Focus session" } else { "Resume session" },
                Act::Task(TaskAct::OpenSession(key())),
            )
            .tint(if l { look::INK } else { sec_rgb }),
        );
    }
    if status == "in_review" {
        items.push(
            Item::new(
                "file-code",
                "Review diff",
                Act::Task(TaskAct::ReviewDiff(key())),
            )
            .tint(sec_rgb),
        );
        items.push(
            Item::new(
                "circle-check",
                "Mark as done",
                Act::Task(TaskAct::Complete(key())),
            )
            .tint(sec_rgb),
        );
    }
    if status == "cancelled" {
        items.push(
            Item::new(
                "rotate-ccw",
                "Reopen task",
                Act::Task(TaskAct::Reopen(key())),
            )
            .tint(sec_rgb),
        );
    }
    items.push(Item::new("pencil", "Edit", Act::Task(TaskAct::Edit(key()))).tint(faint_rgb));
    let terminal = tasks::is_terminal(&status);
    let mut sibling = false;
    if !terminal {
        items.push(
            Item::new("circle-x", "Cancel task", Act::Task(TaskAct::Cancel(key())))
                .tint(look::DANGER)
                .sep(),
        );
        sibling = true;
    }
    if !archived && terminal {
        items.push(
            Item::new("archive", "Archive", Act::Task(TaskAct::Archive(key())))
                .tint(faint_rgb)
                .sep(),
        );
        sibling = true;
    }
    if archived {
        items.push(
            Item::new(
                "archive-restore",
                "Unarchive",
                Act::Task(TaskAct::Unarchive(key())),
            )
            .tint(faint_rgb)
            .sep(),
        );
        sibling = true;
    }
    if s.st.tasks.confirm_delete.as_deref() == Some(id) {
        let btn = |place: &str, act: Act, label: &str, danger: bool| {
            let bid = s.acts.on(place, act);
            let hot = s.ui.hover(bid);
            let (fg, bg) = if danger {
                (c(look::DANGER), Rgba::hexa(look::DANGER, 0.10 + 0.10 * hot))
            } else {
                (secondary().mix(primary(), hot), white(0.04 + 0.04 * hot))
            };
            div()
                .id(bid)
                .on(Action::Click)
                .px(8.0)
                .py(4.0)
                .rounded(6.0)
                .bg(bg)
                .role(Role::Button, label)
                .child(
                    text(label, 11.0, fg)
                        .weight(if danger { 500 } else { 400 })
                        .line_height(14.0)
                        .silent(),
                )
        };
        return vec![Item {
            row: Some(
                div()
                    .col()
                    .px(12.0)
                    .py(8.0)
                    .child(div().pb(10.0).child(
                        text("Delete this task permanently?", 12.0, secondary()).line_height(16.0),
                    ))
                    .child(
                        div()
                            .row()
                            .justify_end()
                            .gap(8.0)
                            .child(btn(
                                "task-card-delete-cancel",
                                Act::Task(TaskAct::AskDelete(String::new())),
                                "Cancel",
                                false,
                            ))
                            .child(
                                btn(
                                    "task-card-delete",
                                    Act::Task(TaskAct::Delete(key())),
                                    "Delete",
                                    true,
                                )
                                .border(1.0, Rgba::hexa(look::DANGER, 0.20)),
                            ),
                    ),
            ),
            ..Item::default()
        }];
    }
    let mut del =
        Item::new("trash-2", "Delete", Act::Task(TaskAct::AskDelete(key()))).tint(look::DANGER);
    del.separator = !sibling;
    items.push(del);
    items
}

/// `StatusPicker`'s entries; a task's own status is reached from todo only
/// when it starts.
pub(in crate::screen) fn status_items(s: &Screen<'_>, form: Form) -> Vec<Item> {
    let f = s.st.tasks.form_ref(form);
    let current = f.status.as_str();
    let stored = form == Form::Detail && f.editing;
    STATUSES
        .iter()
        .map(|st| {
            let mut it = Item::new(
                st.icon,
                st.label,
                Act::Task(TaskAct::SetStatus(form, st.key.to_owned())),
            )
            .tint(st.color.unwrap_or(0x5a5a5e))
            .disabled(stored && st.key == "in_progress" && current != "todo");
            it.checked = st.key == current;
            it
        })
        .collect()
}

/// `ProjectPicker`'s entries.
pub(in crate::screen) fn project_items(s: &Screen<'_>, form: Form) -> Vec<Item> {
    let current = &s.st.tasks.form_ref(form).project;
    s.store
        .projects
        .iter()
        .map(|p| {
            let mut it = Item::new(
                "folder",
                &p.name,
                Act::Task(TaskAct::PickProject(form, p.name.clone())),
            );
            it.checked = &p.name == current;
            it
        })
        .collect()
}

/// `AgentPicker`'s entries, None first.
pub(in crate::screen) fn agent_items(s: &Screen<'_>, form: Form) -> Vec<Item> {
    let current = s.st.tasks.form_ref(form).agent.as_deref();
    let mut none = Item::new("circle", "None", Act::Task(TaskAct::PickAgent(form, None)));
    none.checked = current.is_none();
    let mut items = vec![none];
    for (k, name) in AGENTS {
        let ok =
            s.st.installed
                .as_ref()
                .is_none_or(|i| i.iter().any(|a| a == k));
        let mut it = Item::new(
            "bot",
            name,
            Act::Task(TaskAct::PickAgent(form, Some(k.to_owned()))),
        )
        .tint(look::GRAY_400)
        .disabled(!ok);
        it.checked = current == Some(k);
        if !ok {
            it.detail = Some("Not installed".to_owned());
        }
        items.push(it);
    }
    items
}

/// How a picker's trigger is drawn: `StatusPicker` is bare,
/// `ProjectPicker` adds a chevron, `AgentPicker` a chevron in an outline.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger {
    Bare,
    Chevron,
    Outlined,
}

/// A picker's trigger: an icon and a label that open `kind`.
#[allow(clippy::too_many_arguments)]
fn picker(
    s: &Screen<'_>,
    place: &str,
    kind: MenuKind,
    icon: El,
    label: &str,
    color: Rgba,
    look: Trigger,
    disabled: bool,
) -> El {
    let open = s.st.menu.as_ref().is_some_and(|m| m.kind == kind);
    let id = s.acts.on(place, Act::Open(kind));
    let hot = if disabled { 0.0 } else { s.ui.hover(id) };
    let mut b = div()
        .id(id)
        .row()
        .items_center()
        .gap(6.0)
        .px(6.0)
        .py(2.0)
        .rounded(4.0)
        .bg(white(0.0).mix(white(0.04), hot))
        .role(Role::Button, label)
        .has_popup()
        .expanded(open)
        .child(icon)
        .child(
            text(label, 12.0, color)
                .line_height(18.0)
                .truncate()
                .silent(),
        );
    if look != Trigger::Bare {
        b = b.child(ico("chevron-down", 11.0, 2.0, c(look::GRAY_500)));
    }
    b = if look == Trigger::Outlined {
        b.border(1.0, white(0.12).mix(white(0.25), hot))
    } else {
        // `-mx-1.5`: the label lines up with the text around it.
        b.mx(-6.0)
    };
    if !disabled {
        b = b.on(Action::Click);
    }
    b
}

pub(super) fn status_picker(s: &Screen<'_>, form: Form, disabled: bool) -> El {
    let f = s.st.tasks.form_ref(form);
    // An open task's picker shows the task as it is now, moved from anywhere.
    let live = (form == Form::Detail && f.editing)
        .then(|| s.store.tasks.iter().find(|t| t.id == f.id))
        .flatten()
        .map(|t| t.status.to_string());
    let status = live.unwrap_or_else(|| f.status.clone());
    let st = tasks::status(&status);
    picker(
        s,
        "task-status-picker",
        MenuKind::TaskStatus(form),
        ico(st.icon, 13.0, 2.0, tone(&status)),
        st.label,
        tone(&status),
        Trigger::Bare,
        disabled,
    )
}

pub(super) fn project_picker(s: &Screen<'_>, form: Form) -> El {
    let p = &s.st.tasks.form_ref(form).project;
    let label = if p.is_empty() {
        "Select project..."
    } else {
        p.as_str()
    };
    picker(
        s,
        "task-project-picker",
        MenuKind::TaskProject(form),
        s.store.projects.iter().find(|q| &q.name == p).map_or_else(
            || ico("folder", 14.0, 1.5, c(look::GRAY_500)),
            |q| crate::screen::sidebar_project_icon(q, 14.0),
        ),
        label,
        if p.is_empty() {
            c(look::GRAY_600)
        } else {
            c(look::GRAY_300)
        },
        Trigger::Chevron,
        false,
    )
}

pub(super) fn agent_picker(s: &Screen<'_>, form: Form) -> El {
    let a = s.st.tasks.form_ref(form).agent.as_deref();
    let icon = match a {
        Some(k) => crate::screen::agent_icon(s, k, 14.0),
        None => ico("bot", 14.0, 2.0, c(look::GRAY_500)),
    };
    picker(
        s,
        "task-agent-picker",
        MenuKind::TaskAgent(form),
        icon,
        a.map_or("Unassigned", crate::screen::menus::agent_name),
        if a.is_some() {
            c(look::GRAY_300)
        } else {
            c(look::GRAY_600)
        },
        Trigger::Outlined,
        false,
    )
}
