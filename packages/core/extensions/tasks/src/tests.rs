//! Tasks against a real `vorn.db`: the core's reads answered by the store,
//! and the extension's statements run as vornd runs them, on its own
//! connection under the authorizer with the tables Tasks is lent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map};
use vorn_extension_sdk::{dispatch, Blob, Ctx, Error, Extension, Init, Level, Op, Statement};
use vorn_store::extension::{ExtensionTables, TASKS};
use vorn_store::{Store, StoreOptions};

use super::*;

fn options() -> StoreOptions {
    StoreOptions {
        default_shell: "/bin/sh".into(),
        default_agent_commands: Map::new(),
        default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#000", "order": 0 })).unwrap(),
        owner_name: "o".into(),
        seed_workflows: Vec::new(),
    }
}

/// Vorn over a database of the test's own, recording what it is told.
struct Vorn {
    db: PathBuf,
    tables: ExtensionTables,
    store: Store,
    /// `task.changed`'s args, in order.
    changed: Vec<Value>,
    /// `workflow.fire`'s args, in order.
    fired: Vec<Value>,
    rows: BTreeMap<String, Vec<Value>>,
    /// The badges' ops, in order.
    published: Vec<Op>,
    /// How many times the open panes were told to draw again.
    redraws: usize,
    files: BTreeMap<String, Vec<u8>>,
}

impl Vorn {
    fn new(dir: &Path) -> Vorn {
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
        Vorn {
            tables: ExtensionTables::open_with(&db, ID, TASKS).unwrap(),
            db,
            store,
            changed: Vec::new(),
            fired: Vec::new(),
            rows: BTreeMap::new(),
            published: Vec::new(),
            redraws: 0,
            files: BTreeMap::new(),
        }
    }

    fn read(&mut self, id: &str) -> Value {
        self.store.call("dbGetTask", json!([id])).unwrap()
    }

    /// The tasks each `task.changed` named, in order, and what events it emitted.
    fn told(&mut self) -> Vec<Vec<String>> {
        std::mem::take(&mut self.changed)
            .iter()
            .filter(|c| c.get("events").is_none())
            .map(|c| {
                let rows = c.get("rows").and_then(Value::as_array);
                match rows {
                    Some(rows) => rows.iter().map(|r| js::to_string(&r["id"])).collect(),
                    None => c["removed"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(js::to_string)
                        .collect(),
                }
            })
            .collect()
    }
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::new("failed", e.to_string())
}

impl Host for Vorn {
    fn call(&mut self, method: &str, args: Value) -> Result<Value, Error> {
        match method {
            "task.get" => self
                .store
                .call("dbGetTask", json!([args["id"]]))
                .map_err(err),
            "task.list" => {
                let (offset, limit) = (
                    args["offset"].as_u64().unwrap_or(0),
                    args["limit"].as_u64().unwrap_or(500),
                );
                let tasks = self
                    .store
                    .call(
                        "dbListTasks",
                        json!([args.get("projectName"), args.get("status"), offset, limit]),
                    )
                    .map_err(err)?;
                let n = tasks.as_array().map_or(0, Vec::len) as u64;
                Ok(json!({ "tasks": tasks, "next": (n == limit).then_some(offset + n) }))
            }
            "project.list" => self.store.call("dbListProjects", json!([])).map_err(err),
            "sessions.get" => match args["sessionId"].as_str() {
                Some("s-app") => Ok(json!({ "id": "s-app", "projectName": "app" })),
                Some("s-loose") => Ok(json!({ "id": "s-loose" })),
                _ => Err(err("Session not found")),
            },
            "agents.list" => Ok(json!([{ "id": "claude" }, { "id": "codex" }])),
            CHANGED => {
                self.changed.push(args);
                Ok(Value::Null)
            }
            "workflow.fire" => {
                self.fired.push(args);
                Ok(json!({ "runs": [] }))
            }
            "files.read" => {
                let bytes =
                    std::fs::read(args["path"].as_str().unwrap_or_default()).map_err(err)?;
                Ok(json!({ "base64": base64(&bytes) }))
            }
            other => Err(Error::new(
                "unknown-method",
                format!("Vorn has no method {other}"),
            )),
        }
    }

    fn publish(&mut self, collection: &str, ops: Vec<Op>) {
        if collection == pane::REDRAW {
            self.redraws += 1;
            return;
        }
        let rows = self.rows.entry(collection.to_owned()).or_default();
        for op in &ops {
            match op {
                Op::Upsert(row) => {
                    rows.retain(|r| r["id"] != row["id"]);
                    rows.push(row.clone());
                }
                Op::Delete(id) => rows.retain(|r| r["id"] != id.as_str()),
                Op::Clear => rows.clear(),
            }
        }
        self.published.extend(ops);
    }

    fn log(&mut self, _: Level, _: &str) {}

    fn rows(&mut self, collection: &str) -> Result<Vec<Value>, Error> {
        Ok(self.rows.get(collection).cloned().unwrap_or_default())
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Value>, Error> {
        self.tables.query(sql, params).map_err(err)
    }

    fn transaction(&mut self, statements: &[Statement]) -> Result<Vec<Value>, Error> {
        let statements: Vec<(String, Vec<Value>)> = statements
            .iter()
            .map(|s| (s.sql.clone(), s.params.clone()))
            .collect();
        self.tables.transaction(&statements).map_err(err)
    }

    fn put_file(&mut self, name: &str, data: &[u8]) -> Result<(), Error> {
        self.files.insert(name.to_owned(), data.to_vec());
        Ok(())
    }

    fn files(&mut self, prefix: &str) -> Result<Vec<Blob>, Error> {
        Ok(self
            .files
            .iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .map(|(n, d)| Blob {
                name: n.clone(),
                size: d.len() as u64,
                modified: 0,
            })
            .collect())
    }

    fn delete_file(&mut self, name: &str) -> Result<bool, Error> {
        Ok(self.files.remove(name).is_some())
    }
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn tasks() -> Tasks {
    Tasks::default()
}

fn ask(t: &mut Tasks, v: &mut Vorn, method: &str, params: Value) -> Result<Value, Error> {
    dispatch(t, v, method, params, &Ctx::default())
}

#[test]
fn creating_or_moving_a_task_is_an_event_the_triggers_fire_from() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let made = ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "t1", "projectName": "app", "title": "T" }),
    )
    .unwrap();
    assert_eq!(made["ok"], true);
    for set in [
        json!({ "title": "Renamed" }),
        json!({ "status": "in_progress" }),
        json!({ "status": "in_progress" }),
    ] {
        ask(
            &mut t,
            &mut v,
            "task:patch",
            json!({ "id": "t1", "set": set }),
        )
        .unwrap();
    }
    let events: Vec<Value> = v
        .changed
        .iter()
        .filter_map(|c| c.get("events"))
        .flat_map(|e| e.as_array().cloned().unwrap_or_default())
        .collect();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["effectId"], "task-created/t1");
    assert_eq!(events[1]["kind"], "taskStatusChanged");
    assert_eq!(
        (events[1]["from"].as_str(), events[1]["to"].as_str()),
        (Some("todo"), Some("in_progress"))
    );
    assert_eq!(events[1]["task"]["title"], "Renamed");
    assert_eq!(v.fired.len(), 2);
    assert_eq!(v.fired[1]["fields"]["toStatus"], "in_progress");
}

#[test]
fn each_write_tells_the_rows_it_changed_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    for id in ["a", "b", "c"] {
        ask(
            &mut t,
            &mut v,
            "task:create",
            json!({ "id": id, "projectName": "app", "title": id }),
        )
        .unwrap();
    }
    assert_eq!(v.told(), [["a"], ["b"], ["c"]]);
    assert_eq!(v.read("c")["order"].as_f64(), Some(2.0));

    ask(
        &mut t,
        &mut v,
        "task:reorder",
        json!({ "ids": ["c", "a", "ghost", "c"] }),
    )
    .unwrap();
    assert_eq!(v.told(), [["c", "a"]]);
    assert_eq!(
        (v.read("c")["order"].as_f64(), v.read("a")["order"].as_f64()),
        (Some(0.0), Some(2.0))
    );

    ask(&mut t, &mut v, "task:delete", json!({ "id": "b" })).unwrap();
    assert_eq!(v.told(), [["b"]]);
    assert!(v.read("b").is_null());

    // What changes nothing tells nothing.
    let none = ask(
        &mut t,
        &mut v,
        "task:update",
        json!({ "id": "ghost", "title": "x" }),
    )
    .unwrap();
    assert_eq!(none, json!({ "ok": false }));
    ask(&mut t, &mut v, "task:delete", json!({ "id": "b" })).unwrap();
    let nowhere = ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "projectName": "nowhere", "title": "x" }),
    )
    .unwrap();
    assert_eq!(nowhere, json!({ "ok": false }));
    assert!(v.told().is_empty());
}

#[test]
fn writes_keep_the_boards_rules() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let made = ask(&mut t, &mut v, "task:create", json!({ "id": "mine", "projectName": "app", "title": "T", "branch": "b", "useWorktree": true })).unwrap();
    assert_eq!(made["task"]["useWorktree"], true);
    assert_eq!(v.read("mine")["useWorktree"], true);
    // The id is the client's, so a second task under it is refused.
    let again = ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "mine", "projectName": "app", "title": "U" }),
    )
    .unwrap();
    assert_eq!(again, json!({ "ok": false }));

    let set = json!({ "id": "mine", "set": { "status": "in_progress", "assignedSessionId": "s1", "assignedAgent": "acme", "bogus": 1 } });
    let patched = ask(&mut t, &mut v, "task:patch", set).unwrap();
    assert_eq!(patched["task"]["assignedSessionId"], "s1");
    assert_eq!(
        patched["task"]["branch"], "b",
        "a field it does not name stays"
    );

    let done = ask(
        &mut t,
        &mut v,
        "task:setStatus",
        json!({ "id": "mine", "status": "done" }),
    )
    .unwrap();
    assert_eq!(done, json!({ "ok": true }));
    assert!(v.read("mine")["completedAt"].is_string());
    let archived = ask(
        &mut t,
        &mut v,
        "task:archive",
        json!({ "id": "mine", "archived": true }),
    )
    .unwrap();
    assert_eq!(archived, json!({ "ok": true }));
    assert!(v.read("mine")["archivedAt"].is_string());
    // Reopened, both dates go.
    let reopened = ask(
        &mut t,
        &mut v,
        "task:update",
        json!({ "id": "mine", "status": "todo" }),
    )
    .unwrap();
    assert!(reopened["task"].get("completedAt").is_none(), "{reopened}");
    assert!(reopened["task"].get("archivedAt").is_none());
    // A move to another project goes to the end of its board.
    ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "o1", "projectName": "other", "title": "O" }),
    )
    .unwrap();
    let moved = ask(
        &mut t,
        &mut v,
        "task:update",
        json!({ "id": "mine", "projectName": "other" }),
    )
    .unwrap();
    assert_eq!(
        (
            moved["task"]["projectName"].as_str(),
            moved["task"]["order"].as_f64()
        ),
        (Some("other"), Some(1.0))
    );
}

#[test]
fn a_session_working_on_a_task_shows_it_on_its_card() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "t1", "projectName": "app", "title": "Fix it" }),
    )
    .unwrap();
    assert!(v.published.is_empty(), "nothing in progress yet");
    let set = json!({ "status": "in_progress", "assignedSessionId": "s1" });
    ask(
        &mut t,
        &mut v,
        "task:patch",
        json!({ "id": "t1", "set": set }),
    )
    .unwrap();
    assert_eq!(
        std::mem::take(&mut v.published),
        [Op::Upsert(
            json!({ "id": "s1", "task": "t1", "title": "Fix it" })
        )]
    );
    let badge = ask(&mut t, &mut v, tree::RENDER, json!({ "point": "task" })).unwrap();
    let rendered = tree::read(&badge).unwrap().tree;
    let mine = tree::for_session(&rendered, "s1").expect("the session's badge");
    assert!(serde_json::to_string(mine).unwrap().contains("Fix it"));
    // A change that leaves the badge as it was publishes nothing.
    ask(
        &mut t,
        &mut v,
        "task:update",
        json!({ "id": "t1", "description": "more" }),
    )
    .unwrap();
    assert!(v.published.is_empty());
    ask(
        &mut t,
        &mut v,
        "task:setStatus",
        json!({ "id": "t1", "status": "done" }),
    )
    .unwrap();
    assert_eq!(std::mem::take(&mut v.published), [Op::Delete("s1".into())]);

    // A new instance reads what was shown back from its rows.
    let mut fresh = tasks();
    let set = json!({ "status": "in_progress", "assignedSessionId": "s2" });
    ask(
        &mut fresh,
        &mut v,
        "task:patch",
        json!({ "id": "t1", "set": set }),
    )
    .unwrap();
    assert_eq!(
        v.published,
        [Op::Upsert(
            json!({ "id": "s2", "task": "t1", "title": "Fix it" })
        )]
    );
}

#[test]
fn a_conversation_an_agent_starts_is_kept_on_its_terminals_task() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    for (id, status) in [("doing", "in_progress"), ("done", "done")] {
        let task = json!({
            "id": id, "projectName": "app", "title": id, "description": "", "status": status, "order": 0,
            "assignedSessionId": "t1", "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-01-01T00:00:00.000Z",
        });
        v.store.call("dbInsertTask", json!([task])).unwrap();
    }
    let mut t = tasks();
    let event = |c: &str| Event {
        name: "agent.conversation".into(),
        data: json!({ "sessionId": "t1", "conversationId": c }),
    };
    t.on_events(&mut v, vec![event("c1")]);
    assert_eq!(v.read("doing")["agentSessionId"], "c1");
    assert!(v.read("done")["agentSessionId"].is_null());
    assert_eq!(v.told(), [["doing"]]);
    // A second conversation does not take the place of the first.
    t.on_events(&mut v, vec![event("c2")]);
    assert_eq!(v.read("doing")["agentSessionId"], "c1");
    assert!(v.told().is_empty());
}

#[test]
fn imports_a_connections_items_and_tells_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    v.store
        .call(
            "dbInsertSourceConnection",
            json!([{ "id": "c1", "connectorId": "sdk", "name": "c1", "filters": {}, "syncIntervalMinutes": 0, "statusMapping": {}, "createdAt": "2026-01-01T00:00:00.000Z" }]),
        )
        .unwrap();
    let mut t = tasks();
    let import = json!({
        "connectionId": "c1", "connectorId": "sdk", "sourceConnectorId": "acme",
        "project": "app",
        "items": [{ "externalId": "42", "title": "From outside", "externalUrl": "https://x.example/42", "status": "todo" }],
    });
    let answered = ask(&mut t, &mut v, "task.importItems", import.clone()).unwrap();
    assert_eq!(answered[0]["created"], true);
    let id = answered[0]["taskId"].as_str().unwrap().to_owned();
    assert_eq!(v.told(), [[id.clone()]]);
    let read = v.read(&id);
    assert_eq!(
        (
            read["sourceConnectorId"].as_str(),
            read["sourceExternalUrl"].as_str()
        ),
        (Some("acme"), Some("https://x.example/42"))
    );
    let again = ask(&mut t, &mut v, "task.importItems", import).unwrap();
    assert_eq!(again[0], json!({ "taskId": id, "created": false }));
    let link = ask(&mut t, &mut v, "task.sourceLink", json!({ "taskId": id })).unwrap();
    assert_eq!(
        link,
        v.store.call("dbGetTaskSourceLink", json!([id])).unwrap()
    );
    assert_eq!(link["externalId"], "42");
    assert!(ask(&mut t, &mut v, "task.importItems", json!({ "items": 1 })).is_err());

    // A task whose link was lost is found again by its own ids.
    v.store.call("dbDeleteTaskSourceLink", json!([id])).unwrap();
    let relinked = ask(&mut t, &mut v, "task.importItems", json!({
        "connectionId": "c1", "connectorId": "sdk", "sourceConnectorId": "acme", "project": "app",
        "items": [{ "externalId": "42", "title": "Renamed", "status": "todo" }],
    })).unwrap();
    assert_eq!(relinked[0], json!({ "taskId": id, "created": false }));
    assert_eq!(v.read(&id)["title"], "Renamed");
}

#[test]
fn a_workflow_takes_a_task_and_hands_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "t1", "projectName": "app", "title": "T" }),
    )
    .unwrap();
    v.changed.clear();
    let taken = ask(
        &mut t,
        &mut v,
        "task.take",
        json!({ "id": "t1", "sessionId": "s1", "agent": "claude" }),
    )
    .unwrap();
    assert_eq!(
        (taken["status"].as_str(), taken["assignedAgent"].as_str()),
        (Some("in_progress"), Some("claude"))
    );
    let back = ask(&mut t, &mut v, "task.handBack", json!({ "id": "t1" })).unwrap();
    assert_eq!(back["status"], "todo");
    assert!(back.get("assignedSessionId").is_none_or(Value::is_null));
    assert_eq!(v.fired.len(), 3, "created, taken and handed back");
    assert_eq!(
        ask(&mut t, &mut v, "task.take", json!({ "id": "ghost" })).unwrap(),
        Value::Null
    );
    assert!(ask(&mut t, &mut v, "task.take", json!({})).is_err());
}

#[test]
fn the_board_is_a_tree_whose_cards_the_client_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let answer = ask(&mut t, &mut v, tree::RENDER, json!({ "point": "board" })).unwrap();
    let board = tree::read(&answer).unwrap().tree;
    let tree::Kind::Board { cards, empty } = &board.kind else {
        panic!("a board: {board:?}")
    };
    assert_eq!(*cards, Some(tree::Cards::Tasks));
    assert_eq!(empty.as_deref(), Some("Drop tasks here"));
    let lanes: Vec<_> = board.children.iter().filter_map(|c| c.id.clone()).collect();
    assert_eq!(
        lanes,
        ["todo", "in_progress", "in_review", "done", "cancelled"]
    );
}

#[test]
fn a_client_drawing_the_board_alone_moves_and_files_cards() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    ask(
        &mut t,
        &mut v,
        "task:create",
        json!({ "id": "t1", "projectName": "app", "title": "T" }),
    )
    .unwrap();
    ask(&mut t, &mut v, "move", json!({ "id": "t1", "to": "done" })).unwrap();
    assert_eq!(v.read("t1")["status"], "done");
    ask(&mut t, &mut v, "archive", json!({ "id": "t1" })).unwrap();
    assert!(v.read("t1")["archivedAt"].is_string());
    assert_eq!(
        ask(&mut t, &mut v, "open", json!({ "id": "t1" })).unwrap(),
        Value::Null
    );
    assert_eq!(
        ask(&mut t, &mut v, "fly", json!({ "id": "t1" }))
            .unwrap_err()
            .code,
        "unknown-action"
    );
    ask(&mut t, &mut v, "delete", json!({ "id": "t1" })).unwrap();
    assert!(v.read("t1").is_null());
}

#[test]
fn keeps_a_tasks_images_in_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let name = ask(
        &mut t,
        &mut v,
        "task:imageUpload",
        json!({ "taskId": "t1", "base64": "cG5n", "filename": "a.PNG" }),
    )
    .unwrap();
    let name = name.as_str().unwrap().to_owned();
    assert_eq!(v.files[&format!("t1/{name}")], b"png");
    let src = dir.path().join("pic.jpg");
    std::fs::write(&src, b"jpeg").unwrap();
    let copied = ask(
        &mut t,
        &mut v,
        "task:imageSave",
        json!({ "taskId": "t1", "sourcePath": src }),
    )
    .unwrap();
    assert_eq!(
        v.files[&format!("t1/{}", copied.as_str().unwrap())],
        b"jpeg"
    );
    let missing = ask(
        &mut t,
        &mut v,
        "task:imageSave",
        json!({ "taskId": "t1", "sourcePath": dir.path().join("none.png") }),
    );
    assert!(missing.unwrap_err().message.contains("No such file"));
    ask(&mut t, &mut v, "task:imageCleanup", json!("t1")).unwrap();
    assert!(v.files.is_empty());
    let _ = &v.db;
}

#[test]
fn the_mcp_tools_answer_as_agents_have_always_read_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let tool = |t: &mut Tasks, v: &mut Vorn, name: &str, args: Value| -> Value {
        ask(t, v, &format!("mcp.{name}"), args).unwrap()
    };
    let text = |r: &Value| r["content"][0]["text"].as_str().unwrap().to_owned();
    let made = tool(
        &mut t,
        &mut v,
        "create_task",
        json!({ "project_name": "app", "title": "From an agent" }),
    );
    let made: Value = serde_json::from_str(&text(&made)).unwrap();
    let id = made["id"].as_str().unwrap().to_owned();
    let off = tool(
        &mut t,
        &mut v,
        "create_task",
        json!({ "project_name": "app", "title": "x", "assigned_agent": "nobody" }),
    );
    assert_eq!(off["isError"], true);
    assert!(text(&off).contains("Agents that are on: claude, codex."));
    let lost = tool(
        &mut t,
        &mut v,
        "create_task",
        json!({ "project_name": "nowhere", "title": "x" }),
    );
    assert_eq!(text(&lost), "Error: project \"nowhere\" not found");
    for (i, title) in ["B", "C"].iter().enumerate() {
        tool(
            &mut t,
            &mut v,
            "create_task",
            json!({ "project_name": "app", "title": title, "status": if i == 0 { "done" } else { "todo" } }),
        );
    }
    let placed = tool(
        &mut t,
        &mut v,
        "update_task",
        json!({ "id": id, "order": 1.5 }),
    );
    let placed: Value = serde_json::from_str(&text(&placed)).unwrap();
    assert_eq!(placed["order"].as_f64(), Some(1.0));
    let listed = tool(&mut t, &mut v, "list_tasks", json!({ "status": "done" }));
    let listed: Value = serde_json::from_str(&text(&listed)).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let refused = tool(&mut t, &mut v, "archive_task", json!({ "id": id }));
    assert!(text(&refused).starts_with("Error: only done or cancelled tasks can be archived"));
    let gone = tool(&mut t, &mut v, "delete_task", json!({ "id": id }));
    assert_eq!(text(&gone), "Deleted task: From an agent");
    let ghost = tool(&mut t, &mut v, "get_task", json!({ "id": id }));
    assert_eq!(text(&ghost), format!("Error: task \"{id}\" not found"));
    assert!(ask(&mut t, &mut v, "mcp.nope", json!({})).is_err());
}

#[test]
fn its_manifest_reads_and_draws_only_trees() {
    let init = Init::default();
    assert!(Tasks::init(&init, &mut vorn_extension_sdk::testing::FakeHost::default()).is_ok());
    assert!(MANIFEST.contains("tree = true") && !MANIFEST.contains("view ="));
    let tools: Value = serde_json::from_str(TOOLS).unwrap();
    assert_eq!(tools.as_array().unwrap().len(), 7);
    let docs: Value = serde_json::from_str(NODE_DOCS).unwrap();
    assert!(docs.get("createTaskFromItem").is_some());
}

fn pane_at(t: &mut Tasks, v: &mut Vorn, scope: &str) -> tree::Node {
    let answer = ask(
        t,
        v,
        tree::RENDER,
        json!({ "point": pane::PANE, "surface": { "scope": scope } }),
    )
    .unwrap();
    tree::read(&answer).unwrap().tree
}

/// Each entry's id under its group's heading, in order.
fn listed(pane: &tree::Node) -> Vec<(String, Vec<String>)> {
    pane.children
        .iter()
        .filter_map(|g| {
            let tree::Kind::Group { heading, .. } = &g.kind else {
                return None;
            };
            let ids = g.children.iter().filter_map(|e| e.id.clone()).collect();
            Some((heading.clone(), ids))
        })
        .collect()
}

fn pane_event(t: &mut Tasks, v: &mut Vorn, id: &str, kind: &str, value: &str) -> Value {
    ask(
        t,
        v,
        tree::EVENT,
        json!({ "point": pane::PANE, "event": { "id": id, "kind": kind, "value": value, "scope": "s-app" } }),
    )
    .unwrap()
}

#[test]
fn the_pane_beside_a_card_lists_only_its_projects_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    for (id, project) in [("a1", "app"), ("o1", "other"), ("a2", "app")] {
        let task = json!({ "id": id, "projectName": project, "title": id });
        ask(&mut t, &mut v, "task:create", task).unwrap();
    }
    ask(&mut t, &mut v, "move", json!({ "id": "a2", "to": "done" })).unwrap();

    let pane = pane_at(&mut t, &mut v, "s-app");
    assert_eq!(
        listed(&pane),
        [
            ("Todo".to_owned(), vec!["a1".to_owned()]),
            ("Done".to_owned(), vec!["a2".to_owned()]),
        ]
    );
    for session in ["s-gone", "s-loose"] {
        let unknown = pane_at(&mut t, &mut v, session);
        assert!(matches!(unknown.kind, tree::Kind::Notice { .. }));
    }
}

#[test]
fn a_task_opens_in_the_pane_and_moves_from_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut v = Vorn::new(dir.path());
    let mut t = tasks();
    let task = json!({ "id": "a1", "projectName": "app", "title": "Ship" });
    ask(&mut t, &mut v, "task:create", task).unwrap();

    pane_event(&mut t, &mut v, "a1", "toggle", "true");
    let pane = pane_at(&mut t, &mut v, "s-app");
    let row = &pane.children[0].children[0];
    let tree::Kind::Entry(e) = &row.kind else {
        panic!("an entry: {row:?}")
    };
    assert_eq!(e.expanded, Some(true));
    assert_eq!(e.code.as_deref(), Some("APP-A1"));

    let redraws = v.redraws;
    pane_event(&mut t, &mut v, "status:a1", "select", "in_review");
    assert_eq!(v.read("a1")["status"], "in_review");
    assert_eq!(v.redraws, redraws + 1, "open panes draw again");
    pane_event(&mut t, &mut v, "complete:a1", "click", "");
    assert_eq!(v.read("a1")["status"], "done");
    assert_eq!(
        pane_event(&mut t, &mut v, "focus:s9", "click", ""),
        json!({ "focus": "s9" })
    );

    pane_event(&mut t, &mut v, "a1", "toggle", "false");
    let pane = pane_at(&mut t, &mut v, "s-app");
    let tree::Kind::Entry(e) = &pane.children[0].children[0].kind else {
        panic!("an entry")
    };
    assert_eq!(e.expanded, Some(false));
}
