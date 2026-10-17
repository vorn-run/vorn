//! Tasks: the board by column, its scope, one task, and writing one.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use vorn_app::tasks::{self, STATUSES};
use vorn_protocol::TaskConfig;
use vornui::{El, Id, Ui};

use super::options::{self, TASK_FIELDS, TASK_GROUP, TASK_ORDER};
use super::{
    bar, choice, col, empty, icon, link, listing, menu, present, row, say, section, swipe, Cx, Do,
};
use crate::model::{Fetched, For, Link};
use crate::phone::Phone;

pub fn title_id(task: &str) -> Id {
    Id::new("task-title").child(task)
}

pub fn body_id(task: &str) -> Id {
    Id::new("task-body").child(task)
}

const NEW_TITLE: &str = "new-task-title";
pub(super) const NEW_BODY: &str = "new-task-body";

fn live(t: &TaskConfig) -> bool {
    t.archived_at.as_ref().is_none_or(String::is_empty)
}

fn status_of(t: &TaskConfig) -> String {
    t.status.to_string()
}

pub fn board(cx: &Cx) -> El {
    let m = &cx.p.model;
    let scope = cx.view().tasks_scope.as_deref();
    let head = bar(
        "Tasks",
        vec![
            super::filter(
                Id::new("tasks-scope-open"),
                "Show tasks from",
                "tasks-scope".into(),
                scope.is_some(),
            ),
            options::button("tasks-view-open", "tasks-view", cx.view().tasks_changed()),
            present(
                Id::new("task-new-open"),
                "New task",
                "task-new".into(),
                Some("plus"),
            ),
        ],
    );
    if m.store.tasks.is_empty() {
        let body = match &m.link {
            Link::Online => empty(
                "No tasks yet",
                "Tasks you write on the desktop show up here, and you can move them between columns from your phone.",
                vec![],
            ),
            Link::Connecting => empty("Loading the board.", "", vec![]),
            Link::Offline(why) => empty("Nothing to show until the host answers.", why, vec![]),
            Link::Unpaired => empty("Not paired", "Pair the phone with a computer first.", vec![]),
        };
        return col(vec![head, body]);
    }
    let source = match cx.view().source() {
        "all" => tasks::Source::All,
        "local" => tasks::Source::Local,
        id => tasks::Source::Connector(id.to_owned()),
    };
    let scoped: Vec<&TaskConfig> = m
        .store
        .tasks
        .iter()
        .filter(|t| {
            (cx.view().archived || live(t))
                && source.keeps(t)
                && scope.is_none_or(|p| t.project_name == p)
        })
        .collect();
    let column = cx.view().column;
    let tabs = choice(
        "",
        STATUSES
            .iter()
            .map(|s| {
                let n = scoped.iter().filter(|t| status_of(t) == s.key).count();
                cx.option(
                    Id::new("column").child(s.key),
                    &format!("{} {n}", s.label),
                    s.key == column,
                    Do::Column(s.key),
                )
            })
            .collect(),
    );
    let mut here: Vec<&TaskConfig> = scoped
        .iter()
        .copied()
        .filter(|t| status_of(t) == column)
        .collect();
    match cx.view().pick(&TASK_ORDER) {
        "newest" => here.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        "oldest" => here.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        _ => here.sort_by(|a, b| a.order.total_cmp(&b.order)),
    }
    let key = format!("tasks:{column}");
    let limit = cx.view().limit(&key);
    let mut rows: Vec<El> = Vec::new();
    if let Some(p) = scope {
        rows.push(row(vec![
            say(format!("Showing {p}"), "secondary"),
            cx.button(
                Id::new("tasks-scope-clear"),
                "Show all",
                "plain",
                Do::TasksScope(None),
            ),
        ]));
    }
    if here.is_empty() {
        rows.push(say(
            match scope {
                Some(p) if scoped.is_empty() => format!("Nothing in {p}."),
                _ => "Nothing in this column.".to_owned(),
            },
            "secondary",
        ));
    }
    let shown = &here[..here.len().min(limit)];
    if cx.view().pick(&TASK_GROUP) == "project" {
        let mut groups: Vec<(&str, Vec<El>)> = Vec::new();
        for t in shown {
            let el = task_row(cx, t);
            match groups.iter_mut().find(|g| g.0 == t.project_name) {
                Some(g) => g.1.push(el),
                None => groups.push((&t.project_name, vec![el])),
            }
        }
        rows.extend(
            groups.into_iter().map(|(name, kids)| {
                super::marked_section(name, super::project_mark(cx, name), kids)
            }),
        );
    } else {
        rows.extend(shown.iter().map(|t| task_row(cx, t)));
    }
    rows.extend(cx.more(&key, shown.len(), here.len()));
    col(vec![head, tabs, listing(rows)])
}

fn task_row(cx: &Cx, t: &TaskConfig) -> El {
    let title = if t.title.trim().is_empty() {
        "Untitled"
    } else {
        t.title.as_str()
    };
    let st = tasks::status(&status_of(t));
    let meta = meta(cx, t);
    let mut lines = vec![say(title, "title")];
    if !meta.is_empty() {
        lines.push(say(meta, "secondary"));
    }
    let mut kids = vec![icon(st.icon), col(lines)];
    if !live(t) {
        kids.push(icon("archive"));
    }
    kids.extend(row_actions(cx, t));
    link(
        Id::new("task-row").child(&t.id),
        title,
        format!("task:{}", t.id),
        kids,
    )
}

/// The facts under a task's title, as the options ask.
fn meta(cx: &Cx, t: &TaskConfig) -> String {
    cx.view()
        .fields(&TASK_FIELDS)
        .into_iter()
        .filter_map(|f| match f {
            "project" => Some(t.project_name.clone()),
            "date" => Some(tasks::short_date(&t.created_at)),
            "status" => Some(tasks::status(&status_of(t)).label.to_owned()),
            "agent" => t
                .assigned_agent
                .as_ref()
                .map(|a| cx.p.model.store.agents.label(&a.to_string()).to_owned()),
            "branch" => t.branch.clone().filter(|b| !b.is_empty()),
            "source" => Some(match &t.source_connector_id {
                Some(id) => options::connector_name(cx, id),
                None => "Local".to_owned(),
            }),
            "id" => Some(tasks::short_id(&t.project_name, &t.id)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What a row offers when swiped: complete what is open; file away what is over, or bring it back; delete.
fn row_actions(cx: &Cx, t: &TaskConfig) -> Vec<El> {
    let id = t.id.as_str();
    let over = tasks::is_terminal(&status_of(t));
    let mut out = Vec::new();
    if !over {
        out.push(swipe(
            "leading",
            vec![cx.icon_button(
                Id::new("task-complete").child(id),
                "Complete",
                "check",
                "primary",
                Do::TaskStatus(id.to_owned(), "done"),
            )],
        ));
    }
    let mut trailing = vec![cx.asking_with(
        Id::new("task-delete").child(id),
        "Delete",
        DELETE_ASK,
        Some("trash-2"),
        Do::DeleteTask(id.to_owned(), false),
    )];
    if !live(t) {
        trailing.push(cx.icon_button(
            Id::new("task-unarchive").child(id),
            "Unarchive",
            "archive-restore",
            "plain",
            Do::Archive(id.to_owned(), false),
        ));
    } else if over {
        trailing.push(cx.icon_button(
            Id::new("task-archive").child(id),
            "Archive",
            "archive",
            "plain",
            Do::Archive(id.to_owned(), true),
        ));
    }
    out.push(swipe("trailing", trailing));
    out
}

const DELETE_ASK: &str = "Delete this task permanently?";

pub fn scope(cx: &Cx) -> El {
    let search = Id::new("tasks-scope-find");
    let find = cx.ui.edit_text(search).trim().to_lowercase();
    let current = cx.view().tasks_scope.as_deref();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &cx.p.model.store.projects {
        counts.entry(p.name.as_str()).or_default();
    }
    let mut all = 0;
    for t in cx.p.model.store.tasks.iter().filter(|t| live(t)) {
        *counts.entry(t.project_name.as_str()).or_default() += 1;
        all += 1;
    }
    let mut rows = vec![
        cx.field(search, "Find a project", false),
        cx.option(
            Id::new("tasks-scope-all"),
            &format!("All projects · {all}"),
            current.is_none(),
            Do::TasksScope(None),
        ),
    ];
    for (p, n) in counts {
        if !find.is_empty() && !p.to_lowercase().contains(&find) {
            continue;
        }
        rows.push(
            cx.option(
                Id::new("tasks-scope").child(p),
                &format!("{p} · {n}"),
                current == Some(p),
                Do::TasksScope(Some(p.to_owned())),
            )
            .child(super::project_mark(cx, p)),
        );
    }
    col(vec![bar("Show tasks from", vec![]), listing(rows)])
}

fn detail(label: &str, value: &str) -> El {
    row(vec![say(label, "body"), say(value, "secondary")])
}

pub fn task(cx: &Cx, id: &str) -> El {
    let Some(t) = cx.p.model.store.tasks.iter().find(|t| t.id == id) else {
        return col(vec![
            bar("Task", vec![]),
            empty("This task is gone.", "", vec![]),
        ]);
    };
    let title = if t.title.trim().is_empty() {
        "Untitled"
    } else {
        t.title.as_str()
    };
    let current = status_of(t);
    let statuses = STATUSES
        .iter()
        .map(|s| {
            cx.option(
                Id::new("task-status").child((id, s.key)),
                s.label,
                s.key == current,
                Do::TaskStatus(id.to_owned(), s.key),
            )
        })
        .collect();
    let agent = t.assigned_agent.as_ref().map_or_else(
        || "Not set".to_owned(),
        |a| cx.p.model.store.agents.label(&a.to_string()).to_owned(),
    );
    let branch = t.branch.as_deref().filter(|b| !b.is_empty());
    let mut rows = vec![
        section(
            "",
            vec![
                say(title, "title"),
                say(tasks::short_id(&t.project_name, &t.id), "secondary"),
                choice("Status", statuses),
            ],
        ),
        section(
            "Details",
            vec![
                row(vec![
                    say("Project", "body"),
                    super::project_mark(cx, &t.project_name),
                    say(&t.project_name, "secondary"),
                ]),
                detail("Agent", &agent),
                detail("Branch", branch.unwrap_or("feature/my-task")),
            ],
        ),
        section(
            "Description",
            vec![if t.description.trim().is_empty() {
                say("Nothing written yet.", "secondary")
            } else {
                say(t.description.trim(), "markdown")
            }],
        ),
    ];
    let runs = match cx.fetch("workflowRun:listByTask", json!({ "taskId": id })) {
        Fetched::Ready(Value::Array(list)) => super::workflows::run_rows(cx, &list),
        Fetched::Ready(_) => vec![],
        Fetched::Loading => vec![say("Loading runs.", "secondary")],
        Fetched::Failed(e) => vec![say(e, "error")],
    };
    if !runs.is_empty() {
        rows.push(section("Runs", runs));
    }
    let mut items = vec![present(
        Id::new("task-edit-open").child(id),
        "Edit",
        format!("task-edit:{id}"),
        Some("pencil"),
    )];
    if !live(t) {
        items.push(cx.icon_button(
            Id::new("task-unarchive").child(id),
            "Put this task back",
            "archive-restore",
            "plain",
            Do::Archive(id.to_owned(), false),
        ));
    } else if tasks::is_terminal(&current) {
        items.push(cx.icon_button(
            Id::new("task-archive").child(id),
            "Archive this task",
            "archive",
            "plain",
            Do::Archive(id.to_owned(), true),
        ));
    }
    items.push(cx.asking_with(
        Id::new("task-delete").child(id),
        "Delete this task",
        DELETE_ASK,
        Some("trash-2"),
        Do::DeleteTask(id.to_owned(), true),
    ));
    col(vec![bar(title, vec![menu(items)]), listing(rows)])
}

/// Files the task away or brings it back, as the desktop does; only finished work is filed.
pub fn archive(p: &mut Phone, id: &str, archived: bool) {
    change(p, id, |config, now| {
        tasks::set_archived(config, id, archived, now)
    });
}

/// Deletes the task, leaving its page when `leave`.
pub fn delete(p: &mut Phone, id: &str, leave: bool) {
    change(p, id, |config, _| tasks::remove(config, id));
    if leave {
        p.model.go = Some("back".into());
    }
}

/// Changes a task's status as the desktop does, and tells vornd.
pub fn set_status(p: &mut Phone, id: &str, status: &str) {
    change(p, id, |config, now| {
        tasks::set_status(config, id, status, now)
    });
}

fn change(p: &mut Phone, id: &str, f: impl FnOnce(&mut Value, &str) -> bool) {
    let store = &mut p.model.store;
    let row = |config: &Value| {
        config["tasks"]
            .as_array()
            .and_then(|l| l.iter().find(|t| t["id"] == id))
            .cloned()
    };
    let before = row(&store.config);
    if !f(&mut store.config, &tasks::now_iso()) {
        return;
    }
    store.reread_task(id);
    let after = row(&store.config);
    for (method, params) in vorn_app::client::sync::task(before.as_ref(), after.as_ref()) {
        p.model.call(For::Quiet, method, params);
    }
}

fn new_project(p: &Phone) -> Option<String> {
    let want = p
        .view
        .new_task_project
        .clone()
        .or_else(|| p.view.tasks_scope.clone());
    let projects = &p.model.store.projects;
    projects
        .iter()
        .find(|x| Some(&x.name) == want.as_ref())
        .or_else(|| projects.first())
        .map(|x| x.name.clone())
}

pub fn new(cx: &Cx) -> El {
    let project = new_project(cx.p);
    let projects =
        cx.p.model
            .store
            .projects
            .iter()
            .map(|x| {
                cx.option(
                    Id::new("task-new-project").child(&x.name),
                    &x.name,
                    project.as_ref() == Some(&x.name),
                    Do::NewTaskProject(x.name.clone()),
                )
            })
            .collect();
    let mut rows = vec![
        cx.field(Id::new(NEW_TITLE), "Title", false),
        cx.prose(Id::new(NEW_BODY), "What needs to be done"),
        choice("Project", projects),
        say(
            format!("Goes in {}.", tasks::status(cx.view().column).label),
            "secondary",
        ),
    ];
    rows.extend(cx.error("task"));
    col(vec![
        bar(
            "New task",
            vec![cx.button(Id::new("task-new-add"), "Add", "primary", Do::NewTask)],
        ),
        listing(rows),
    ])
}

pub fn add(p: &mut Phone, ui: &mut Ui) {
    let title = ui.edit_text(Id::new(NEW_TITLE)).trim().to_owned();
    if title.is_empty() {
        p.model
            .errors
            .insert("task".into(), "Give the task a title.".into());
        return;
    }
    let Some(project) = new_project(p) else {
        p.model
            .errors
            .insert("task".into(), "Add a project on the computer first.".into());
        return;
    };
    p.model.errors.remove("task");
    let body = ui.edit_text(Id::new(NEW_BODY)).to_owned();
    let form = tasks::TaskForm {
        id: tasks::uuid(),
        status: p.view.column.to_owned(),
        project,
        ..tasks::TaskForm::default()
    };
    let id = form.id.clone();
    change(p, &id, |config, now| {
        tasks::add(config, &form, &title, &body, now);
        true
    });
    ui.set_edit_text(Id::new(NEW_TITLE), "");
    ui.set_edit_text(Id::new(NEW_BODY), vorn_app::tasks::TEMPLATE);
    p.model.go = Some("back".into());
}

pub fn edit(cx: &Cx, id: &str) -> El {
    col(vec![
        bar(
            "Edit task",
            vec![cx.button(
                Id::new("task-save").child(id),
                "Save",
                "primary",
                Do::SaveTask(id.to_owned()),
            )],
        ),
        listing(vec![
            cx.field(title_id(id), "Untitled", false),
            cx.prose(body_id(id), "Nothing written yet."),
        ]),
    ])
}

pub fn save(p: &mut Phone, ui: &mut Ui, id: &str) {
    let (t, b) = (title_id(id), body_id(id));
    let title = ui.edit_text(t).trim().to_owned();
    let body = ui.edit_text(b).trim().to_owned();
    p.view.seeded.remove(&t);
    p.view.seeded.remove(&b);
    change(p, id, |config, now| {
        tasks::patch(config, id, now, |row| {
            row.insert("title".into(), json!(title));
            row.insert("description".into(), json!(body));
        })
    });
    p.model.go = Some("back".into());
}
