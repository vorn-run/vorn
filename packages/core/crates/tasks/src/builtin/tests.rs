use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Map;
use vorn_extensions::events::AGENT_CONVERSATION;
use vorn_extensions::manifest::{file, Place};
use vorn_store::StoreOptions;

use super::*;
use crate::host::{Pending, TableWork};

/// A host over a database of the test's own, recording what it is told.
pub(crate) struct Fake {
    db: PathBuf,
    events: Mutex<Vec<(String, Value)>>,
    changes: Mutex<Vec<Change>>,
    published: Mutex<Vec<RowOp>>,
    agents: Value,
}

fn options() -> StoreOptions {
    StoreOptions {
        default_shell: "/bin/sh".into(),
        default_agent_commands: Map::new(),
        default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#000", "order": 0 })).unwrap(),
        owner_name: "o".into(),
        seed_workflows: Vec::new(),
    }
}

impl Fake {
    pub(crate) fn new(dir: &Path) -> Arc<Fake> {
        let db = dir.join("vorn.db");
        let (mut store, _) = Store::open(&db, options()).unwrap();
        for name in ["app", "other"] {
            store
                .call(
                    "dbInsertProject",
                    json!([{ "name": name, "path": format!("/{name}"), "preferredAgents": [] }]),
                )
                .unwrap();
        }
        Arc::new(Fake {
            db,
            events: Mutex::new(Vec::new()),
            changes: Mutex::new(Vec::new()),
            published: Mutex::new(Vec::new()),
            agents: json!([{ "id": "claude" }, { "id": "codex" }]),
        })
    }

    pub(crate) fn store(&self) -> Store {
        Store::open(&self.db, options()).unwrap().0
    }

    pub(crate) fn changes(&self) -> Vec<Change> {
        std::mem::take(&mut *self.changes.lock().unwrap())
    }

    fn published(&self) -> Vec<RowOp> {
        std::mem::take(&mut *self.published.lock().unwrap())
    }
}

impl Host for Fake {
    fn tables(&self, work: TableWork) -> Pending<Result<(), String>> {
        let mut store = self.store();
        Box::pin(async move { work(&mut store).map_err(|e| e.to_string()) })
    }

    fn data_dir(&self) -> Option<PathBuf> {
        self.db.parent().map(Path::to_path_buf)
    }

    fn emit(&self, name: &str, data: Value) {
        self.events.lock().unwrap().push((name.to_owned(), data));
    }

    fn changed(&self, change: Change) -> Pending<()> {
        self.changes.lock().unwrap().push(change);
        Box::pin(async {})
    }

    fn call(&self, method: &str, params: Value) -> Pending<Result<Value, String>> {
        let answer = match method {
            "agents.list" => Ok(self.agents.clone()),
            "task.get" => self
                .store()
                .call("dbGetTask", json!([params.get("id")]))
                .map_err(|e| e.to_string()),
            "project.list" => Ok(self.store().call("dbListProjects", json!([])).unwrap()),
            "task.list" => {
                let get = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
                let limit = get("limit").as_u64().unwrap_or(100);
                let offset = get("offset").as_u64().unwrap_or(0);
                self.store()
                    .call(
                        "dbListTasks",
                        json!([get("projectName"), get("status"), offset, limit]),
                    )
                    .map(|tasks| {
                        let n = tasks.as_array().map_or(0, Vec::len) as u64;
                        json!({ "tasks": tasks, "next": (n == limit).then_some(offset + n) })
                    })
                    .map_err(|e| e.to_string())
            }
            other => Err(format!("no {other} here")),
        };
        Box::pin(async move { answer })
    }

    fn publish(&self, _collection: &str, ops: Vec<RowOp>) {
        self.published.lock().unwrap().extend(ops);
    }
}

fn result(a: Answer) -> Value {
    match a {
        Answer::Result(v) => v,
        other => panic!("not answered: {other:?}"),
    }
}

fn ids(change: &Change) -> Vec<String> {
    match change {
        Change::Rows(rows) => rows
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect(),
        Change::Removed(ids) => ids.clone(),
    }
}

#[tokio::test]
async fn creating_or_moving_a_task_is_an_event_the_triggers_fire_from() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    let made = tasks
        .answer(
            "task:create",
            json!({ "id": "t1", "projectName": "app", "title": "T" }),
        )
        .await;
    assert!(matches!(made, Answer::Result(_)), "{made:?}");
    for set in [
        json!({ "title": "Renamed" }),
        json!({ "status": "in_progress" }),
        json!({ "status": "in_progress" }),
    ] {
        tasks
            .answer("task:patch", json!({ "id": "t1", "set": set }))
            .await;
    }
    let seen = host.events.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].0, TASK_CREATED);
    assert_eq!(seen[0].1["effectId"], "task-created/t1");
    assert_eq!(seen[1].0, TASK_STATUS_CHANGED);
    assert_eq!(
        (seen[1].1["from"].as_str(), seen[1].1["to"].as_str()),
        (Some("todo"), Some("in_progress"))
    );
    assert_eq!(seen[1].1["task"]["title"], "Renamed");
}

#[tokio::test]
async fn each_write_tells_the_rows_it_changed_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    for id in ["a", "b", "c"] {
        tasks
            .answer(
                "task:create",
                json!({ "id": id, "projectName": "app", "title": id }),
            )
            .await;
    }
    let made = host.changes();
    assert_eq!(
        made.iter().map(ids).collect::<Vec<_>>(),
        [["a"], ["b"], ["c"]]
    );
    let Change::Rows(rows) = &made[0] else {
        panic!("rows")
    };
    assert_eq!(rows[0]["projectName"], "app");

    tasks
        .answer("task:reorder", json!({ "ids": ["c", "a", "ghost", "c"] }))
        .await;
    let moved = host.changes();
    assert_eq!(moved.iter().map(ids).collect::<Vec<_>>(), [["c", "a"]]);

    tasks.answer("task:delete", json!({ "id": "b" })).await;
    assert_eq!(host.changes(), [Change::Removed(vec!["b".into()])]);

    // What changes nothing tells nothing.
    tasks
        .answer("task:update", json!({ "id": "ghost", "title": "x" }))
        .await;
    tasks.answer("task:delete", json!({ "id": "b" })).await;
    tasks
        .answer(
            "task:create",
            json!({ "projectName": "nowhere", "title": "x" }),
        )
        .await;
    assert_eq!(
        tasks.answer("task:get", json!({ "id": "a" })).await,
        Answer::Unanswered,
        "reading a task is the core's"
    );
    assert!(host.changes().is_empty());
    assert_eq!(
        tasks.answer("task:list", json!({})).await,
        Answer::Unanswered,
        "the list is the core's"
    );
}

#[tokio::test]
async fn a_session_working_on_a_task_shows_it_on_its_card() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    tasks
        .answer(
            "task:create",
            json!({ "id": "t1", "projectName": "app", "title": "Fix it" }),
        )
        .await;
    assert!(host.published().is_empty(), "nothing in progress yet");
    let set = json!({ "status": "in_progress", "assignedSessionId": "s1" });
    tasks
        .answer("task:patch", json!({ "id": "t1", "set": set }))
        .await;
    assert_eq!(
        host.published(),
        [RowOp::Upsert(
            json!({ "id": "s1", "task": "t1", "title": "Fix it" })
        )]
    );
    // A change that leaves the badge as it was publishes nothing.
    tasks
        .answer("task:update", json!({ "id": "t1", "description": "more" }))
        .await;
    assert!(host.published().is_empty());
    tasks
        .answer("task:setStatus", json!({ "id": "t1", "status": "done" }))
        .await;
    assert_eq!(host.published(), [RowOp::Delete("s1".into())]);
}

#[tokio::test]
async fn a_conversation_an_agent_starts_is_kept_on_its_terminals_task() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let mut store = host.store();
    for (id, status) in [("doing", "in_progress"), ("done", "done")] {
        let t = json!({
            "id": id, "projectName": "app", "title": id, "description": "", "status": status, "order": 0,
            "assignedSessionId": "t1", "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-01-01T00:00:00.000Z",
        });
        store.call("dbInsertTask", json!([t])).unwrap();
    }
    let tasks = Tasks::new(host.clone());
    tasks.keep_conversation("t1".into(), "c1".into()).await;
    let read = |id: &str| host.store().call("dbGetTask", json!([id])).unwrap();
    assert_eq!(read("doing")["agentSessionId"], "c1");
    assert!(read("done")["agentSessionId"].is_null());
    assert_eq!(
        host.changes().iter().map(ids).collect::<Vec<_>>(),
        [["doing"]]
    );
    // A second conversation does not take the place of the first.
    tasks.keep_conversation("t1".into(), "c2".into()).await;
    assert_eq!(read("doing")["agentSessionId"], "c1");
    assert!(host.changes().is_empty());
    assert_eq!(AGENT_CONVERSATION, "agent.conversation");
}

#[tokio::test]
async fn imports_a_connections_items_and_tells_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    host.store()
        .call(
            "dbInsertSourceConnection",
            json!([{ "id": "c1", "connectorId": "sdk", "name": "c1", "filters": {}, "syncIntervalMinutes": 0, "statusMapping": {}, "createdAt": "2026-01-01T00:00:00.000Z" }]),
        )
        .unwrap();
    let tasks = Tasks::new(host.clone());
    let import = json!({
        "connectionId": "c1", "connectorId": "sdk", "sourceConnectorId": "acme",
        "project": "app",
        "items": [{ "externalId": "42", "title": "From outside", "status": "todo" }],
    });
    let answered = tasks.import(import.clone()).await.unwrap();
    assert_eq!(answered[0]["created"], true);
    let id = answered[0]["taskId"].as_str().unwrap().to_owned();
    assert_eq!(
        host.changes().iter().map(ids).collect::<Vec<_>>(),
        [[id.clone()]]
    );
    let again = tasks.import(import).await.unwrap();
    assert_eq!(again[0]["created"], false);
    let link = tasks.source_link(id).await.unwrap();
    assert_eq!(link["externalId"], "42");
    assert!(tasks.import(json!({ "items": 1 })).await.is_err());
}

#[tokio::test]
async fn keeps_a_tasks_images_under_its_data_directory() {
    let dir = tempfile::tempdir().unwrap();
    let host = Fake::new(dir.path());
    let tasks = Tasks::new(host.clone());
    let name = result(
        tasks
            .answer(
                "task:imageUpload",
                json!({ "taskId": "t1", "base64": "cG5n", "filename": "a.PNG" }),
            )
            .await,
    );
    let path = result(
        tasks
            .answer(
                "task:imageGetPath",
                json!({ "taskId": "t1", "filename": name }),
            )
            .await,
    );
    assert!(path
        .as_str()
        .unwrap()
        .starts_with(dir.path().join("task-images").to_str().unwrap()));
    assert_eq!(
        tasks.answer("task:imageCleanup", json!("t1")).await,
        Answer::Void
    );
    assert_eq!(
        tasks
            .answer(
                "task:imageUpload",
                json!({ "taskId": "t1", "base64": "", "filename": "a.svg" }),
            )
            .await,
        Answer::Error("Unsupported image type: .svg".into())
    );
}

#[test]
fn its_manifest_and_templates_read() {
    let m = file::parse_builtin(MANIFEST).unwrap();
    assert_eq!(m.id, ID);
    let c = m.contributes.as_ref().unwrap();
    assert_eq!(
        c.points(Place::Views)[0].view.as_deref(),
        Some("views/board.kdl")
    );
    assert_eq!(
        c.points(Place::Badges)[0].view.as_deref(),
        Some("views/badge.kdl")
    );
    let mut tools: Vec<&str> = m.mcp.iter().map(|t| t.name.as_str()).collect();
    tools.sort_unstable();
    let mut listed: Vec<&str> = crate::tools::listed()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    listed.sort_unstable();
    assert_eq!(tools, listed);
    for (path, text) in crate::VIEWS {
        let place = if path.ends_with("badge.kdl") {
            Place::Badges
        } else {
            Place::Views
        };
        let read = vorn_extensions::views::read(path, text, place);
        assert!(read.is_ok(), "{path}: {read:?}");
    }
    let board = vorn_view::file::parse(
        "views/board.kdl",
        crate::VIEWS[0].1,
        vorn_view::Options::default(),
    )
    .unwrap();
    let vorn_view::Kind::Board(b) = board.view.kind else {
        panic!("a board")
    };
    let statuses: Vec<&str> = b.columns.iter().map(|c| c.value.as_str()).collect();
    assert_eq!(
        statuses,
        ["todo", "in_progress", "in_review", "done", "cancelled"]
    );
}
