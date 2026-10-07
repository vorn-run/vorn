//! What the engine needs from the process it runs in.
//!
//! The engine decides; the host does. Every effect a step has (starting an
//! agent, a terminal or a script, calling a connector, writing a task) goes
//! through the host, which in vornd routes each call as a client's would be
//! routed, so a session a workflow starts is started the way a person
//! starts one.

use std::collections::HashMap;
use std::future::Future;

use serde_json::Value;
use tokio::sync::broadcast;
use vorn_protocol::WorkflowExecution;

/// What a session or script says while a step waits on it.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    /// A headless agent printed `data`.
    HeadlessData { id: String, data: String },
    /// A headless agent ended.
    HeadlessExit { id: String, code: i64 },
    /// A script started under `run_id` printed `data`.
    ScriptData { run_id: String, data: String },
}

/// Who started a run, which the schedule log records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Scheduler,
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Scheduler => "scheduler",
            Source::Manual => "manual",
        }
    }
}

/// A run that ended, for the workflow's last-run badge and the schedule log
/// (`workflow:executionComplete`).
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub workflow_id: String,
    pub workflow_name: String,
    pub completed_at: String,
    /// `success`, `error` or `cancelled`.
    pub status: String,
    pub sessions_launched: usize,
    pub source: Option<Source>,
}

/// A task a step took on or handed back, as it is now and the status it
/// moved from, so the workflows watching for that move can start.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskMove {
    pub task: Value,
    pub from: String,
}

/// The process the engine runs in. Futures are `Send` so steps of one wave
/// run on any worker.
pub trait Host: Send + Sync + 'static {
    /// The configuration (`config:load`): projects, tasks, workflows and
    /// defaults. `None` when it cannot be read.
    fn config(&self) -> impl Future<Output = Option<Value>> + Send;

    /// Writes a run. Called in order; the engine never waits on two.
    fn save_run(&self, run: WorkflowExecution) -> impl Future<Output = ()> + Send;

    /// A stored run.
    fn load_run(&self, run_id: &str) -> impl Future<Output = Option<WorkflowExecution>> + Send;

    /// A workflow's stored runs.
    fn runs_of(&self, workflow_id: &str) -> impl Future<Output = Vec<WorkflowExecution>> + Send;

    /// Tells every client how a run stands (`workflow:runUpdated`).
    fn publish(&self, run: &WorkflowExecution);

    /// A call as a client sends it: `headless:create`, `script:execute`,
    /// `connection:executeAction` and the rest. The error is the message
    /// the call failed with.
    fn call(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// What sessions and scripts say from now on.
    fn notes(&self) -> broadcast::Receiver<Note>;

    /// Marks a task in progress for a step's session.
    fn take_task(
        &self,
        task_id: &str,
        session_id: &str,
        agent: &str,
    ) -> impl Future<Output = Option<TaskMove>> + Send;

    /// Hands a task back to the queue, unassigned.
    fn reopen_task(&self, task_id: &str) -> impl Future<Output = Option<TaskMove>> + Send;

    /// Keeps a gate's review page for this round: the token that opens it,
    /// or why there is none.
    fn gate_view(
        &self,
        run_id: &str,
        node_id: &str,
        round: u32,
        view: &str,
    ) -> Result<String, String>;

    /// Keeps the round's review page as a version of the gate's artifact.
    fn keep_gate_page(&self, run_id: &str, node_id: &str, title: &str, round: u32);

    /// The run ended.
    fn report_complete(&self, completion: Completion) -> impl Future<Output = ()> + Send;

    /// Settles the connector inbox row a run was delivered with.
    fn complete_inbox(
        &self,
        id: i64,
        lease: &str,
        disposition: &str,
        error: Option<String>,
    ) -> impl Future<Output = ()> + Send;

    /// Extends an inbox row's lease; false once the row is no longer this
    /// run's.
    fn renew_inbox(&self, id: i64, lease: &str) -> impl Future<Output = bool> + Send;

    /// The agent conversations of the sessions running now, by session id.
    fn agent_sessions(&self) -> impl Future<Output = HashMap<String, String>> + Send;
}
