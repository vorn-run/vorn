//! The Tasks built-in in vornd: [`vorn_tasks::builtin`] on the public API,
//! an entry of the built-ins' registry ([`super::Builtins`]) like the
//! others. vornd starts it with its database, as its calls answer the app
//! from the first moment; the registry keeps its switch, and this module
//! hands the built-in what [`Host`] asks for and refuses its calls while it
//! is off. Off, the tasks stay where they are and the core still lists them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use tracing::info;
use vorn_extensions::api::types::{Event, RowOp};
use vorn_extensions::api::ExtError;
use vorn_extensions::events::AGENT_CONVERSATION;
use vorn_extensions::gate::{self, Caller, World};
use vorn_extensions::manifest::{file, Manifest};
use vorn_tasks::{builtin, Answer as BuiltinAnswer, Change, Host, Pending, TableWork, Tasks};

use super::{Api, Builtin};
use crate::native::task_list;
use crate::native::{Answer, Native};

/// The built-in's manifest, which it is held to.
pub fn manifest() -> Manifest {
    file::parse_builtin(builtin::MANIFEST)
        .expect("the Tasks built-in's manifest is checked by its tests")
}

/// The built-in, and whether its switch is on.
pub struct TasksBuiltin {
    inner: Arc<Tasks>,
    on: AtomicBool,
}

impl std::fmt::Debug for TasksBuiltin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TasksBuiltin")
            .field("on", &self.on)
            .finish_non_exhaustive()
    }
}

impl TasksBuiltin {
    /// The built-in on `native`, `on` as its switch was left, following the
    /// conversations agents start until vornd stops.
    pub fn new(native: &Arc<Native>, on: bool) -> Arc<TasksBuiltin> {
        let host = Arc::new(Wired {
            native: Arc::downgrade(native),
            manifest: manifest(),
        });
        let me = Arc::new(TasksBuiltin {
            inner: Tasks::new(host),
            on: AtomicBool::new(on),
        });
        follow(native, &me);
        me
    }

    /// The built-in itself, whatever its switch says.
    pub fn inner(&self) -> &Arc<Tasks> {
        &self.inner
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    fn set_on(&self, on: bool) {
        if self.on.swap(on, Ordering::AcqRel) != on {
            if on {
                info!("[tasks] switched on");
            } else {
                info!("[tasks] switched off; the tasks are kept");
                self.inner.forget_badges();
            }
        }
    }

    /// Answers one of the built-in's calls; while it is off, none.
    pub async fn answer(&self, method: &str, params: Value) -> Answer {
        if !self.is_on() {
            return Answer::Error(builtin::OFF.to_owned());
        }
        match self.inner.answer(method, params).await {
            BuiltinAnswer::Result(v) => Answer::Result(v),
            BuiltinAnswer::Void => Answer::Void,
            BuiltinAnswer::Error(e) => Answer::Error(e),
            BuiltinAnswer::Unanswered => Answer::Unanswered,
        }
    }

    /// A workflow step of a node type Tasks declares; refused while off.
    pub async fn run_node(&self, node_type: &str, request: &Value) -> Result<Value, String> {
        if !self.is_on() {
            return Err(builtin::OFF.to_owned());
        }
        self.inner
            .run_node(node_type, request)
            .await
            .ok_or_else(|| format!("Tasks runs no {node_type} steps"))
    }

    /// `task.importItems`, `task.sourceLink`, `task.take` and `task.handBack`,
    /// which the other built-ins call; refused while off.
    pub async fn api(&self, method: &str, args: Value) -> Result<Value, ExtError> {
        if !self.is_on() {
            return Err(ExtError::new("off", builtin::OFF));
        }
        let failed = |e: String| ExtError::new("failed", e);
        match method {
            "task.importItems" => self.inner.import(args).await.map_err(failed),
            "task.take" | "task.handBack" => self
                .inner
                .take_or_hand_back(method, &args)
                .await
                .map_err(failed),
            _ => {
                let task = args
                    .get("taskId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ExtError::new("bad-request", "task.sourceLink takes taskId"))?;
                self.inner
                    .source_link(task.to_owned())
                    .await
                    .map_err(failed)
            }
        }
    }

    /// The tools it adds, while it is on.
    pub fn mcp_tools(&self) -> Vec<Value> {
        if self.is_on() {
            vorn_tasks::tools::listed().to_vec()
        } else {
            Vec::new()
        }
    }

    /// An agent's call to one of its tools, while it is on.
    pub async fn mcp_call(&self, name: &str, arguments: Value) -> Option<Value> {
        if !self.is_on() {
            return None;
        }
        vorn_tasks::tools::call(&self.inner, name, arguments).await
    }
}

/// Keeps the conversation an agent starts on its terminal's task, while Tasks is on.
fn follow(native: &Arc<Native>, tasks: &Arc<TasksBuiltin>) {
    let mut events = native.events().subscribe("tasks", &[AGENT_CONVERSATION]);
    let tasks = Arc::downgrade(tasks);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let Some(tasks) = tasks.upgrade().filter(|t| t.is_on()) else {
                continue;
            };
            let text = |k: &str| event.data.get(k).and_then(Value::as_str).map(str::to_owned);
            if let (Some(terminal), Some(conversation)) =
                (text("sessionId"), text("conversationId"))
            {
                tasks.inner.keep_conversation(terminal, conversation).await;
            }
        }
    });
}

/// Tasks as an entry of the built-ins' registry.
pub fn entry(tasks: &Arc<TasksBuiltin>) -> Arc<dyn Builtin> {
    Arc::new(Entry(Arc::clone(tasks)))
}

struct Entry(Arc<TasksBuiltin>);

impl Builtin for Entry {
    fn manifest(&self) -> &'static str {
        builtin::MANIFEST
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        vorn_tasks::VIEWS
    }

    fn events(&self) -> &'static [&'static str] {
        // It follows its events itself, from vornd's start, as the board always did.
        &[]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            self.0.set_on(true);
            self.0.inner.badges().await;
        }
        .boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move { board_action(&self.0, action, args).await }.boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn switched(&self, on: bool) {
        self.0.set_on(on);
    }

    fn mcp_tools(&self) -> Vec<Value> {
        self.0.mcp_tools()
    }

    fn mcp_call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Option<Value>> {
        self.0.mcp_call(name, arguments).boxed()
    }
}

/// What a client that draws the board from its template alone asks of it:
/// a card dropped on a column, or a card's menu entry.
async fn board_action(tasks: &TasksBuiltin, action: &str, args: Value) -> Result<Value, ExtError> {
    let id = super::arg(&args, "id", action)?.to_owned();
    let status = |s: &str| serde_json::json!({ "id": id, "status": s });
    let (method, params) = match action {
        "move" => {
            let to = super::arg(&args, "to", action)?;
            ("task:setStatus", status(to))
        }
        "complete" => ("task:setStatus", status("done")),
        "reopen" => ("task:setStatus", status("todo")),
        "cancel" => ("task:setStatus", status("cancelled")),
        "archive" => (
            "task:archive",
            serde_json::json!({ "id": id, "archived": true }),
        ),
        "unarchive" => (
            "task:archive",
            serde_json::json!({ "id": id, "archived": false }),
        ),
        "delete" => ("task:delete", serde_json::json!({ "id": id })),
        // Opening a card is the client's; there is nothing to write.
        "open" => return Ok(Value::Null),
        _ => return Err(super::unknown("Tasks", action)),
    };
    match tasks.answer(method, params).await {
        Answer::Result(v) => Ok(v),
        Answer::Void | Answer::Unanswered => Ok(Value::Null),
        Answer::Error(e) => Err(ExtError::new("failed", e)),
    }
}

/// vornd, as the built-in reaches it.
struct Wired {
    native: Weak<Native>,
    manifest: Manifest,
}

fn stopping() -> String {
    "vornd is stopping".to_owned()
}

impl Host for Wired {
    fn tables(&self, work: TableWork) -> Pending<Result<(), String>> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let native = native.ok_or_else(stopping)?;
            tokio::task::spawn_blocking(move || {
                crate::native::config::with_store(&native, |store| work(store))
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn data_dir(&self) -> Option<PathBuf> {
        let native = self.native.upgrade()?;
        native
            .database()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf)
    }

    fn emit(&self, name: &str, data: Value) {
        if let Some(native) = self.native.upgrade() {
            native.events().emit(name, || data);
        }
    }

    fn changed(&self, change: Change) -> Pending<()> {
        let native = self.native.upgrade();
        Box::pin(async move {
            let Some(native) = native else { return };
            let change = match change {
                Change::Rows(rows) => task_list::Change::Rows(rows),
                Change::Removed(ids) => task_list::Change::Removed(ids),
            };
            task_list::told(&native, change).await;
        })
    }

    fn call(&self, method: &str, args: Value) -> Pending<Result<Value, String>> {
        let admitted = gate::admit_call(&self.manifest, method).map(|_| ());
        let world = crate::native::extensions::world::Wired {
            native: self.native.clone(),
        };
        let caller = Caller {
            extension: builtin::ID.to_owned(),
            project: None,
        };
        let method = method.to_owned();
        Box::pin(async move {
            admitted.map_err(|e| e.message)?;
            world
                .call(&caller, &method, args)
                .await
                .map_err(|e| e.message)
        })
    }

    fn publish(&self, collection: &str, ops: Vec<RowOp>) {
        let Some(native) = self.native.upgrade() else {
            return;
        };
        if let Some(extensions) = native.extensions() {
            let caller = Caller {
                extension: builtin::ID.to_owned(),
                project: None,
            };
            extensions.router().publish(&caller, collection, ops);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::native::config::Viewer;

    async fn ask(native: &Arc<Native>, method: &str, params: Value) -> Answer {
        native
            .answer(method.to_owned(), params, &Viewer::Local)
            .await
    }

    fn native() -> (tempfile::TempDir, Arc<Native>) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vorn.db");
        let mut store = crate::native::work::test_store(&db);
        store
            .call(
                "dbInsertProject",
                json!([{ "name": "app", "path": "/app", "preferredAgents": [] }]),
            )
            .unwrap();
        let native = Native::new();
        native.set_database(db);
        native.set_tasks(TasksBuiltin::new(&native, true));
        (dir, native)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn off_it_refuses_task_writes_and_the_core_still_lists_them() {
        let (_dir, native) = native();
        let made = ask(
            &native,
            "task:create",
            json!({ "id": "t1", "projectName": "app", "title": "T" }),
        )
        .await;
        assert!(matches!(made, Answer::Result(_)), "{made:?}");
        let tasks = native.tasks().cloned().unwrap();
        assert_eq!(tasks.mcp_tools().len(), 7);
        entry(&tasks).switched(false);
        assert_eq!(
            ask(
                &native,
                "task:setStatus",
                json!({ "id": "t1", "status": "done" })
            )
            .await,
            Answer::Error(builtin::OFF.to_owned())
        );
        let Answer::Result(read) = ask(&native, "task:get", json!({ "id": "t1" })).await else {
            panic!("the core reads a task while Tasks is off");
        };
        assert_eq!(read["title"], "T");
        assert!(tasks.mcp_tools().is_empty());
        assert!(tasks.mcp_call("list_tasks", json!({})).await.is_none());
        let refused = tasks.api("task.importItems", json!({})).await.unwrap_err();
        assert_eq!(refused.code, "off");
        let Answer::Result(listed) = ask(&native, "task:list", json!({})).await else {
            panic!("the core lists the board");
        };
        assert_eq!(listed[0]["id"], "t1");
        assert_eq!(listed[0]["status"], "todo");
        entry(&tasks).switched(true);
        assert!(matches!(
            ask(
                &native,
                "task:setStatus",
                json!({ "id": "t1", "status": "done" })
            )
            .await,
            Answer::Result(_)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_conversation_an_agent_starts_is_kept_on_its_terminals_task() {
        let (dir, native) = native();
        let mut store = crate::native::work::test_store(&dir.path().join("vorn.db"));
        let t = json!({
            "id": "doing", "projectName": "app", "title": "D", "description": "", "status": "in_progress",
            "order": 0, "assignedSessionId": "t1", "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-01-01T00:00:00.000Z",
        });
        store.call("dbInsertTask", json!([t])).unwrap();
        let reached = native.events().emit(
            AGENT_CONVERSATION,
            || json!({ "sessionId": "t1", "conversationId": "c1" }),
        );
        assert_eq!(reached, 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while store.call("dbGetTask", json!(["doing"])).unwrap()["agentSessionId"].is_null() {
            assert!(std::time::Instant::now() < deadline, "never kept");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_extension_reads_the_board_a_page_at_a_time() {
        let (_dir, native) = native();
        for id in ["a", "b", "c"] {
            ask(
                &native,
                "task:create",
                json!({ "id": id, "projectName": "app", "title": id }),
            )
            .await;
        }
        let world = crate::native::extensions::world::Wired {
            native: Arc::downgrade(&native),
        };
        let caller = Caller {
            extension: "x".into(),
            project: None,
        };
        let page = world
            .call(&caller, "task.list", json!({ "limit": 2 }))
            .await
            .unwrap();
        assert_eq!(page["tasks"].as_array().unwrap().len(), 2);
        assert_eq!(page["next"], 2);
        assert_eq!(page["tasks"][0]["description"], "");
        let agents = world.call(&caller, "agents.list", json!({})).await.unwrap();
        assert!(agents.is_array());
    }
}
