//! A step run in vornd by the extension that declares its node type, with
//! the run at hand: what it reads (its node, the workflow, the context and
//! the outputs before it) and what it may do to the run (change its own
//! state, write it, say it out loud, watch for a stop).

use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::WaitForCancellationFuture;
use vorn_protocol::{NodeExecutionState, WorkflowExecution};
use vorn_work::model::{Context, Node, RunExt, Workflow};
use vorn_work::template::StepOutputs;

use crate::engine::{lock, Active, Engine, Run};
use crate::host::Host;
use crate::waves::BoxFuture;

/// A node type run in vornd. Its step was marked running and written
/// before [`Step::run`]; the step ends by setting its own outcome
/// ([`StepCx::finish`]) or by waiting, and an `Err` fails it with the message.
pub trait Step<H: Host>: Send + Sync + 'static {
    fn run<'a>(&'a self, cx: StepCx<'a, H>) -> BoxFuture<'a, Result<(), String>>;

    /// Whether this step made the worktree its state names, so a workflow
    /// that cleans up after itself removes it when the run ends.
    fn made_worktree(&self, _node: &Node) -> bool {
        false
    }
}

/// One step's run, as its node type sees it.
pub struct StepCx<'a, H: Host> {
    pub(crate) engine: &'a Engine<H>,
    pub node: &'a Node,
    pub workflow: &'a Workflow,
    pub(crate) run: &'a Run,
    pub context: Option<&'a Context>,
    pub outputs: &'a StepOutputs,
    pub(crate) active: &'a Arc<Active>,
}

impl<'a, H: Host> StepCx<'a, H> {
    /// The process the engine runs in.
    pub fn host(&self) -> &'a H {
        self.engine.host()
    }

    /// Changes this step's state. The run's lock is held only for `change`.
    pub fn update(&self, change: impl FnOnce(&mut NodeExecutionState)) {
        lock(self.run).update(&self.node.id, change);
    }

    /// Changes this step's state and stamps it complete, then writes the run.
    pub fn finish(&self, change: impl FnOnce(&mut NodeExecutionState)) {
        self.engine.finish(self.run, self.node, change);
    }

    /// Reads the run as it stands.
    pub fn read<T>(&self, f: impl FnOnce(&WorkflowExecution) -> T) -> T {
        f(&lock(self.run))
    }

    /// Tells clients how the run stands, and writes it.
    pub fn persist(&self) {
        self.engine.persist(self.run);
    }

    /// Tells clients how the run stands; output alone at most every few seconds.
    pub fn publish(&self) {
        self.engine.publish(self.run);
    }

    /// Writes the run without telling anyone.
    pub fn save(&self) {
        self.engine.save(self.run);
    }

    /// The configuration the engine reads: projects, workflows and defaults.
    pub async fn config(&self) -> Option<Value> {
        self.engine.config().await
    }

    /// Resolves when the run is stopped.
    pub fn stopped(&self) -> WaitForCancellationFuture<'a> {
        self.active.cancel.cancelled()
    }

    /// Whether the run was stopped.
    pub fn is_stopped(&self) -> bool {
        self.active.cancel.is_cancelled()
    }

    /// A session this step started, which a stop ends.
    pub fn track_session(&self, session_id: &str) {
        self.active
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session_id.to_owned());
    }
}
