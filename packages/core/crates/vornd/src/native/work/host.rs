//! The workflow engine's host in vornd.
//!
//! Runs, tasks, the schedule log and the inbox are written through the work
//! model's own store ([`super::db`]). Every other effect is a call on
//! vornd's own endpoint ([`crate::mcp::Loopback`]), routed as a client's
//! is: agents, terminals and worktrees through vornd's own session and
//! worktree code, the rest (scripts, HTTP requests, connector items) to the
//! server. What sessions and scripts print comes back as the broadcasts on
//! that connection, and run progress goes out to every client through the
//! server's broadcast ([`crate::applink`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};
use tracing::{debug, warn};
use vorn_mcp::Rpc;
use vorn_protocol::WorkflowExecution;
use vorn_work::js::iso_now;
use vorn_workflow::{Completion, Host, Note, Source, TaskMove};

use super::db::Db;
use crate::mcp::Loopback;
use crate::native::Native;

/// How long a call a step makes may take: scripts and agents' starts can
/// take minutes, and their own limits end them first.
const CALL_LIMIT: Duration = Duration::from_secs(6 * 60 * 60);

/// Notes kept for a slow step before the oldest are dropped.
const NOTES_KEPT: usize = 4096;

/// The broadcasts the engine listens to on its connection.
pub const TOPICS: &[&str] = &[
    "headless:data",
    "headless:exit",
    "script:data",
    "session:updated",
    "terminal:exit",
];

pub struct VorndHost {
    pub(crate) db: Db,
    pub(crate) data_dir: PathBuf,
    pub(crate) native: Weak<Native>,
    pub(crate) loopback: Arc<Loopback>,
    pub(crate) notes: broadcast::Sender<Note>,
    /// Woken when an inbox row is settled, so the next one is delivered.
    pub(crate) drain: Arc<Notify>,
}

impl VorndHost {
    pub fn new(
        db: Db,
        data_dir: PathBuf,
        native: Weak<Native>,
        loopback: Arc<Loopback>,
        drain: Arc<Notify>,
    ) -> VorndHost {
        VorndHost {
            db,
            data_dir,
            native,
            loopback,
            notes: broadcast::channel(NOTES_KEPT).0,
            drain,
        }
    }

    /// Hands a broadcast from the connection to the steps waiting on it.
    pub fn hear(&self, frame: &Value) {
        let params = &frame["params"];
        let text = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let note = match frame["method"].as_str() {
            Some("headless:data") => Note::HeadlessData {
                id: text("id"),
                data: text("data"),
            },
            Some("headless:exit") => Note::HeadlessExit {
                id: text("id"),
                code: params
                    .get("exitCode")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0) as i64,
            },
            Some("script:data") => Note::ScriptData {
                run_id: text("runId"),
                data: text("data"),
            },
            _ => return,
        };
        let _ = self.notes.send(note);
    }

    /// Tells every client `method`.
    pub fn broadcast(&self, method: &str, params: Value) {
        if let Some(native) = self.native.upgrade() {
            native.broadcast(method, params);
        }
    }

    /// Tells the server the configuration changed, as `dbSignalChange`
    /// does: it reads it again and tells every client.
    pub fn signal_change(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        if let Err(err) = std::fs::write(self.data_dir.join(".db-signal"), now.to_string()) {
            debug!(%err, "could not signal a configuration change");
        }
    }

    /// Moves a task, as the engine's task writes do, and says where from.
    async fn move_task(
        &self,
        id: &str,
        change: impl FnOnce(&Value) -> serde_json::Map<String, Value> + Send + 'static,
    ) -> Option<TaskMove> {
        let id = id.to_owned();
        let moved = self
            .db
            .run(move |store| {
                let task = store
                    .call("dbGetTask", json!([id]))
                    .ok()
                    .filter(|t| t.is_object())?;
                let from = task["status"].as_str().unwrap_or("").to_owned();
                let updates = change(&task);
                let present: Vec<String> = updates
                    .iter()
                    .filter(|(_, v)| v.is_null())
                    .map(|(k, _)| k.clone())
                    .collect();
                store
                    .call("dbUpdateTask", json!([id, updates, present]))
                    .ok()?;
                let after = store.call("dbGetTask", json!([id])).ok()?;
                Some(TaskMove {
                    task: vorn_work::js_numbers(after),
                    from,
                })
            })
            .await
            .flatten()?;
        self.signal_change();
        Some(moved)
    }
}

/// A stored run, as the engine reads it.
fn run_of(value: Value) -> Option<WorkflowExecution> {
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value)
        .map_err(|err| warn!(%err, "a stored run could not be read"))
        .ok()
}

/// A run as clients get it: the definition is the engine's.
pub fn without_definition(run: &WorkflowExecution) -> Value {
    let mut value = serde_json::to_value(run).unwrap_or(Value::Null);
    if let Some(map) = value.as_object_mut() {
        map.remove("definition");
    }
    vorn_work::js_numbers(value)
}

impl Host for VorndHost {
    /// What the engine reads of the configuration: projects, tasks,
    /// workflows and the stored defaults. A store beside the server has none
    /// of the app's fallbacks, so `loadConfig` is not read whole.
    async fn config(&self) -> Option<Value> {
        self.db
            .run(|store| {
                let config = json!({
                    "projects": store.call("dbListProjects", json!([])).ok()?,
                    "tasks": store.call("dbListTasks", json!([null, null])).ok()?,
                    "workflows": store.call("dbListWorkflows", json!([])).ok()?,
                    "defaults": store.stored_defaults().ok()?,
                });
                Some(vorn_work::js_numbers(config))
            })
            .await
            .flatten()
    }

    async fn save_run(&self, run: WorkflowExecution) {
        let data_dir = self.data_dir.clone();
        self.db
            .run(move |store| match store.save_workflow_run(&run) {
                Ok(trimmed) => {
                    for id in trimmed {
                        vorn_work::gates::remove(&data_dir, &id);
                    }
                }
                Err(err) => warn!(%err, run = %run.run_id, "a run could not be saved"),
            })
            .await;
    }

    async fn load_run(&self, run_id: &str) -> Option<WorkflowExecution> {
        let id = run_id.to_owned();
        let value = self
            .db
            .run(move |store| store.call("getWorkflowRun", json!([id])).ok())
            .await
            .flatten()?;
        run_of(value)
    }

    async fn runs_of(&self, workflow_id: &str) -> Vec<WorkflowExecution> {
        let id = workflow_id.to_owned();
        let list = self
            .db
            .run(move |store| store.call("listWorkflowRuns", json!([id, null])).ok())
            .await
            .flatten();
        list.and_then(|l| l.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(run_of)
            .collect()
    }

    fn publish(&self, run: &WorkflowExecution) {
        self.broadcast("workflow:runUpdated", without_definition(run));
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.loopback
            .call(method, Some(params), CALL_LIMIT)
            .await
            .map_err(|e| e.0)
    }

    fn notes(&self) -> broadcast::Receiver<Note> {
        self.notes.subscribe()
    }

    async fn take_task(&self, task_id: &str, session_id: &str, agent: &str) -> Option<TaskMove> {
        let (session, agent) = (session_id.to_owned(), agent.to_owned());
        self.move_task(task_id, move |_| {
            let mut u = serde_json::Map::new();
            u.insert("status".into(), json!("in_progress"));
            u.insert("assignedSessionId".into(), json!(session));
            u.insert("assignedAgent".into(), json!(agent));
            u.insert("updatedAt".into(), json!(iso_now()));
            u.insert("archivedAt".into(), Value::Null);
            u
        })
        .await
    }

    async fn reopen_task(&self, task_id: &str) -> Option<TaskMove> {
        self.move_task(task_id, |_| {
            let mut u = serde_json::Map::new();
            u.insert("status".into(), json!("todo"));
            u.insert("updatedAt".into(), json!(iso_now()));
            u.insert("completedAt".into(), Value::Null);
            u.insert("archivedAt".into(), Value::Null);
            u.insert("assignedSessionId".into(), Value::Null);
            u.insert("assignedAgent".into(), Value::Null);
            u
        })
        .await
    }

    fn gate_view(
        &self,
        run_id: &str,
        node_id: &str,
        round: u32,
        view: &str,
    ) -> Result<String, String> {
        vorn_work::gates::publish(&self.data_dir, run_id, node_id, round, view)
    }

    fn keep_gate_page(&self, run_id: &str, node_id: &str, title: &str, round: u32) {
        let file = vorn_work::gates::file(&self.data_dir, run_id, node_id, round);
        let (data_dir, run, node, title) = (
            self.data_dir.clone(),
            run_id.to_owned(),
            node_id.to_owned(),
            title.to_owned(),
        );
        self.db.fire(move |store| {
            let kept = std::fs::read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|html| {
                    vorn_work::artifacts::publish_gate_page(
                        store, &data_dir, &run, &node, &title, &html,
                    )
                });
            if let Err(err) = kept {
                warn!(%err, gate = %node, "the review page could not be kept for comments");
            }
        });
    }

    async fn report_complete(&self, c: Completion) {
        if !matches!(c.status.as_str(), "success" | "error" | "cancelled") {
            return;
        }
        let logged = c.source == Some(Source::Scheduler) && c.status != "cancelled";
        self.db
            .run(move |store| {
                if logged {
                    let entry = json!({ "workflowId": c.workflow_id, "workflowName": c.workflow_name, "executedAt": c.completed_at, "status": c.status, "sessionsLaunched": c.sessions_launched });
                    if let Err(err) = store.call("addScheduleLogEntry", json!([entry])) {
                        warn!(%err, "the schedule log could not be written");
                    }
                }
                let _ = store.call("updateWorkflowRunStatus", json!([c.workflow_id, c.completed_at, c.status]));
            })
            .await;
        self.signal_change();
    }

    async fn complete_inbox(&self, id: i64, lease: &str, disposition: &str, error: Option<String>) {
        let (lease, disposition) = (lease.to_owned(), disposition.to_owned());
        self.db
            .run(move |store| {
                vorn_work::inbox::complete(
                    store,
                    id,
                    &lease,
                    &disposition,
                    error.as_deref(),
                    now_ms(),
                )
            })
            .await;
        self.drain.notify_one();
    }

    async fn renew_inbox(&self, id: i64, lease: &str) -> bool {
        let lease = lease.to_owned();
        self.db
            .run(move |store| vorn_work::inbox::renew(store, id, &lease, now_ms()))
            .await
            .unwrap_or(false)
    }

    async fn agent_sessions(&self) -> HashMap<String, String> {
        let Some((terminals, headless)) = self.native.upgrade().and_then(|n| n.session_records())
        else {
            return HashMap::new();
        };
        terminals
            .iter()
            .chain(&headless)
            .filter_map(|s| {
                let id = s.get("id")?.as_str()?;
                let conv = s
                    .get("agentSessionId")?
                    .as_str()
                    .filter(|c| !c.is_empty())?;
                Some((id.to_owned(), conv.to_owned()))
            })
            .collect()
    }
}

/// Unix milliseconds now.
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}
