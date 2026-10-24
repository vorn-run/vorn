//! The approval gates waiting for a person, as public events: each time a
//! run's waiting gates change, `workflow.gates` says which wait now, so the
//! Workflows extension lists them beside agents asking. A run
//! with no gate, now or before, costs a scan of its states and nothing more.
//! A run seen running that ends is told once as `workflow.ended`, and a step
//! that starts waiting for a sign-in as `workflow.signIn`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::{json, Value};
use vorn_protocol::{NodeExecutionState, WorkflowExecution};

pub use vorn_extensions::events::{
    WORKFLOW_CHANGED, WORKFLOW_ENDED, WORKFLOW_GATES, WORKFLOW_SIGN_IN,
};

/// The id of a gate's row: its run and its step.
fn gate_row_id(run: &str, node: &str) -> String {
    format!("{run}|{node}")
}

/// Each step's label in a workflow's `nodes`, by id.
pub fn labels(nodes: Option<&Value>) -> BTreeMap<String, String> {
    nodes
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| {
            Some((
                n.get("id")?.as_str()?.to_owned(),
                n.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            ))
        })
        .collect()
}

fn is_gate(s: &NodeExecutionState) -> bool {
    s.status.0 == "waiting" && s.waiting_for.as_deref() != Some("signIn")
}

/// Whether a gate of `run` asks now.
fn asking(run: &WorkflowExecution) -> bool {
    run.status == "running" && run.node_states.iter().any(is_gate)
}

/// The gates of `run` asking now, as rows: its workflow is `name`, its steps labelled `labels`.
pub fn gate_rows(
    run: &WorkflowExecution,
    name: &str,
    labels: &BTreeMap<String, String>,
) -> Vec<Value> {
    if run.status != "running" {
        return Vec::new();
    }
    run.node_states
        .iter()
        .filter(|s| is_gate(s))
        .map(|s| {
            let label = labels
                .get(&s.node_id)
                .filter(|l| !l.is_empty())
                .cloned()
                .unwrap_or_else(|| "Approval".to_owned());
            json!({
                "id": gate_row_id(&run.run_id, &s.node_id),
                "title": label,
                "subtitle": name,
                "message": s.message,
                "workflow": run.workflow_id,
                "run": run.run_id,
                "node": s.node_id,
                "at": s.started_at,
            })
        })
        .collect()
}

/// The gate rows of `run`, named and labelled from its own definition.
fn rows_of(run: &WorkflowExecution) -> Vec<Value> {
    let name = run
        .definition
        .as_ref()
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let labelled = labels(run.definition.as_ref().and_then(|d| d.get("nodes")));
    gate_rows(run, name, &labelled)
}

/// The name its own definition gives `run`'s workflow.
fn workflow_name(run: &WorkflowExecution) -> &str {
    run.definition
        .as_ref()
        .and_then(|d| d.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The waiting gates told last, by run; the runs seen running; the steps
/// told waiting for a sign-in, by run.
#[derive(Debug, Default)]
pub struct Waiting {
    told: Mutex<BTreeMap<String, Vec<String>>>,
    running: Mutex<BTreeSet<String>>,
    signing: Mutex<BTreeMap<String, BTreeSet<String>>>,
}

impl Waiting {
    /// `run` moved: what to tell when its waiting gates changed, `{runId, gates}`.
    pub fn changed(&self, run: &WorkflowExecution) -> Option<Value> {
        let mut told = self.told.lock().unwrap_or_else(|e| e.into_inner());
        if !asking(run) && !told.contains_key(&run.run_id) {
            return None;
        }
        let rows = rows_of(run);
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let before = if ids.is_empty() {
            told.remove(&run.run_id).unwrap_or_default()
        } else {
            told.insert(run.run_id.clone(), ids.clone())
                .unwrap_or_default()
        };
        (before != ids).then(|| json!({ "runId": run.run_id, "gates": rows }))
    }

    /// `run` moved: what to tell when a run seen running just ended.
    pub fn ended(&self, run: &WorkflowExecution) -> Option<Value> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if run.status == "running" {
            running.insert(run.run_id.clone());
            return None;
        }
        if !matches!(run.status.as_str(), "success" | "error") || !running.remove(&run.run_id) {
            return None;
        }
        let rejected = run
            .node_states
            .iter()
            .any(|s| s.status.0 == "error" && s.rejected_at.is_some());
        let outcome = match run.status.as_str() {
            "success" => "completed",
            _ if rejected => "rejected",
            _ => "failed",
        };
        let steps = run
            .node_states
            .iter()
            .filter(|s| s.status.0 != "pending")
            .count();
        Some(json!({
            "runId": run.run_id,
            "workflowId": run.workflow_id,
            "workflow": workflow_name(run),
            "outcome": outcome,
            "steps": steps,
        }))
    }

    /// `run` moved: each step that just started waiting for a sign-in.
    pub fn signing_in(&self, run: &WorkflowExecution) -> Vec<Value> {
        let mut signing = self.signing.lock().unwrap_or_else(|e| e.into_inner());
        let now: BTreeSet<String> = run
            .node_states
            .iter()
            .filter(|s| {
                run.status == "running"
                    && s.status.0 == "waiting"
                    && s.waiting_for.as_deref() == Some("signIn")
            })
            .map(|s| s.node_id.clone())
            .collect();
        if now.is_empty() {
            signing.remove(&run.run_id);
            return Vec::new();
        }
        let before = signing
            .insert(run.run_id.clone(), now.clone())
            .unwrap_or_default();
        let labelled = labels(run.definition.as_ref().and_then(|d| d.get("nodes")));
        now.difference(&before)
            .map(|node| {
                let state = run.node_states.iter().find(|s| &s.node_id == node);
                json!({
                    "runId": run.run_id,
                    "workflowId": run.workflow_id,
                    "workflow": workflow_name(run),
                    "node": node,
                    "title": labelled.get(node).filter(|l| !l.is_empty()),
                    "message": state.and_then(|s| s.error.clone()),
                })
            })
            .collect()
    }
}

/// Every gate waiting in `waiting` (runs as `workflowRun:listWaiting` gives
/// them), named and labelled from `workflows` (`workflow:list`).
pub fn all_rows(waiting: &[Value], workflows: &[Value]) -> Vec<Value> {
    let named: BTreeMap<&str, (&str, BTreeMap<String, String>)> = workflows
        .iter()
        .filter_map(|w| {
            Some((
                w.get("id")?.as_str()?,
                (w.get("name")?.as_str()?, labels(w.get("nodes"))),
            ))
        })
        .collect();
    let mut out = Vec::new();
    for r in waiting {
        let Ok(run) = serde_json::from_value::<WorkflowExecution>(r.clone()) else {
            continue;
        };
        let (name, labelled) = named
            .get(run.workflow_id.as_str())
            .cloned()
            .unwrap_or_default();
        out.extend(gate_rows(&run, name, &labelled));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: &str, waiting: bool) -> WorkflowExecution {
        serde_json::from_value(json!({
            "runId": "r1", "workflowId": "w1", "status": status, "startedAt": "t",
            "definition": { "name": "Ship", "nodes": [{ "id": "g", "label": "Check" }] },
            "nodeStates": [{ "nodeId": "g", "status": if waiting { "waiting" } else { "success" },
                             "message": "Look", "startedAt": "t1" }],
        }))
        .unwrap()
    }

    #[test]
    fn tells_a_runs_gates_only_when_they_change() {
        let w = Waiting::default();
        let told = w.changed(&run("running", true)).expect("a gate asks");
        assert_eq!(told["gates"][0]["id"], "r1|g");
        assert_eq!(told["gates"][0]["title"], "Check");
        assert_eq!(told["gates"][0]["subtitle"], "Ship");
        assert!(w.changed(&run("running", true)).is_none(), "nothing new");
        let gone = w.changed(&run("running", false)).expect("answered");
        assert_eq!(gone["gates"], json!([]));
        assert!(w.changed(&run("success", false)).is_none());
    }

    #[test]
    fn tells_a_run_that_was_seen_running_ended_once() {
        let w = Waiting::default();
        assert!(
            w.ended(&run("success", false)).is_none(),
            "never seen running"
        );
        assert!(w.ended(&run("running", false)).is_none());
        let told = w.ended(&run("success", false)).expect("it ended");
        assert_eq!(
            (
                told["workflow"].as_str(),
                told["outcome"].as_str(),
                told["steps"].as_u64()
            ),
            (Some("Ship"), Some("completed"), Some(1))
        );
        assert!(w.ended(&run("success", false)).is_none(), "told once");
        w.ended(&run("running", false));
        let mut rejected = run("error", false);
        rejected.node_states[0].status.0 = "error".into();
        rejected.node_states[0].rejected_at = serde_json::from_value(json!("t2")).unwrap();
        assert_eq!(w.ended(&rejected).unwrap()["outcome"], "rejected");
    }

    #[test]
    fn tells_a_step_waiting_for_a_sign_in_once() {
        let w = Waiting::default();
        let mut r = run("running", true);
        r.node_states[0].waiting_for = Some("signIn".into());
        r.node_states[0].error = serde_json::from_value(json!("Sign in to Jira")).unwrap();
        let told = w.signing_in(&r);
        assert_eq!(told.len(), 1);
        assert_eq!(told[0]["title"], "Check");
        assert_eq!(told[0]["message"], "Sign in to Jira");
        assert!(w.signing_in(&r).is_empty(), "told once");
        assert!(w.changed(&r).is_none(), "a sign-in is not a gate");
    }

    #[test]
    fn lists_every_waiting_gate_with_its_workflows_names() {
        let waiting = [serde_json::to_value(run("running", true)).unwrap()];
        let workflows =
            [json!({ "id": "w1", "name": "Named", "nodes": [{ "id": "g", "label": "" }] })];
        let rows = all_rows(&waiting, &workflows);
        assert_eq!(
            (rows[0]["subtitle"].as_str(), rows[0]["title"].as_str()),
            (Some("Named"), Some("Approval"))
        );
    }
}
