//! What the engine needs from the process it runs in.
//!
//! The engine decides; the host does. Every effect a step has (starting an
//! agent, a terminal or a script, calling a connector, writing a task) goes
//! through the host, which in vornd routes each call as a client's would be
//! routed, so a session a workflow starts is started the way a person
//! starts one; and a step an extension's node type runs by a call goes to
//! that extension through it.

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

/// The process the engine runs in. Futures are `Send` so steps of one wave
/// run on any worker.
pub trait Host: Send + Sync + 'static {
    /// The configuration (`config:load`): projects, tasks, workflows and
    /// defaults. `None` when it cannot be read.
    fn config(&self) -> impl Future<Output = Option<Value>> + Send;

    /// The tasks of `project`, of `status` when given, in the board's order:
    /// what a step working from the board reads. The configuration's by default.
    fn tasks(
        &self,
        project: &str,
        status: Option<&str>,
    ) -> impl Future<Output = Vec<Value>> + Send {
        async move {
            let config = self.config().await.unwrap_or_default();
            crate::engine::tasks_of(&config)
                .iter()
                .filter(|t| {
                    t.get("projectName").and_then(Value::as_str) == Some(project)
                        && status.is_none_or(|s| t.get("status").and_then(Value::as_str) == Some(s))
                })
                .cloned()
                .collect()
        }
    }

    /// Task `id` as it is now. The configuration's by default.
    fn task(&self, id: &str) -> impl Future<Output = Option<Value>> + Send {
        async move {
            let config = self.config().await?;
            crate::engine::tasks_of(&config)
                .iter()
                .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
        }
    }

    /// Writes a run. Called in order; the engine never waits on two.
    fn save_run(&self, run: WorkflowExecution) -> impl Future<Output = ()> + Send;

    /// Writes runs in the order given; a host with one queue takes them as one job.
    fn save_runs(&self, runs: Vec<WorkflowExecution>) -> impl Future<Output = ()> + Send {
        async move {
            for run in runs {
                self.save_run(run).await;
            }
        }
    }

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

    /// The agent a step runs when it names none and nothing assigns one:
    /// the first whose adapter is on; `None` runs a shell.
    fn first_agent(&self) -> Option<String> {
        None
    }

    /// A method of the extension API, called as the extension whose node
    /// types the engine runs in vornd: another extension's public method,
    /// as `task.take`. The error is the message the call failed with.
    fn api(&self, method: &str, args: Value) -> impl Future<Output = Result<Value, String>> + Send {
        let method = method.to_owned();
        let _ = args;
        async move { Err(format!("this host offers no {method}")) }
    }

    /// Runs a step of `node_type` by a call to `provider`, the extension
    /// that declares it: `request` is the step's node, resolved config and
    /// context, and the answer the fields of its state.
    fn run_node(
        &self,
        provider: &str,
        node_type: &str,
        request: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        let (provider, node_type) = (provider.to_owned(), node_type.to_owned());
        let _ = request;
        async move {
            Err(format!(
                "{provider} cannot be reached to run a {node_type} step"
            ))
        }
    }

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
