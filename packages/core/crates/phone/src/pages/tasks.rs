//! Tasks: the board by column, its scope, one task, and writing one.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use vorn_app::screen::menus::agent_name;
use vorn_app::tasks::{self, STATUSES};
use vorn_protocol::TaskConfig;
use vornui::{El, Id, Ui};

use super::{bar, choice, col, empty, icon, link, listing, present, row, say, section, Cx, Do};
use crate::model::{Fetched, For, Link};
use crate::phone::Phone;

pub fn title_id(task: &str) -> Id {
    Id::new("task-title").child(task)
}

pub fn body_id(task: &str) -> Id {
    Id::new("task-body").child(task)
}

const NEW_TITLE: &str = "new-task-title";
const NEW_BODY: &str = "new-task-body";

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
            present(
                Id::new("tasks-scope-open"),
                "Show tasks from",
                "tasks-scope".into(),
                Some(if scope.is_some() {
                    "filter-on"
                } else {
                    "filter"
                }),
            ),
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
    let scoped: Vec<&TaskConfig> = m
        .store
        .tasks
        .iter()
        .filter(|t| live(t) && scope.is_none_or(|p| t.project_name == p))
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
    here.sort_by(|a, b| a.order.total_cmp(&b.order));
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
    rows.extend(here.iter().take(limit).map(|t| task_row(cx, t)));
    rows.extend(cx.more(&key, here.len().min(limit), here.len()));
    col(vec![head, tabs, listing(rows)])
}

fn task_row(cx: &Cx, t: &TaskConfig) -> El {
    let title = if t.title.trim().is_empty() {
        "Untitled"
    } else {
        t.title.as_str()
    };
    let st = tasks::status(&status_of(t));
    let when = vorn_app::runs::relative(&t.updated_at, cx.now);
    link(
        Id::new("task-row").child(&t.id),
        title,
        format!("task:{}", t.id),
        vec![
            icon(st.icon),
            col(vec![
                say(title, "title"),
                say(
                    format!(
                        "{} · {} · {when}",
                        t.project_name,
                        tasks::short_id(&t.project_name, &t.id)
                    ),
                    "secondary",
                ),
            ]),
        ],
    )
}

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
        rows.push(cx.option(
            Id::new("tasks-scope").child(p),
            &format!("{p} · {n}"),
            current == Some(p),
            Do::TasksScope(Some(p.to_owned())),
        ));
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
        |a| agent_name(&a.to_string()).to_owned(),
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
                detail("Project", &t.project_name),
                detail("Agent", &agent),
                detail("Branch", branch.unwrap_or("feature/my-task")),
            ],
        ),
        section(
            "Description",
            vec![if t.description.trim().is_empty() {
                say("Nothing written yet.", "secondary")
            } else {
                say(t.description.trim(), "body")
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
    col(vec![
        bar(
            title,
            vec![present(
                Id::new("task-edit-open").child(id),
                "Edit",
                format!("task-edit:{id}"),
                None,
            )],
        ),
        listing(rows),
    ])
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
        cx.field(Id::new(NEW_BODY), "What needs to be done", true),
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
    ui.set_edit_text(Id::new(NEW_BODY), "");
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
            cx.field(body_id(id), "Nothing written yet.", true),
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
