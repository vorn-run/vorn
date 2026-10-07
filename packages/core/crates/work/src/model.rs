//! Workflows as the engine reads them: a definition's nodes and edges, the
//! context a run starts with, and the status each node's state is in.
//!
//! Definitions are stored as JSON a person or an agent wrote, so every field
//! is read the way JavaScript reads an object it has not checked: what is
//! missing or of another type is absent, never an error. A node's `config`
//! stays JSON and is read where its kind is known.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use vorn_protocol::{NodeExecutionState, NodeExecutionStatus, WorkflowExecution};

/// What a node does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Trigger,
    LaunchAgent,
    Script,
    Condition,
    Approval,
    CreateTaskFromItem,
    CallConnectorAction,
    HttpRequest,
    Loop,
    /// A kind this build does not know, which runs as an agent step: the
    /// engine's last branch.
    Other(String),
}

impl NodeKind {
    pub fn parse(kind: &str) -> NodeKind {
        match kind {
            "trigger" => NodeKind::Trigger,
            "launchAgent" => NodeKind::LaunchAgent,
            "script" => NodeKind::Script,
            "condition" => NodeKind::Condition,
            "approval" => NodeKind::Approval,
            "createTaskFromItem" => NodeKind::CreateTaskFromItem,
            "callConnectorAction" => NodeKind::CallConnectorAction,
            "httpRequest" => NodeKind::HttpRequest,
            "loop" => NodeKind::Loop,
            other => NodeKind::Other(other.to_owned()),
        }
    }
}

/// One step of a workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// The name later steps read its outputs by (`{{steps.<slug>.*}}`).
    pub slug: Option<String>,
    /// Whether its failure ends the run: absent is `stop`.
    pub continue_on_error: bool,
    pub config: Value,
}

impl Node {
    pub fn from_json(node: &Value) -> Option<Node> {
        let text = |key: &str| node.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Node {
            id: text("id")?,
            kind: NodeKind::parse(node.get("type").and_then(Value::as_str).unwrap_or("")),
            label: text("label").unwrap_or_default(),
            slug: text("slug").filter(|s| !s.is_empty()),
            continue_on_error: node.get("onError").and_then(Value::as_str) == Some("continue"),
            config: node.get("config").cloned().unwrap_or(Value::Null),
        })
    }

    /// A string field of the config.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Value::as_str)
    }

    /// `stopsRunOnError`: a failure ends the run unless the node opted out.
    pub fn stops_run_on_error(&self) -> bool {
        !self.continue_on_error
    }
}

/// One edge between two steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    /// The branch of a condition this edge is taken on.
    pub branch: Option<String>,
}

impl Edge {
    pub fn from_json(edge: &Value) -> Option<Edge> {
        let text = |key: &str| edge.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Edge {
            id: text("id").unwrap_or_default(),
            source: text("source")?,
            target: text("target")?,
            branch: text("conditionBranch").filter(|b| !b.is_empty()),
        })
    }
}

/// A workflow definition: its steps, and the stored JSON a run keeps as
/// its snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Milliseconds between waves.
    pub stagger_ms: Option<u64>,
    pub auto_cleanup_worktrees: bool,
    pub raw: Value,
}

impl Workflow {
    /// `None` without an id; nodes and edges JavaScript could not read are
    /// left out.
    pub fn from_json(raw: &Value) -> Option<Workflow> {
        let id = raw.get("id")?.as_str()?.to_owned();
        let list = |key: &str| raw.get(key).and_then(Value::as_array).cloned();
        Some(Workflow {
            name: raw
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            enabled: crate::is_truthy(raw.get("enabled")),
            nodes: list("nodes")
                .unwrap_or_default()
                .iter()
                .filter_map(Node::from_json)
                .collect(),
            edges: list("edges")
                .unwrap_or_default()
                .iter()
                .filter_map(Edge::from_json)
                .collect(),
            stagger_ms: raw
                .get("staggerDelayMs")
                .and_then(Value::as_f64)
                .filter(|ms| *ms > 0.0)
                .map(|ms| ms as u64),
            auto_cleanup_worktrees: crate::is_truthy(raw.get("autoCleanupWorktrees")),
            raw: raw.clone(),
            id,
        })
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The first trigger node.
    pub fn trigger(&self) -> Option<&Node> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Trigger)
    }

    /// The trigger's `triggerType`.
    pub fn trigger_type(&self) -> Option<&str> {
        self.trigger().and_then(|t| t.text("triggerType"))
    }
}

/// Where a loop's body is on a pass, for `{{loop.*}}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoopPass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Value>,
    pub index: u32,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// What a run is started with (`WorkflowExecutionContext`). The records in
/// it stay JSON: templates walk into them by any path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Context {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Value>,
    #[serde(
        rename = "connectorItem",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub connector_item: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Map<String, Value>>,
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub pass: Option<LoopPass>,
}

impl Context {
    /// A context from what a client sent: absent for `null`, and for
    /// anything that is not an object.
    pub fn from_json(value: &Value) -> Option<Context> {
        let map = value.as_object()?;
        let field = |key: &str| map.get(key).filter(|v| !v.is_null()).cloned();
        Some(Context {
            task: field("task").filter(Value::is_object),
            source: field("source").filter(Value::is_object),
            trigger: field("trigger").filter(Value::is_object),
            connector_item: field("connectorItem").filter(Value::is_object),
            inputs: field("inputs").and_then(|v| v.as_object().cloned()),
            pass: field("loop").and_then(|v| serde_json::from_value(v).ok()),
        })
    }

    /// The triggering task's id.
    pub fn task_id(&self) -> Option<&str> {
        self.task.as_ref()?.get("id")?.as_str()
    }

    /// A string field of the triggering task.
    pub fn task_text(&self, key: &str) -> Option<&str> {
        self.task.as_ref()?.get(key)?.as_str()
    }

    pub fn source_text(&self, key: &str) -> Option<&str> {
        self.source.as_ref()?.get(key)?.as_str()
    }

    pub fn item_text(&self, key: &str) -> Option<&str> {
        self.connector_item.as_ref()?.get(key)?.as_str()
    }

    pub fn trigger_text(&self, key: &str) -> Option<&str> {
        self.trigger.as_ref()?.get(key)?.as_str()
    }
}

/// A node state's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Pending,
    Running,
    Success,
    Error,
    Skipped,
    Waiting,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Success => "success",
            Status::Error => "error",
            Status::Skipped => "skipped",
            Status::Waiting => "waiting",
        }
    }

    /// `None` for a status no build writes.
    pub fn parse(text: &str) -> Option<Status> {
        Some(match text {
            "pending" => Status::Pending,
            "running" => Status::Running,
            "success" => Status::Success,
            "error" => Status::Error,
            "skipped" => Status::Skipped,
            "waiting" => Status::Waiting,
            _ => return None,
        })
    }

    /// Finished one way or another: success, error or skipped.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Success | Status::Error | Status::Skipped)
    }
}

impl From<Status> for NodeExecutionStatus {
    fn from(status: Status) -> Self {
        NodeExecutionStatus(status.as_str().to_owned())
    }
}

/// Reading and setting a node state's status as a [`Status`].
pub trait StateExt {
    /// `None` for a status no build writes.
    fn status(&self) -> Option<Status>;
    fn is(&self, status: Status) -> bool {
        self.status() == Some(status)
    }
    fn set_status(&mut self, status: Status);
}

impl StateExt for NodeExecutionState {
    fn status(&self) -> Option<Status> {
        Status::parse(&self.status.0)
    }

    fn set_status(&mut self, status: Status) {
        self.status = status.into();
    }
}

/// A node state with only its id and status.
pub fn state(node_id: &str, status: Status) -> NodeExecutionState {
    NodeExecutionState {
        node_id: node_id.to_owned(),
        status: status.into(),
        agent_session_id: None,
        agent_type: None,
        approved_at: None,
        completed_at: None,
        diagnostics: None,
        editable_text: None,
        edited_text: None,
        error: None,
        feedback: None,
        iteration: None,
        logs: None,
        message: None,
        output: None,
        project_name: None,
        project_path: None,
        rejected_at: None,
        round: None,
        session_id: None,
        skip_reason: None,
        started_at: None,
        structured_output: None,
        task_id: None,
        view_token: None,
        waiting_for: None,
        worktree_name: None,
        worktree_origin: None,
        worktree_path: None,
    }
}

/// Finding a node's state in a run.
pub trait RunExt {
    fn node_state(&self, node_id: &str) -> Option<&NodeExecutionState>;
    fn node_state_mut(&mut self, node_id: &str) -> Option<&mut NodeExecutionState>;
    /// `updateNodeState`: changes the node's state when the run has one.
    fn update(&mut self, node_id: &str, change: impl FnOnce(&mut NodeExecutionState)) {
        if let Some(state) = self.node_state_mut(node_id) {
            change(state);
        }
    }
    /// Whether the run is still going.
    fn is_running(&self) -> bool;
}

impl RunExt for WorkflowExecution {
    fn node_state(&self, node_id: &str) -> Option<&NodeExecutionState> {
        self.node_states.iter().find(|s| s.node_id == node_id)
    }

    fn node_state_mut(&mut self, node_id: &str) -> Option<&mut NodeExecutionState> {
        self.node_states.iter_mut().find(|s| s.node_id == node_id)
    }

    fn is_running(&self) -> bool {
        self.status == "running"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_definition_as_javascript_would() {
        let wf = Workflow::from_json(&json!({
            "id": "w", "name": "W", "enabled": 1, "staggerDelayMs": 250,
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Start", "config": { "triggerType": "manual" } },
                { "id": "a", "type": "launchAgent", "label": "Do", "slug": "do", "onError": "continue", "config": {} },
                { "type": "script" },
                { "id": "x", "type": "teleport", "slug": "" }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "a", "conditionBranch": "true" }, { "source": "t" }]
        }))
        .unwrap();
        assert!(wf.enabled);
        assert_eq!(wf.stagger_ms, Some(250));
        assert_eq!(wf.nodes.len(), 3);
        assert_eq!(wf.trigger_type(), Some("manual"));
        assert!(!wf.node("a").unwrap().stops_run_on_error());
        assert_eq!(
            wf.node("x").unwrap().kind,
            NodeKind::Other("teleport".into())
        );
        assert_eq!(wf.node("x").unwrap().slug, None);
        assert_eq!(wf.edges.len(), 1);
        assert_eq!(wf.edges[0].branch.as_deref(), Some("true"));
        assert!(Workflow::from_json(&json!({ "name": "no id" })).is_none());
    }

    #[test]
    fn a_context_keeps_only_what_has_a_shape() {
        let ctx = Context::from_json(&json!({
            "task": { "id": "t1" }, "source": null, "inputs": { "a": 1 }, "loop": { "index": 0, "number": 1 }
        }))
        .unwrap();
        assert_eq!(ctx.task_id(), Some("t1"));
        assert!(ctx.source.is_none());
        assert_eq!(ctx.pass.as_ref().map(|p| p.number), Some(1));
        assert!(Context::from_json(&json!(null)).is_none());
        assert!(Context::from_json(&json!("x")).is_none());
    }

    #[test]
    fn statuses_read_and_write_through_the_stored_string() {
        let mut s = state("n", Status::Pending);
        assert!(s.is(Status::Pending));
        s.set_status(Status::Waiting);
        assert_eq!(s.status.0, "waiting");
        assert!(Status::Skipped.is_terminal());
        assert!(!Status::Waiting.is_terminal());
        assert_eq!(Status::parse("odd"), None);
    }
}
