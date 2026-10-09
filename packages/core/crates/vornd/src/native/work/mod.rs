//! The work model, run by vornd: workflows and their runs, the scheduler,
//! webhooks and artifacts (the `workflow`, `workflowRun`, `scheduler`,
//! `webhook` and `artifact` groups).
//!
//! The engine ([`vorn_workflow`]) walks runs through [`host::VorndHost`];
//! the scheduler and the connector inbox ([`schedule`]) start them; the
//! pages a browser opens (an artifact, a gate's review page, a webhook) are
//! answered over HTTP ([`routes`]). Reads come from `vorn.db` as the server
//! answered them ([`vorn_work::reads`]). Every client hears about runs and
//! artifacts through the server's broadcast, since the server holds every
//! client; what it no longer does is run any of this.

pub mod db;
pub mod host;
pub mod routes;
pub mod schedule;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use jiff::tz::TimeZone;
use serde_json::{json, Map, Value};
use tokio::sync::Notify;
use tracing::{info, warn};
use vorn_mcp::Rpc;
use vorn_work::artifacts::{self as service, Publisher, Queue};
use vorn_work::model::{Context, Workflow};
use vorn_work::reads::{self, Reply};
use vorn_workflow::{workflows_of, Answer as GateAnswer, Decision, Engine, Options, Source};

use self::db::Db;
use self::host::{now_ms, without_definition, VorndHost};
use super::{Answer, Native};
use crate::mcp::Loopback;

/// The groups the work model answers.
pub const GROUPS: &[&str] = &[
    "workflow",
    "workflowRun",
    "scheduler",
    "webhook",
    "artifact",
];

impl std::fmt::Debug for Work {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Work")
            .field("active_runs", &self.engine.active_count())
            .finish_non_exhaustive()
    }
}

/// Whether `method` is the work model's.
pub fn is_work(method: &str) -> bool {
    GROUPS.contains(&crate::groups::group_of(method))
}

/// Every call the work model answers.
pub const METHODS: &[&str] = &[
    "workflow:list",
    "workflow:get",
    "workflow:create",
    "workflow:update",
    "workflow:delete",
    "workflow:setEnabled",
    "workflow:run",
    "workflow:retryRun",
    "workflow:rerun",
    "workflow:runManual",
    "workflow:stopRun",
    "workflow:resolveGate",
    "workflow:sessionRestored",
    "workflow:executionComplete",
    "workflowRun:save",
    "workflowRun:list",
    "workflowRun:listByTask",
    "workflowRun:listWaiting",
    "workflowRun:listRunning",
    "workflowRun:listAll",
    "workflowRun:claim",
    "workflowRun:release",
    "scheduler:getLog",
    "scheduler:getNextRun",
    "webhook:info",
    "artifact:publish",
    "artifact:list",
    "artifact:get",
    "artifact:versionUrl",
    "artifact:forGate",
    "artifact:readComments",
    "artifact:saveComment",
    "artifact:updateComment",
    "artifact:deleteComment",
    "artifact:send",
    "artifact:readSource",
    "artifact:saveUserVersion",
];

/// Starts and ends a bracketed paste.
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// Long enough for a prompt to take the paste before Enter lands.
const SUBMIT_DELAY: Duration = Duration::from_millis(80);

/// How often the connection the engine listens on is checked.
const KEEP_OPEN: Duration = Duration::from_secs(2);

pub struct Work {
    pub(crate) engine: Engine<VorndHost>,
    pub(crate) db: Db,
    native: Weak<Native>,
    /// Woken when an inbox row is added or settled.
    pub(crate) drain: Arc<Notify>,
    /// Woken when the workflows changed, so the scheduler reads them again.
    pub(crate) rearm: Notify,
    /// Connector polls in flight, by workflow.
    pub(crate) polls: Mutex<HashSet<String>>,
    /// Review batches waiting for their agent's prompt.
    queue: Mutex<Queue>,
}

fn param<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    params.get(key).filter(|v| !v.is_null())
}

fn text(params: &Value, key: &str) -> Option<String> {
    param(params, key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The JSON a store call answers, or the error a handler would throw.
fn stored(value: Option<Result<Value, String>>) -> Answer {
    match value {
        Some(Ok(v)) => Answer::Result(v),
        Some(Err(e)) => Answer::Error(e),
        None => Answer::Error("the database is not available".into()),
    }
}

impl Work {
    /// The work model on `native`'s database, reaching the endpoint through
    /// `loopback`.
    pub fn new(native: &Arc<Native>, db: Db, loopback: Arc<Loopback>) -> Arc<Work> {
        let data_dir = db
            .path()
            .parent()
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        let drain = Arc::new(Notify::new());
        let host = VorndHost::new(
            db.clone(),
            data_dir,
            Arc::downgrade(native),
            loopback,
            Arc::clone(&drain),
        );
        Arc::new(Work {
            engine: Engine::new(host),
            db,
            native: Arc::downgrade(native),
            drain,
            rearm: Notify::new(),
            polls: Mutex::default(),
            queue: Mutex::default(),
        })
    }

    pub(crate) fn host(&self) -> &VorndHost {
        self.engine.host()
    }

    /// Starts the scheduler, the inbox, the connection the engine listens
    /// on, and picks up what the last vornd left: gates waiting and runs
    /// still marked running.
    pub fn start(self: &Arc<Self>) {
        tokio::spawn(Arc::clone(self).listen());
        tokio::spawn(Arc::clone(self).schedule());
        tokio::spawn(Arc::clone(self).deliver_inbox());
        let work = Arc::clone(self);
        tokio::spawn(async move { work.resume().await });
    }

    /// Keeps the engine's connection open and hands on what it hears.
    async fn listen(self: Arc<Self>) {
        let mut heard = self.host().loopback.subscribe();
        let mut check = tokio::time::interval(KEEP_OPEN);
        loop {
            tokio::select! {
                _ = check.tick() => {
                    if let Err(err) = self.host().loopback.connect().await {
                        tracing::debug!(err = %err.0, "the work model's connection is not open");
                    }
                }
                frame = heard.recv() => match frame {
                    Ok(frame) => self.hear(&frame).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => warn!(n, "the work model missed broadcasts"),
                    Err(_) => return,
                },
            }
        }
    }

    async fn hear(self: &Arc<Self>, frame: &Value) {
        self.host().hear(frame);
        let id = frame["params"]["id"].as_str().unwrap_or("").to_owned();
        if matches!(
            frame["method"].as_str(),
            Some("terminal:exit" | "headless:exit")
        ) {
            if let Some(native) = self.native.upgrade() {
                let code = frame["params"]
                    .get("exitCode")
                    .cloned()
                    .unwrap_or(Value::Null);
                super::tasks::log_event(&native, &id, "exited", Some(json!({ "exitCode": code })));
            }
        }
        match frame["method"].as_str() {
            Some("session:updated") => self.session_status_changed(&id).await,
            Some("terminal:exit") => {
                let gone = self
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .forget_session(&id);
                for artifact in gone {
                    self.comments_changed(&artifact);
                }
            }
            _ => {}
        }
    }

    /// `resumeRunsAfterStart`, and the sweeps of pages and artifacts no
    /// longer kept.
    async fn resume(&self) {
        let data_dir = self.host().data_dir.clone();
        let found = self
            .db
            .run(move |s| {
                let workflows = s.call("dbListWorkflows", json!([])).ok()?;
                let waiting = s.call("listRunsWithWaitingGates", json!([null])).ok()?;
                let running = s.call("listRunningRuns", json!([])).ok()?;
                let kept: Vec<String> = s.list_workflow_run_ids().unwrap_or_default();
                vorn_work::gates::sweep(&data_dir, kept.iter().map(String::as_str));
                service::sweep(s, &data_dir, now_ms());
                Some((workflows, waiting, running))
            })
            .await
            .flatten();
        let Some((workflows, waiting, running)) = found else {
            return;
        };
        let workflows: Vec<Workflow> = workflows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Workflow::from_json)
            .collect();
        if workflows.is_empty() {
            return;
        }
        let runs = |v: Value| -> Vec<vorn_protocol::WorkflowExecution> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| serde_json::from_value(r.clone()).ok())
                .collect()
        };
        let (waiting, running) = (runs(waiting), runs(running));
        if !waiting.is_empty() {
            info!(count = waiting.len(), "re-arming gates left waiting");
            self.engine.rearm_gates(waiting, &workflows).await;
        }
        if !running.is_empty() {
            info!(count = running.len(), "reconciling runs left running");
            self.engine.reconcile(running, &workflows).await;
        }
    }

    /// Answers `method` with `params`: the work model's calls, and the
    /// connector inbox's, whose leases are its.
    pub async fn answer(self: &Arc<Self>, method: &str, params: &Value) -> Answer {
        use vorn_workflow::Host;
        let id = params.get("id").and_then(Value::as_f64).map(|id| id as i64);
        let lease = text(params, "leaseToken");
        match (method, id, lease) {
            ("connector:inboxComplete", Some(id), Some(lease)) => {
                let disposition = text(params, "disposition").unwrap_or_else(|| "retry".into());
                self.host()
                    .complete_inbox(id, &lease, &disposition, text(params, "error"))
                    .await;
                Answer::Void
            }
            ("connector:inboxRenew", Some(id), Some(lease)) => {
                Answer::Result(Value::Bool(self.host().renew_inbox(id, &lease).await))
            }
            ("connector:inboxComplete" | "connector:inboxRenew", _, _) => {
                Answer::Error(format!("{method} needs an id and a leaseToken"))
            }
            _ => self.call(method, params).await,
        }
    }

    /// Answers one of the work model's own calls.
    pub async fn call(self: &Arc<Self>, method: &str, params: &Value) -> Answer {
        match crate::groups::group_of(method) {
            "artifact" => self.artifact(method, params).await,
            _ => self.workflow(method, params).await,
        }
    }

    /// What the server's process knew that the database does not.
    fn read_host(&self) -> (PathBuf, Option<u16>) {
        (
            self.host().data_dir.clone(),
            self.native.upgrade().and_then(|n| n.server_port()),
        )
    }

    /// A read the database answers as the server's handler did.
    async fn read(&self, method: &str, params: &Value) -> Answer {
        let (data_dir, port) = self.read_host();
        let (method, params) = (method.to_owned(), params.clone());
        let reply = self
            .db
            .run(move |store| {
                let zone = TimeZone::system();
                let host = reads::Host {
                    data_dir: &data_dir,
                    server_port: port,
                    now_ms: now_ms(),
                    zone: &zone,
                };
                reads::read(store, &host, &method, &params)
            })
            .await;
        match reply {
            Some(Reply::Value(v)) => Answer::Result(v),
            Some(Reply::Void) => Answer::Void,
            _ => Answer::Error("vornd could not read the database".into()),
        }
    }

    async fn workflow_row(&self, id: &str) -> Option<Value> {
        let id = id.to_owned();
        self.db
            .run(move |s| {
                s.call("dbGetWorkflow", json!([id]))
                    .ok()
                    .map(vorn_work::js_numbers)
            })
            .await
            .flatten()
            .filter(Value::is_object)
    }

    async fn stored_run(&self, id: &str) -> Option<vorn_protocol::WorkflowExecution> {
        self.engine.flush().await;
        let id = id.to_owned();
        let value = self
            .db
            .run(move |s| s.call("getWorkflowRun", json!([id])).ok())
            .await
            .flatten()?;
        serde_json::from_value(value).ok()
    }

    /// The workflows changed: clients are told, and the scheduler reads
    /// them again.
    fn workflows_changed(&self) {
        self.host().signal_change();
        self.rearm.notify_one();
    }

    async fn workflow(self: &Arc<Self>, method: &str, params: &Value) -> Answer {
        match method {
            "workflow:list"
            | "workflow:get"
            | "workflowRun:list"
            | "workflowRun:listByTask"
            | "workflowRun:listWaiting"
            | "workflowRun:listRunning"
            | "workflowRun:listAll"
            | "scheduler:getLog"
            | "scheduler:getNextRun"
            | "webhook:info" => self.read(method, params).await,
            "workflow:create" => {
                let Some(workflow) = param(params, "workflow").cloned().filter(Value::is_object)
                else {
                    return Answer::Error("workflow:create needs a workflow".into());
                };
                let row = workflow.clone();
                let made = self
                    .db
                    .run(move |s| {
                        s.call("dbInsertWorkflow", json!([row]))
                            .map_err(|e| e.to_string())
                    })
                    .await;
                match made {
                    Some(Ok(_)) => {
                        self.workflows_changed();
                        Answer::Result(workflow)
                    }
                    other => stored(other),
                }
            }
            "workflow:update" => {
                let (Some(id), Some(updates)) =
                    (text(params, "id"), param(params, "updates").cloned())
                else {
                    return Answer::Error("workflow:update needs an id and updates".into());
                };
                let changed = self
                    .db
                    .run(move |s| {
                        let n = s
                            .call("dbUpdateWorkflow", json!([id, updates]))
                            .map_err(|e| e.to_string())?;
                        let changed = n.as_f64().unwrap_or(0.0) > 0.0;
                        // Nothing to set is still an update of a workflow that is there.
                        let there = changed
                            || s.call("dbGetWorkflow", json!([id]))
                                .is_ok_and(|w| w.is_object());
                        Ok::<_, String>((changed, there))
                    })
                    .await;
                match changed {
                    Some(Ok((changed, there))) => {
                        if changed {
                            self.workflows_changed();
                        }
                        Answer::Result(json!({ "ok": there }))
                    }
                    Some(Err(e)) => Answer::Error(e),
                    None => stored(None),
                }
            }
            "workflow:delete" => {
                let Some(id) = text(params, "id") else {
                    return Answer::Error("workflow:delete needs an id".into());
                };
                let gone = self
                    .db
                    .run(move |s| {
                        let had = s
                            .call("dbGetWorkflow", json!([id]))
                            .ok()
                            .is_some_and(|w| w.is_object());
                        s.call("dbDeleteWorkflow", json!([id]))
                            .map(|_| had)
                            .map_err(|e| e.to_string())
                    })
                    .await;
                match gone {
                    Some(Ok(had)) => {
                        if had {
                            self.workflows_changed();
                        }
                        Answer::Result(json!({ "ok": had }))
                    }
                    Some(Err(e)) => Answer::Error(e),
                    None => stored(None),
                }
            }
            "workflow:setEnabled" => {
                let id = text(params, "id").unwrap_or_default();
                let enabled = params.get("enabled").cloned().unwrap_or(Value::Null);
                let changed = self
                    .db
                    .run(move |s| {
                        s.call("dbUpdateWorkflow", json!([id, { "enabled": enabled }]))
                            .map_err(|e| e.to_string())
                    })
                    .await;
                match changed {
                    Some(Ok(n)) if n.as_f64().unwrap_or(0.0) > 0.0 => {
                        self.workflows_changed();
                        Answer::Result(json!({ "ok": true }))
                    }
                    Some(Ok(_)) => Answer::Result(json!({ "ok": false })),
                    other => stored(other),
                }
            }
            "workflow:run" => {
                let Some(workflow) = self.workflow_named(params, "workflowId").await else {
                    return Answer::Result(Value::Null);
                };
                let context = params.get("context").and_then(Context::from_json);
                let options = Options {
                    source: Some(Source::Manual),
                    target: text(params, "targetNodeId"),
                };
                started(self.engine.execute(&workflow, context, options).await)
            }
            "workflow:retryRun" | "workflow:rerun" => {
                let Some(run) = self
                    .stored_run(&text(params, "runId").unwrap_or_default())
                    .await
                else {
                    return Answer::Result(Value::Null);
                };
                let Some(workflow) = self
                    .workflow_row(&run.workflow_id)
                    .await
                    .and_then(|w| Workflow::from_json(&w))
                else {
                    return Answer::Result(Value::Null);
                };
                if method == "workflow:retryRun" {
                    started(self.engine.retry(&workflow, &run).await)
                } else {
                    started(self.engine.rerun(&workflow, &run).await)
                }
            }
            "workflow:runManual" => {
                let id = text(params, "workflowId").unwrap_or_default();
                let Some(row) = self.workflow_row(&id).await else {
                    return Answer::Error(format!("Workflow {id} not found"));
                };
                let workflow = Workflow::from_json(&row);
                if workflow.as_ref().is_none_or(|w| w.trigger().is_none()) {
                    let name = row["name"].as_str().unwrap_or("");
                    return Answer::Error(format!(
                        "Workflow \"{name}\" has no trigger; add one before running it"
                    ));
                }
                let inputs = param(params, "inputs").and_then(Value::as_object).cloned();
                let work = Arc::clone(self);
                tokio::spawn(async move { work.fire(&id, inputs).await });
                Answer::Void
            }
            "workflow:stopRun" => {
                let id = text(params, "runId").unwrap_or_default();
                let engine = self.engine.clone();
                tokio::spawn(async move { engine.stop(&id).await });
                Answer::Void
            }
            "workflow:resolveGate" => self.resolve_gate(params).await,
            "workflow:sessionRestored" => {
                let id = text(params, "sessionId").unwrap_or_default();
                let session = self
                    .native
                    .upgrade()
                    .and_then(|n| n.session_records())
                    .and_then(|(terminals, _)| {
                        terminals.into_iter().find(|t| t["id"] == id.as_str())
                    });
                if let Some(session) = session {
                    let restore = text(params, "restore").unwrap_or_else(|| "cold".into());
                    let environment = param(params, "environment").cloned();
                    self.engine
                        .fire_session_restored(&session, &restore, environment)
                        .await;
                }
                Answer::Void
            }
            "workflow:executionComplete" => {
                use vorn_workflow::Host;
                let status = text(params, "status").unwrap_or_default();
                let source = match params.get("source").and_then(Value::as_str) {
                    Some("scheduler") => Some(Source::Scheduler),
                    Some("manual") => Some(Source::Manual),
                    _ => None,
                };
                self.host()
                    .report_complete(vorn_workflow::Completion {
                        workflow_id: text(params, "workflowId").unwrap_or_default(),
                        workflow_name: text(params, "workflowName").unwrap_or_default(),
                        completed_at: text(params, "completedAt").unwrap_or_default(),
                        status,
                        sessions_launched: params
                            .get("sessionsLaunched")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as usize,
                        source,
                    })
                    .await;
                Answer::Void
            }
            "workflowRun:save" => {
                let Ok(run) =
                    serde_json::from_value::<vorn_protocol::WorkflowExecution>(params.clone())
                else {
                    return Answer::Error("workflowRun:save needs a run".into());
                };
                use vorn_workflow::Host;
                self.host().save_run(run).await;
                Answer::Void
            }
            "workflowRun:claim" => {
                let claim = self.engine.claim(
                    &text(params, "workflowId").unwrap_or_default(),
                    text(params, "params").as_deref(),
                    params
                        .get("windowMs")
                        .and_then(Value::as_f64)
                        .map(|ms| ms as i64),
                );
                Answer::Result(json!({ "granted": claim.granted, "runId": claim.run_id }))
            }
            "workflowRun:release" => {
                self.engine.release(
                    &text(params, "workflowId").unwrap_or_default(),
                    text(params, "params").as_deref(),
                    &text(params, "runId").unwrap_or_default(),
                );
                Answer::Void
            }
            _ => Answer::Error(format!("Method not found: {method}")),
        }
    }

    async fn workflow_named(&self, params: &Value, key: &str) -> Option<Workflow> {
        let id = text(params, key)?;
        Workflow::from_json(&self.workflow_row(&id).await?)
    }

    /// `workflow:resolveGate`: refuses what the gate cannot take, at once,
    /// and acts on the rest where the run is.
    async fn resolve_gate(self: &Arc<Self>, params: &Value) -> Answer {
        let run_id = text(params, "runId").unwrap_or_default();
        let node_id = text(params, "nodeId").unwrap_or_default();
        let raw = text(params, "decision").unwrap_or_default();
        let comment = text(params, "comment");
        let edited = text(params, "edited");
        let Some(decision) = Decision::parse(&raw) else {
            return Answer::Result(json!({ "accepted": true }));
        };
        if decision == Decision::Approve && self.engine.waits_for_sign_in(&run_id, &node_id).await {
            return Answer::Result(json!({ "accepted": false }));
        }
        let pinned = match decision {
            Decision::Changes => Some(match param(params, "comments") {
                Some(c) => c.clone(),
                None => {
                    let (run, node) = (run_id.clone(), node_id.clone());
                    self.db
                        .run(move |s| service::gate_draft_comments(s, &run, &node))
                        .await
                        .unwrap_or(json!([]))
                }
            }),
            _ => None,
        };
        if decision == Decision::Changes {
            let comments = pinned.clone().unwrap_or(Value::Null);
            if !self
                .engine
                .takes_changes(
                    &run_id,
                    &node_id,
                    comment.as_deref().unwrap_or(""),
                    &comments,
                )
                .await
            {
                return Answer::Result(json!({ "accepted": false }));
            }
        }
        if decision != Decision::Reject {
            if let Some(reason) = self
                .engine
                .edit_refusal(&run_id, &node_id, edited.as_deref())
                .await
            {
                return Answer::Result(json!({ "accepted": false, "reason": reason }));
            }
        }
        info!(run = %run_id, node = %node_id, decision = %raw, "a gate was answered");
        let engine = self.engine.clone();
        let (run, node) = (run_id.clone(), node_id.clone());
        let answer = GateAnswer {
            decision,
            comment,
            edited,
            comments: pinned,
        };
        tokio::spawn(async move { engine.apply_gate_decision(&run, &node, answer).await });
        if decision == Decision::Changes {
            let (run, node) = (run_id.clone(), node_id.clone());
            let sent = self
                .db
                .run(move |s| {
                    let artifact = s.call("findGateArtifact", json!([run, node])).ok()?;
                    let id = artifact.get("id")?.as_str()?.to_owned();
                    let sent = s.call("sendArtifactDrafts", json!([id])).ok()?;
                    (!sent.is_null()).then_some(id)
                })
                .await
                .flatten();
            if let Some(id) = sent {
                self.comments_changed(&id);
            }
        }
        self.host().broadcast(
            "workflow:gateResolved",
            json!({ "runId": run_id, "nodeId": node_id, "decision": raw }),
        );
        Answer::Result(json!({ "accepted": true }))
    }

    fn comments_changed(&self, artifact_id: &str) {
        self.host().broadcast(
            "artifact:commentsChanged",
            json!({ "artifactId": artifact_id }),
        );
    }

    /// `callingSession`: the terminal an agent publishes from.
    fn calling_session(&self, id: &str) -> Result<Publisher, String> {
        self.native
            .upgrade()
            .and_then(|n| n.session_records())
            .and_then(|(terminals, _)| terminals.into_iter().find(|t| t["id"] == id))
            .map(|t| Publisher::of_session(&t))
            .ok_or_else(|| format!("Session not found: {id}"))
    }

    /// The terminal record of a session, as the copy holds it now.
    fn terminal(&self, id: &str) -> Option<Value> {
        self.native
            .upgrade()?
            .session_records()?
            .0
            .into_iter()
            .find(|t| t["id"] == id)
    }

    async fn artifact_row(&self, id: &str) -> Option<Value> {
        let id = id.to_owned();
        self.db
            .run(move |s| service::call(s, "getArtifact", json!([id])).ok())
            .await
            .flatten()
            .filter(Value::is_object)
    }

    /// `visibleTo`: the artifact, when the session may see it.
    async fn visible(&self, session: &str, artifact: &str) -> Result<Value, String> {
        let me = self.calling_session(session)?;
        match self.artifact_row(artifact).await {
            Some(a) if service::can_see(&a, &me) => Ok(a),
            _ => Err(format!(
                "No artifact {artifact} in this session or project."
            )),
        }
    }

    fn loopback_url(&self, path: &str) -> String {
        let port = self
            .native
            .upgrade()
            .and_then(|n| n.server_port())
            .unwrap_or(0);
        format!("http://127.0.0.1:{port}{path}")
    }

    async fn artifact(self: &Arc<Self>, method: &str, params: &Value) -> Answer {
        let data_dir = self.host().data_dir.clone();
        match method {
            "artifact:versionUrl" | "artifact:forGate" => self.read(method, params).await,
            "artifact:publish" => {
                let session_id = text(params, "sessionId").unwrap_or_default();
                let me = match self.calling_session(&session_id) {
                    Ok(me) => me,
                    Err(e) => return Answer::Error(e),
                };
                let request = params.clone();
                let outcome = self
                    .db
                    .run(move |s| service::publish(s, &data_dir, &me, &request))
                    .await;
                let outcome = match outcome {
                    Some(Ok(o)) => o,
                    Some(Err(e)) => return Answer::Error(e),
                    None => return stored(None),
                };
                let url = self.loopback_url(outcome["path"].as_str().unwrap_or(""));
                self.host().broadcast(
                    "artifact:published",
                    json!({ "artifact": outcome["artifact"], "version": outcome["version"] }),
                );
                let mut opened = false;
                if params.get("open") != Some(&Value::Bool(false)) {
                    let a = &outcome["artifact"];
                    let pane = json!({
                        "sessionId": session_id, "url": url,
                        "artifact": { "id": a["id"], "version": outcome["version"]["version"], "kind": a["kind"], "title": a["title"] }
                    });
                    match self
                        .host()
                        .loopback
                        .call("browser:openPane", Some(pane), Duration::from_secs(30))
                        .await
                    {
                        Ok(_) => opened = true,
                        Err(err) => info!(err = %err.0, "published without opening the pane"),
                    }
                }
                Answer::Result(json!({
                    "artifact": outcome["artifact"], "version": outcome["version"], "url": url,
                    "answered": outcome["answered"], "opened": opened
                }))
            }
            "artifact:list" => {
                let limit = params.get("limit").cloned().unwrap_or(Value::Null);
                let filter = match text(params, "sessionId").filter(|s| !s.is_empty()) {
                    Some(session) => match self.calling_session(&session) {
                        Ok(me) => {
                            let mut f = Map::new();
                            f.insert("sessionId".into(), json!(session));
                            if let Some(p) = me.project_name {
                                f.insert("projectName".into(), json!(p));
                            }
                            Value::Object(f)
                        }
                        Err(e) => return Answer::Error(e),
                    },
                    None => {
                        let mut f = Map::new();
                        if let Some(p) = params.get("projectName") {
                            f.insert("projectName".into(), p.clone());
                        }
                        Value::Object(f)
                    }
                };
                stored(
                    self.db
                        .run(move |s| service::call(s, "listArtifacts", json!([filter, limit])))
                        .await,
                )
            }
            "artifact:get" => {
                let id = text(params, "artifactId").unwrap_or_default();
                let queued = self
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_queued(&id);
                let got = self
                    .db
                    .run(move |s| {
                        let artifact = service::call(s, "getArtifact", json!([id]))?;
                        if artifact.is_null() {
                            return Ok(Value::Null);
                        }
                        Ok(json!({
                            "artifact": artifact,
                            "versions": service::call(s, "listArtifactVersions", json!([id]))?,
                            "comments": service::call(s, "listArtifactComments", json!([id, {}]))?,
                            "queued": queued,
                        }))
                    })
                    .await;
                stored(got)
            }
            "artifact:readComments" => {
                let id = text(params, "artifactId").unwrap_or_default();
                if let Err(e) = self
                    .visible(&text(params, "sessionId").unwrap_or_default(), &id)
                    .await
                {
                    return Answer::Error(e);
                }
                let filter = match param(params, "version") {
                    Some(v) => json!({ "version": v }),
                    None => json!({}),
                };
                stored(
                    self.db
                        .run(move |s| service::call(s, "listArtifactComments", json!([id, filter])))
                        .await,
                )
            }
            "artifact:saveComment" => {
                let id = text(params, "artifactId").unwrap_or_default();
                let version = params.get("version").and_then(Value::as_f64).unwrap_or(0.0);
                let Some(artifact) = self.artifact_row(&id).await else {
                    return Answer::Error(format!("Artifact not found: {id}"));
                };
                if version < 1.0 || version > artifact["latestVersion"].as_f64().unwrap_or(0.0) {
                    return Answer::Error(format!(
                        "Artifact {id} has no version {}",
                        vorn_work::js::to_string(&params["version"])
                    ));
                }
                let fields = json!({ "artifactId": id, "version": params["version"], "anchor": params.get("anchor").cloned().unwrap_or(Value::Null), "body": params.get("body").cloned().unwrap_or(json!("")) });
                let saved = self
                    .db
                    .run(move |s| service::call(s, "insertArtifactComment", json!([fields])))
                    .await;
                if matches!(saved, Some(Ok(_))) {
                    self.comments_changed(&id);
                }
                stored(saved)
            }
            "artifact:updateComment" => {
                let id = text(params, "commentId").unwrap_or_default();
                let mut change = Map::new();
                for key in ["body", "anchor"] {
                    if let Some(v) = params.get(key) {
                        change.insert(key.into(), v.clone());
                    }
                }
                let updated = self
                    .db
                    .run(move |s| service::call(s, "updateArtifactComment", json!([id, change])))
                    .await;
                if let Some(Ok(comment)) = &updated {
                    if let Some(a) = comment.get("artifactId").and_then(Value::as_str) {
                        self.comments_changed(a);
                    }
                }
                stored(updated)
            }
            "artifact:deleteComment" => {
                let id = text(params, "commentId").unwrap_or_default();
                let gone = self
                    .db
                    .run(move |s| {
                        let artifact = service::call(s, "getArtifactComment", json!([id]))
                            .ok()
                            .and_then(|c| {
                                c.get("artifactId")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            });
                        let deleted = service::call(s, "deleteArtifactComment", json!([id]))?
                            .as_bool()
                            .unwrap_or(false);
                        Ok::<_, String>((deleted, artifact))
                    })
                    .await;
                match gone {
                    Some(Ok((deleted, artifact))) => {
                        if let (true, Some(a)) = (deleted, artifact) {
                            self.comments_changed(&a);
                        }
                        Answer::Result(json!({ "deleted": deleted }))
                    }
                    Some(Err(e)) => Answer::Error(e),
                    None => stored(None),
                }
            }
            "artifact:send" => match self
                .send(&text(params, "artifactId").unwrap_or_default())
                .await
            {
                Ok(v) => Answer::Result(v),
                Err(e) => Answer::Error(e),
            },
            "artifact:readSource" => {
                let id = text(params, "artifactId").unwrap_or_default();
                if let Some(session) = text(params, "sessionId").filter(|s| !s.is_empty()) {
                    if let Err(e) = self.visible(&session, &id).await {
                        return Answer::Error(e);
                    }
                }
                let version = params
                    .get("version")
                    .and_then(Value::as_f64)
                    .map(|v| v as u32);
                stored(
                    self.db
                        .run(move |s| Ok(service::read_source(s, &data_dir, &id, version)))
                        .await,
                )
            }
            "artifact:saveUserVersion" => {
                let id = text(params, "artifactId").unwrap_or_default();
                let body = text(params, "body").unwrap_or_default();
                let edits = params.get("edits").cloned().unwrap_or(json!([]));
                let a = id.clone();
                let saved = self
                    .db
                    .run(move |s| {
                        let saved = service::save_user_version(s, &data_dir, &a, &body, &edits)?;
                        Ok::<_, String>((saved, service::call(s, "getArtifact", json!([a]))?))
                    })
                    .await;
                let (saved, artifact) = match saved {
                    Some(Ok(pair)) => pair,
                    Some(Err(e)) => return Answer::Error(e),
                    None => return stored(None),
                };
                let version = saved["version"].clone();
                self.host().broadcast(
                    "artifact:published",
                    json!({ "artifact": artifact, "version": version }),
                );
                self.comments_changed(&id);
                if params.get("send") != Some(&Value::Bool(true))
                    && !vorn_work::is_truthy(params.get("send"))
                {
                    return Answer::Result(json!({ "version": version, "sent": null }));
                }
                match self.send(&id).await {
                    Ok(sent) => Answer::Result(json!({ "version": version, "sent": sent })),
                    Err(e) => {
                        Answer::Result(json!({ "version": version, "sent": null, "sendError": e }))
                    }
                }
            }
            _ => Answer::Error(format!("Method not found: {method}")),
        }
    }

    /// `delivery.send`: the artifact's drafts as one batch to the agent that
    /// published it, at once at its prompt, or held until it is.
    async fn send(self: &Arc<Self>, artifact_id: &str) -> Result<Value, String> {
        let artifact = self
            .artifact_row(artifact_id)
            .await
            .ok_or_else(|| format!("Artifact not found: {artifact_id}"))?;
        let session_id = artifact["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("No session published this artifact.")?
            .to_owned();
        let session = self
            .terminal(&session_id)
            .ok_or("The session that published this artifact has ended.")?;
        let id = artifact_id.to_owned();
        let drafts = self
            .db
            .run(move |s| {
                service::call(s, "listArtifactComments", json!([id, { "state": "draft" }])).ok()
            })
            .await
            .flatten()
            .and_then(|d| d.as_array().map(Vec::len))
            .unwrap_or(0);
        if drafts == 0 {
            return Ok(json!({ "state": "empty", "count": 0 }));
        }
        if !service::at_prompt(&session) {
            self.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .hold(artifact_id, &session_id);
            self.comments_changed(artifact_id);
            return Ok(json!({ "state": "queued", "count": 0 }));
        }
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drop_artifact(artifact_id);
        let count = self.deliver(artifact_id, &session_id).await;
        self.comments_changed(artifact_id);
        Ok(json!({ "state": if count > 0 { "delivered" } else { "empty" }, "count": count }))
    }

    /// Pastes the artifact's drafts into the session as one message, then
    /// submits it.
    async fn deliver(&self, artifact_id: &str, session_id: &str) -> usize {
        let id = artifact_id.to_owned();
        let batch = self
            .db
            .run(move |s| {
                let sent = service::call(s, "sendArtifactDrafts", json!([id])).ok()?;
                let comments = sent.get("comments")?.as_array()?.clone();
                let artifact = service::call(s, "getArtifact", json!([id])).ok()?;
                let versions = service::call(s, "listArtifactVersions", json!([id])).ok()?;
                let author = versions
                    .as_array()?
                    .last()?
                    .get("author")?
                    .as_str()
                    .map(str::to_owned);
                Some((
                    service::feedback_message(&artifact, &comments, author.as_deref()),
                    comments.len(),
                ))
            })
            .await
            .flatten();
        let Some((message, count)) = batch else {
            return 0;
        };
        let loopback = Arc::clone(&self.host().loopback);
        let write = |data: String| {
            let loopback = Arc::clone(&loopback);
            let session = session_id.to_owned();
            async move {
                let _ = loopback
                    .notify(
                        "terminal:write",
                        Some(json!({ "id": session, "data": data })),
                    )
                    .await;
            }
        };
        write(format!("{PASTE_START}{message}{PASTE_END}")).await;
        let submit = write("\r".to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(SUBMIT_DELAY).await;
            submit.await;
        });
        count
    }

    /// A session's status moved: what waited for its prompt is handed over,
    /// one paste per turn.
    async fn session_status_changed(&self, session_id: &str) {
        let Some(session) = self.terminal(session_id) else {
            return;
        };
        if !service::at_prompt(&session) {
            return;
        }
        let next = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .next_for(session_id);
        if let Some(artifact) = next {
            self.deliver(&artifact, session_id).await;
            self.comments_changed(&artifact);
        }
    }

    /// The page of an artifact version, for whoever holds its token.
    pub async fn artifact_page(&self, id: &str, version: u32, token: &str) -> Option<String> {
        let (data_dir, id, token) = (
            self.host().data_dir.clone(),
            id.to_owned(),
            token.to_owned(),
        );
        self.db
            .run(move |s| service::page(s, &data_dir, &id, version, &token))
            .await
            .flatten()
    }

    /// The file of a gate's review page, when the token is this round's and
    /// the gate still asks.
    pub async fn gate_page(&self, run_id: &str, node_id: &str, token: &str) -> Option<PathBuf> {
        let round = self.engine.gate_page_round(run_id, node_id, token).await?;
        Some(vorn_work::gates::file(
            &self.host().data_dir,
            run_id,
            node_id,
            round,
        ))
    }

    /// A webhook request for a workflow.
    pub async fn webhook(
        &self,
        workflow_id: &str,
        token: &str,
        request: vorn_work::inbox::Request,
    ) -> vorn_work::inbox::Received {
        let (wf, token) = (workflow_id.to_owned(), token.to_owned());
        let event = uuid::Uuid::new_v4().to_string();
        let received = self
            .db
            .run(move |s| {
                vorn_work::inbox::receive_webhook(s, &wf, &token, &request, &event, now_ms())
            })
            .await
            .unwrap_or(vorn_work::inbox::Received::NotFound);
        if received == vorn_work::inbox::Received::Queued {
            info!(workflow = %workflow_id, "a webhook event was queued");
            self.drain.notify_one();
        }
        received
    }

    /// A call a client connected to the server made, which the server
    /// hands here: the work model's, or any other vornd answers.
    pub async fn relayed(self: &Arc<Self>, method: String, params: Value) -> Answer {
        if is_work(&method) {
            return self.answer(&method, &params).await;
        }
        match self.native.upgrade() {
            Some(native) => {
                native
                    .answer(method, params, &super::config::Viewer::Local)
                    .await
            }
            None => Answer::Forward,
        }
    }

    /// A workflow trigger a configuration save fires: a task created or moved. It
    /// is received once by its `effectId`, however often it is delivered.
    pub async fn trigger(&self, params: &Value) -> Result<bool, String> {
        let effect = text(params, "effectId").ok_or("a trigger needs an effectId")?;
        let task = param(params, "task")
            .cloned()
            .filter(Value::is_object)
            .ok_or("a trigger needs a task")?;
        let first = self
            .db
            .run(move |s| vorn_work::receipts::receive(s, &effect, now_ms()))
            .await
            == Some(vorn_work::receipts::Delivery::First);
        if !first {
            return Ok(false);
        }
        match text(params, "kind").as_deref() {
            Some("taskCreated") => self.engine.fire_task_created(&task).await,
            Some("taskStatusChanged") => {
                let from = text(params, "from").unwrap_or_default();
                let to = text(params, "to").unwrap_or_default();
                self.engine
                    .fire_task_status_changed(&task, &from, &to)
                    .await;
            }
            other => warn!(kind = ?other, "a trigger of a kind vornd does not know"),
        }
        Ok(true)
    }

    /// A connection signed in again: the steps waiting for it run again.
    pub async fn signed_in(&self, connection_id: &str) {
        let parked = self
            .db
            .run(|s| s.call("listRunsWithWaitingGates", json!(["signIn"])).ok())
            .await
            .flatten();
        let runs = parked
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| serde_json::from_value(r.clone()).ok())
            .collect();
        self.engine.resume_sign_in_waits(connection_id, runs).await;
    }

    /// The workflows changed elsewhere: the scheduler reads them again.
    pub fn workflows_changed_elsewhere(&self) {
        self.rearm.notify_one();
    }

    /// How many schedules are armed, which keeps an idle server up.
    pub async fn armed_schedules(&self) -> usize {
        self.db
            .run(|s| {
                let list = s.call("dbListWorkflows", json!([])).ok()?;
                let zone = TimeZone::system();
                let workflows = workflows_of(&json!({ "workflows": list }));
                Some(
                    workflows
                        .iter()
                        .filter(|w| {
                            w.enabled
                                && vorn_work::schedule::Schedule::arm(
                                    std::slice::from_ref(&w.raw),
                                    &zone,
                                    now_ms(),
                                )
                                .arms(&w.id)
                        })
                        .count(),
                )
            })
            .await
            .flatten()
            .unwrap_or(0)
    }
}

/// `startedRun`: the run as it stood when it started, or `null` when it
/// did not start.
fn started(result: Result<vorn_workflow::Started, String>) -> Answer {
    match result {
        Ok(started) => Answer::Result(without_definition(&started.run)),
        Err(err) => {
            warn!(%err, "a run did not start");
            Answer::Result(Value::Null)
        }
    }
}

#[cfg(test)]
pub(crate) fn test_store(path: &std::path::Path) -> vorn_store::Store {
    let options = vorn_store::StoreOptions {
        default_shell: String::new(),
        default_agent_commands: Map::new(),
        default_workspace: serde_json::from_value(json!({ "id": "personal", "name": "Personal", "icon": "User", "iconColor": "#6b7280", "order": 0 })).expect("a workspace"),
        owner_name: "owner".into(),
        seed_workflows: Vec::new(),
    };
    vorn_store::Store::open(path, options).expect("a store").0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    /// A work model on a fresh database, its endpoint nowhere: steps that
    /// call out fail, the rest run. The native side is kept beside it.
    fn work(dir: &std::path::Path) -> (Arc<Native>, Arc<Work>) {
        let db_path = dir.join("vorn.db");
        drop(test_store(&db_path));
        let native = Native::new();
        native.set_database(db_path.clone());
        let db = Db::open(&db_path).expect("the database opens");
        let nowhere: SocketAddr = "127.0.0.1:9".parse().expect("an address");
        let work = Work::new(&native, db, Arc::new(Loopback::new(nowhere, b"token")));
        native.set_work(Arc::clone(&work));
        (native, work)
    }

    fn conditional(id: &str, trigger: Value) -> Value {
        json!({
            "id": id, "name": id, "icon": "x", "iconColor": "#000", "enabled": true,
            "nodes": [
                { "id": "t", "type": "trigger", "label": "T", "position": { "x": 0, "y": 0 }, "config": trigger },
                { "id": "c", "type": "condition", "label": "C", "slug": "c", "position": { "x": 0, "y": 0 }, "config": { "variable": "{{trigger.body.n}}{{task.title}}", "operator": "isNotEmpty", "value": "" } }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "c" }]
        })
    }

    async fn runs(work: &Arc<Work>, id: &str) -> Vec<Value> {
        match work
            .call("workflowRun:list", &json!({ "workflowId": id }))
            .await
        {
            Answer::Result(Value::Array(list)) => list,
            other => panic!("{other:?}"),
        }
    }

    async fn settled(work: &Arc<Work>, id: &str, count: usize) -> Vec<Value> {
        for _ in 0..200 {
            let list = runs(work, id).await;
            if list.len() >= count && list.iter().all(|r| r["status"] != "running") {
                return list;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "no {count} finished run(s) of {id}: {:?}",
            runs(work, id).await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_webhook_delivered_twice_runs_once() {
        let dir = tempfile::tempdir().unwrap();
        let (_native, work) = work(dir.path());
        work.start();
        let wf = conditional(
            "hook",
            json!({ "triggerType": "webhook", "method": "POST", "token": "tok" }),
        );
        assert!(matches!(
            work.call("workflow:create", &json!({ "workflow": wf }))
                .await,
            Answer::Result(_)
        ));
        let request = |key: &str| vorn_work::inbox::Request {
            method: "POST".into(),
            body: json!({ "n": 7 }),
            delivery_id: Some(key.into()),
            ..Default::default()
        };
        assert_eq!(
            work.webhook("hook", "tok", request("d1")).await,
            vorn_work::inbox::Received::Queued
        );
        assert_eq!(
            work.webhook("hook", "tok", request("d1")).await,
            vorn_work::inbox::Received::Repeat
        );
        let done = settled(&work, "hook", 1).await;
        assert_eq!(done[0]["status"], "success");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(runs(&work, "hook").await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_trigger_delivered_twice_starts_its_workflow_once() {
        let dir = tempfile::tempdir().unwrap();
        let (_native, work) = work(dir.path());
        let wf = conditional(
            "moved",
            json!({ "triggerType": "taskStatusChanged", "toStatus": "in_progress" }),
        );
        work.call("workflow:create", &json!({ "workflow": wf }))
            .await;
        let trigger = json!({
            "effectId": "task-status/t1/todo/in_progress/now", "kind": "taskStatusChanged",
            "task": { "id": "t1", "title": "Moved", "projectName": "p" }, "from": "todo", "to": "in_progress"
        });
        assert_eq!(work.trigger(&trigger).await, Ok(true));
        assert_eq!(work.trigger(&trigger).await, Ok(false));
        let done = settled(&work, "moved", 1).await;
        assert_eq!(done[0]["triggerTaskId"], "t1");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(runs(&work, "moved").await.len(), 1);
        assert!(work
            .trigger(&json!({ "kind": "taskCreated" }))
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_to_a_missing_workflow_is_not_ok() {
        let dir = tempfile::tempdir().unwrap();
        let (_native, work) = work(dir.path());
        let wf = conditional("w", json!({ "triggerType": "manual" }));
        work.call("workflow:create", &json!({ "workflow": wf }))
            .await;
        let answer = |a: Answer| match a {
            Answer::Result(v) => v,
            other => panic!("{other:?}"),
        };
        let update = |id: &str, updates: Value| json!({ "id": id, "updates": updates });
        for (params, ok) in [
            (update("w", json!({ "name": "Renamed" })), true),
            (update("w", json!({})), true),
            (update("gone", json!({ "name": "x" })), false),
        ] {
            let got = answer(work.call("workflow:update", &params).await);
            assert_eq!(got, json!({ "ok": ok }), "{params}");
        }
        for ok in [true, false] {
            let got = answer(work.call("workflow:delete", &json!({ "id": "w" })).await);
            assert_eq!(got, json!({ "ok": ok }));
        }
    }
}
