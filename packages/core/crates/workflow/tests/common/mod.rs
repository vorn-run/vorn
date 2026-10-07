//! A host for the engine's tests: replies taken from a script, calls
//! recorded, runs kept in memory.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
use vorn_protocol::WorkflowExecution;
use vorn_workflow::{Completion, Host, Note, TaskMove};

#[derive(Default)]
pub struct World {
    pub config: Value,
    pub calls: Vec<Value>,
    pub saved: HashMap<String, WorkflowExecution>,
    pub order: Vec<String>,
    pub replies: Map<String, Value>,
    pub used: HashMap<String, Vec<bool>>,
    pub sessions: u32,
    pub pages: HashMap<(String, String, u32), String>,
}

#[derive(Clone)]
pub struct Fake {
    pub world: Arc<Mutex<World>>,
    pub notes: broadcast::Sender<Note>,
    pub data_dir: PathBuf,
}

impl Fake {
    pub fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    pub fn record(&self, method: &str, params: Value) {
        self.world()
            .calls
            .push(json!({ "method": method, "params": params }));
    }

    /// The first unused reply for `method` whose `when` matches.
    pub fn take(&self, method: &str, params: &Value) -> Option<Value> {
        let mut w = self.world();
        let list = w.replies.get(method)?.as_array()?.clone();
        let used = w
            .used
            .entry(method.to_owned())
            .or_insert_with(|| vec![false; list.len()]);
        for (i, reply) in list.iter().enumerate() {
            if used[i] {
                continue;
            }
            if let Some(Value::Object(when)) = reply.get("when") {
                if !when.iter().all(|(k, v)| params.get(k) == Some(v)) {
                    continue;
                }
            }
            used[i] = true;
            return Some(reply.clone());
        }
        None
    }

    /// Says each note after its pause, in order.
    pub fn later(&self, notes: Vec<(u64, Note)>) {
        let sender = self.notes.clone();
        tokio::spawn(async move {
            for (ms, note) in notes {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let _ = sender.send(note);
            }
        });
    }
}

fn failed(reply: &Value) -> Option<String> {
    reply
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Host for Fake {
    async fn config(&self) -> Option<Value> {
        Some(self.world().config.clone())
    }

    async fn save_run(&self, run: WorkflowExecution) {
        let mut w = self.world();
        if !w.saved.contains_key(&run.run_id) {
            w.order.push(run.run_id.clone());
        }
        w.saved.insert(run.run_id.clone(), run);
    }

    async fn load_run(&self, run_id: &str) -> Option<WorkflowExecution> {
        self.world().saved.get(run_id).cloned()
    }

    async fn runs_of(&self, workflow_id: &str) -> Vec<WorkflowExecution> {
        self.world()
            .saved
            .values()
            .filter(|r| r.workflow_id == workflow_id)
            .cloned()
            .collect()
    }

    fn publish(&self, _run: &WorkflowExecution) {}

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.record(method, params.clone());
        // An answer comes back on a later turn, as a promise's does.
        tokio::task::yield_now().await;
        match method {
            "headless:create" => {
                let reply = self
                    .take(method, &params)
                    .unwrap_or(json!({ "exitCode": 0 }));
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                let id = {
                    let mut w = self.world();
                    w.sessions += 1;
                    format!("sess-{}", w.sessions)
                };
                let mut session = json!({ "id": id, "pid": 4242, "launchCommand": "agent -p" });
                for (k, v) in reply
                    .get("session")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    session[k] = v.clone();
                }
                let code = reply.get("exitCode").and_then(Value::as_i64);
                if reply.get("exitBeforeId") == Some(&Value::Bool(true)) {
                    let _ = self.notes.send(Note::HeadlessExit {
                        id,
                        code: code.unwrap_or(0),
                    });
                } else {
                    let mut said: Vec<(u64, Note)> = reply
                        .get("output")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|data| {
                            (
                                1,
                                Note::HeadlessData {
                                    id: id.clone(),
                                    data: data.as_str().unwrap_or("").into(),
                                },
                            )
                        })
                        .collect();
                    if let Some(code) = code {
                        said.push((9, Note::HeadlessExit { id, code }));
                    }
                    self.later(said);
                }
                Ok(session)
            }
            "terminal:create" => {
                let reply = self.take(method, &params).unwrap_or(json!({}));
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                let id = {
                    let mut w = self.world();
                    w.sessions += 1;
                    format!("term-{}", w.sessions)
                };
                let mut session = json!({ "id": id });
                for (k, v) in reply
                    .get("session")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    session[k] = v.clone();
                }
                Ok(session)
            }
            "script:execute" => {
                let reply = self
                    .take(method, &params)
                    .unwrap_or(json!({ "result": { "success": true, "output": "" } }));
                let run_id = params
                    .get("runId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                for data in reply
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let _ = self.notes.send(Note::ScriptData {
                        run_id: run_id.clone(),
                        data: data.as_str().unwrap_or("").into(),
                    });
                }
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                Ok(reply["result"].clone())
            }
            "connection:executeAction" | "http:request" | "connection:upsertFromItem" => {
                let default = match method {
                    "connection:executeAction" => json!({ "result": { "success": true } }),
                    "http:request" => {
                        json!({ "result": { "success": true, "output": { "status": 200 } } })
                    }
                    _ => json!({ "result": { "taskId": "t", "created": true } }),
                };
                let reply = self.take(method, &params).unwrap_or(default);
                if let Some(e) = failed(&reply) {
                    return Err(e);
                }
                Ok(reply["result"].clone())
            }
            "sessionEvent:listBySession" => {
                let key = json!({ "sessionId": params["sessionId"] });
                Ok(self
                    .take(method, &key)
                    .map_or(json!([]), |r| r["result"].clone()))
            }
            "worktree:activeSessions" => Ok(self
                .take(method, &json!({}))
                .map_or(json!({ "count": 0, "sessionIds": [] }), |r| {
                    r["result"].clone()
                })),
            "git:worktreeDirty" => Ok(self
                .take(method, &json!({}))
                .map_or(json!(false), |r| r["result"].clone())),
            "git:removeWorktree" => Ok(json!(true)),
            _ => Ok(Value::Null),
        }
    }

    fn notes(&self) -> broadcast::Receiver<Note> {
        self.notes.subscribe()
    }

    async fn take_task(&self, task_id: &str, session_id: &str, agent: &str) -> Option<TaskMove> {
        self.record(
            "task:start",
            json!({ "id": task_id, "sessionId": session_id, "agentType": agent }),
        );
        None
    }

    async fn reopen_task(&self, task_id: &str) -> Option<TaskMove> {
        self.record("task:reopen", json!({ "id": task_id }));
        None
    }

    fn gate_view(
        &self,
        run_id: &str,
        node_id: &str,
        round: u32,
        view: &str,
    ) -> Result<String, String> {
        let token = vorn_work::gates::publish(&self.data_dir, run_id, node_id, round, view)?;
        let html = std::fs::read_to_string(vorn_work::gates::file(
            &self.data_dir,
            run_id,
            node_id,
            round,
        ))
        .unwrap_or_default();
        self.world()
            .pages
            .insert((run_id.into(), node_id.into(), round), html);
        Ok(token)
    }

    fn keep_gate_page(&self, run_id: &str, node_id: &str, title: &str, round: u32) {
        let html = self
            .world()
            .pages
            .get(&(run_id.into(), node_id.into(), round))
            .cloned()
            .unwrap_or_default();
        self.record(
            "gate:keepPage",
            json!({ "runId": run_id, "nodeId": node_id, "title": title, "html": html }),
        );
    }

    async fn report_complete(&self, c: Completion) {
        let mut p = json!({
            "workflowId": c.workflow_id, "workflowName": c.workflow_name, "completedAt": c.completed_at,
            "status": c.status, "sessionsLaunched": c.sessions_launched,
        });
        if let Some(source) = c.source {
            p["source"] = json!(source.as_str());
        }
        self.record("workflow:executionComplete", p);
    }

    async fn complete_inbox(&self, id: i64, lease: &str, disposition: &str, error: Option<String>) {
        let mut p = json!({ "id": id, "leaseToken": lease, "disposition": disposition });
        if let Some(e) = error {
            p["error"] = json!(e);
        }
        self.record("connector:inboxComplete", p);
    }

    async fn renew_inbox(&self, id: i64, lease: &str) -> bool {
        self.record(
            "connector:inboxRenew",
            json!({ "id": id, "leaseToken": lease }),
        );
        true
    }

    async fn agent_sessions(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

impl Fake {
    /// A host with `config` and `replies`, writing review pages under `data_dir`.
    pub fn new(config: Value, replies: Map<String, Value>, data_dir: PathBuf) -> Fake {
        Fake {
            world: Arc::new(Mutex::new(World {
                config,
                replies,
                ..World::default()
            })),
            notes: broadcast::channel(1024).0,
            data_dir,
        }
    }

    /// The calls made of `method`, in order.
    pub fn calls_of(&self, method: &str) -> Vec<Value> {
        self.world()
            .calls
            .iter()
            .filter(|c| c["method"] == method)
            .map(|c| c["params"].clone())
            .collect()
    }

    /// Every run kept, in the order each was first written.
    pub fn runs(&self) -> Vec<WorkflowExecution> {
        let w = self.world();
        w.order.iter().map(|id| w.saved[id].clone()).collect()
    }
}

/// A temporary directory of its own.
pub fn temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vorn-workflow-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
