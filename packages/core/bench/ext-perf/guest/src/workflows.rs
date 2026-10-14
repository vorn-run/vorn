//! The engine's wave walk for conditions and approval gates, as a step machine a component can run one call at a time.

use std::collections::HashSet;

use serde_json::Value;
pub use vorn_protocol::WorkflowExecution;

use crate::graph::{
    collapse_loop_bodies, collect_skipped_branch, dedupe_fingerprint, evaluate_condition,
    run_ended_in_error, skip_entry_points, step_outputs, Graph,
};
use crate::js::iso_now;
use crate::model::{state, Edge, Node, NodeKind, RunExt, StateExt, Status, Workflow};
use crate::template::{resolve, StepOutputs};

/// What the walk needs from vornd: the run's fan-out and writes, and its log.
pub trait Host {
    /// The run changed: tell viewers and write it, in order.
    fn persist(&mut self, run: &WorkflowExecution);
    /// One info line, where the built-in engine logs one.
    fn log(&mut self, line: &str);
}

/// How a walk ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The run is over, `success` or `error`.
    Ended(String),
    /// A gate waits; vornd keeps the stored run until it is answered.
    Parked,
}

impl Outcome {
    pub fn as_str(&self) -> &str {
        match self {
            Outcome::Ended(status) => status,
            Outcome::Parked => "parked",
        }
    }
}

/// A walk's result: the run as it stands and how many steps ran.
#[derive(Debug)]
pub struct Walk {
    pub run: WorkflowExecution,
    pub outcome: Outcome,
    pub steps: u32,
}

/// A new run of `raw`, walked until it ends or parks.
pub fn start(raw: &Value, run_id: String, host: &mut impl Host) -> Result<Walk, String> {
    let wf = Workflow::from_json(raw).ok_or("the workflow has no id")?;
    let mut run = WorkflowExecution {
        run_id,
        workflow_id: wf.id.clone(),
        started_at: iso_now(),
        status: "running".into(),
        node_states: Vec::new(),
        completed_at: None,
        connector_inbox_disposition: None,
        connector_inbox_id: None,
        connector_inbox_lease_token: None,
        connector_item: None,
        dedupe_params: Some(dedupe_fingerprint(None)),
        definition: Some(wf.raw.clone()),
        inputs: None,
        partial: None,
        retry_of_run_id: None,
        trigger_session: None,
        trigger_task_id: None,
    };
    run.node_states = wf
        .nodes
        .iter()
        .map(|n| {
            let status = if n.kind == NodeKind::Trigger {
                Status::Success
            } else {
                Status::Pending
            };
            state(&n.id, status)
        })
        .collect();
    host.log(&format!(
        "a workflow run started: {} {}",
        wf.name, run.run_id
    ));
    host.persist(&run);
    walk(&wf, run, host)
}

/// Answers the gate `node_id` of a stored run and walks on.
pub fn resume(
    raw: &Value,
    mut run: WorkflowExecution,
    node_id: &str,
    approve: bool,
    host: &mut impl Host,
) -> Result<Walk, String> {
    let wf = Workflow::from_json(raw).ok_or("the workflow has no id")?;
    if !run
        .node_state(node_id)
        .is_some_and(|s| s.is(Status::Waiting))
    {
        host.persist(&run);
        let outcome = Outcome::Ended(run.status.clone());
        return Ok(Walk {
            run,
            outcome,
            steps: 0,
        });
    }
    let now = iso_now();
    run.update(node_id, |s| {
        s.completed_at = Some(now.clone());
        if approve {
            s.set_status(Status::Success);
            s.approved_at = Some(now.clone());
        } else {
            s.set_status(Status::Error);
            s.rejected_at = Some(now.clone());
            s.error = Some("Rejected".into());
        }
    });
    if !approve {
        run.status = "error".into();
        run.completed_at = Some(iso_now());
        host.persist(&run);
        return Ok(Walk {
            run,
            outcome: Outcome::Ended("error".into()),
            steps: 0,
        });
    }
    host.persist(&run);
    walk(&wf, run, host)
}

/// What a wave knows of each step, as the engine's `Seen`.
#[derive(Default)]
struct Seen {
    completed: HashSet<String>,
    skipped: Vec<String>,
    skipped_set: HashSet<String>,
}

impl Seen {
    fn settled(&self, id: &str) -> bool {
        self.completed.contains(id) || self.skipped_set.contains(id)
    }

    fn skip(&mut self, id: String) {
        if self.skipped_set.insert(id.clone()) {
            self.skipped.push(id);
        }
    }

    fn reset(&mut self) {
        self.completed.clear();
        self.skipped.clear();
        self.skipped_set.clear();
    }
}

fn walk(wf: &Workflow, mut run: WorkflowExecution, host: &mut impl Host) -> Result<Walk, String> {
    let edges = collapse_loop_bodies(&wf.nodes, &wf.edges);
    let graph = Graph::of(&edges);
    let actions = wf
        .nodes
        .iter()
        .filter(|n| n.kind != NodeKind::Trigger)
        .count();
    let max_waves = 50 * actions.max(1);
    let mut seen = Seen::default();
    let mut wave = 0;
    let mut steps = 0;
    loop {
        seen.reset();
        for ns in &run.node_states {
            match ns.status() {
                Some(Status::Success | Status::Error) => {
                    seen.completed.insert(ns.node_id.clone());
                }
                Some(Status::Skipped) => seen.skip(ns.node_id.clone()),
                _ => {}
            }
        }
        let batch: Vec<&Node> = wf
            .nodes
            .iter()
            .filter(|n| n.kind != NodeKind::Trigger && !seen.settled(&n.id))
            .filter(|n| {
                !run.node_state(&n.id)
                    .is_some_and(|ns| ns.is(Status::Waiting))
            })
            .filter(|n| {
                let preds = graph.predecessors_of(&n.id);
                preds.iter().all(|p| seen.settled(p))
                    && preds.iter().any(|p| seen.completed.contains(p))
            })
            .collect();
        if batch.is_empty() {
            break;
        }
        wave += 1;
        if wave > max_waves {
            return Err(format!(
                "Stopped after {max_waves} waves: steps kept becoming ready again"
            ));
        }
        let labels: Vec<&str> = batch.iter().map(|n| n.label.as_str()).collect();
        host.log(&format!(
            "a wave of steps runs: {wave} {}",
            labels.join(", ")
        ));
        let outputs = step_outputs(&run, wf);
        for node in batch {
            if let Err(message) = execute(node, &mut run, &outputs, host) {
                run.update(&node.id, |s| {
                    s.set_status(Status::Error);
                    s.completed_at = Some(iso_now());
                    s.error = Some(message);
                });
                host.persist(&run);
            }
            steps += 1;
            settle(node, &mut run, &mut seen, &graph, &edges, host);
        }
    }
    let skipped = seen.skipped_set;
    if run.node_states.iter().any(|s| s.is(Status::Waiting)) {
        host.persist(&run);
        return Ok(Walk {
            run,
            outcome: Outcome::Parked,
            steps,
        });
    }
    let now = iso_now();
    let mut stranded = false;
    for s in run.node_states.iter_mut().filter(|s| s.is(Status::Pending)) {
        s.set_status(Status::Error);
        s.completed_at = Some(now.clone());
        s.error = Some("Skipped: predecessor nodes did not complete".into());
        stranded = true;
    }
    if stranded {
        host.persist(&run);
    }
    let failed = run_ended_in_error(&run, wf, Some(&skipped));
    run.status = if failed { "error" } else { "success" }.into();
    run.completed_at = Some(iso_now());
    host.persist(&run);
    let outcome = Outcome::Ended(run.status.clone());
    Ok(Walk {
        run,
        outcome,
        steps,
    })
}

/// One step, its result recorded in the run.
fn execute(
    node: &Node,
    run: &mut WorkflowExecution,
    outputs: &StepOutputs,
    host: &mut impl Host,
) -> Result<(), String> {
    if node.kind == NodeKind::Approval {
        open_gate(node, run, outputs, host);
        return Ok(());
    }
    run.update(&node.id, |s| {
        s.set_status(Status::Running);
        s.started_at = Some(iso_now());
    });
    host.persist(run);
    match node.kind {
        NodeKind::Condition => {
            let resolved = resolve(node.text("variable").unwrap_or(""), None, Some(outputs));
            let value = resolve(node.text("value").unwrap_or(""), None, Some(outputs));
            let operator = node
                .config
                .get("operator")
                .and_then(Value::as_str)
                .unwrap_or("");
            let result = evaluate_condition(operator, &resolved, &value);
            host.log(&format!(
                "a condition was evaluated: {} {resolved} {operator} {value} {result}",
                node.label
            ));
            run.update(&node.id, |s| {
                s.completed_at = Some(iso_now());
                s.set_status(Status::Success);
                s.output = Some(result.to_string());
            });
            host.persist(run);
            Ok(())
        }
        _ => Err(format!(
            "\"{}\" is a step this slice does not run",
            node.label
        )),
    }
}

/// An approval gate asks and waits; its message resolved as the engine resolves it.
fn open_gate(
    node: &Node,
    run: &mut WorkflowExecution,
    outputs: &StepOutputs,
    host: &mut impl Host,
) {
    let existing = run.node_state(&node.id);
    if existing.is_some_and(|s| s.is(Status::Waiting)) {
        return;
    }
    let round = existing.and_then(|s| s.round).unwrap_or(1.0);
    host.log(&format!("an approval gate waits: {} {round}", node.label));
    let message = node
        .text("message")
        .filter(|m| !m.is_empty())
        .map(|m| resolve(m, None, Some(outputs)).trim().to_owned())
        .filter(|m| !m.is_empty());
    run.update(&node.id, |s| {
        s.set_status(Status::Waiting);
        s.started_at = Some(iso_now());
        s.round = Some(round);
        s.message = message;
        s.editable_text = None;
        s.edited_text = None;
        s.view_token = None;
        s.diagnostics = None;
    });
    host.persist(run);
}

/// What one finished step means for the rest, as the engine's `settle_step`.
fn settle(
    node: &Node,
    run: &mut WorkflowExecution,
    seen: &mut Seen,
    graph: &Graph,
    edges: &[Edge],
    host: &mut impl Host,
) {
    let Some((status, output)) = run
        .node_state(&node.id)
        .map(|ns| (ns.status(), ns.output.clone()))
    else {
        return;
    };
    if status == Some(Status::Waiting) {
        return;
    }
    seen.completed.insert(node.id.clone());
    if status == Some(Status::Error) && node.stops_run_on_error() {
        for entry in skip_entry_points(&node.id, edges, graph, |id| seen.settled(id)) {
            for id in collect_skipped_branch(&entry, graph, |id| seen.settled(id)) {
                seen.skip(id);
            }
        }
        let error = format!("Skipped: \"{}\" failed", node.label);
        stamp_skipped(run, seen, true, |ns| ns.error = Some(error.clone()));
        host.persist(run);
        return;
    }
    if node.kind != NodeKind::Condition {
        return;
    }
    let skip_branch = match status {
        Some(Status::Error) => None,
        _ if output.as_deref() == Some("true") => Some("false"),
        _ => Some("true"),
    };
    let mut any = false;
    for edge in edges.iter().filter(|e| e.source == node.id) {
        let Some(branch) = edge.branch.as_deref() else {
            continue;
        };
        if skip_branch.is_some_and(|b| b != branch) {
            continue;
        }
        for id in collect_skipped_branch(&edge.target, graph, |id| seen.settled(id)) {
            seen.skip(id);
        }
        any = true;
    }
    if any {
        stamp_skipped(run, seen, false, |ns| {
            ns.skip_reason = Some("branch".into())
        });
        host.persist(run);
    }
}

fn stamp_skipped(
    run: &mut WorkflowExecution,
    seen: &Seen,
    only_pending: bool,
    extra: impl Fn(&mut vorn_protocol::NodeExecutionState),
) {
    let now = iso_now();
    for id in &seen.skipped {
        run.update(id, |ns| {
            if only_pending && !ns.is(Status::Pending) {
                return;
            }
            ns.set_status(Status::Skipped);
            ns.completed_at = Some(now.clone());
            extra(ns);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Kept {
        runs: Vec<WorkflowExecution>,
    }

    impl Host for Kept {
        fn persist(&mut self, run: &WorkflowExecution) {
            self.runs.push(run.clone());
        }
        fn log(&mut self, _line: &str) {}
    }

    fn chain(steps: usize, gate_after: Option<usize>) -> Value {
        let mut nodes = vec![
            json!({ "id": "t", "type": "trigger", "label": "Start", "config": { "triggerType": "manual" } }),
        ];
        let mut edges = Vec::new();
        let mut prev = "t".to_owned();
        for i in 1..=steps {
            let id = format!("s{i}");
            nodes.push(json!({ "id": id, "type": "condition", "label": format!("Step {i}"), "config": { "variable": "1", "operator": "equals", "value": "1" } }));
            edges.push(json!({ "id": format!("e{i}"), "source": prev, "target": id }));
            prev = id;
            if gate_after == Some(i) {
                nodes.push(json!({ "id": "g", "type": "approval", "label": "Gate", "config": { "message": "Go on?" } }));
                edges.push(json!({ "id": "eg", "source": prev, "target": "g" }));
                prev = "g".into();
            }
        }
        json!({ "id": "w", "name": "W", "enabled": true, "nodes": nodes, "edges": edges })
    }

    #[test]
    fn walks_a_chain_writing_each_step_twice() {
        let mut host = Kept::default();
        let walk = start(&chain(10, None), "r".into(), &mut host).unwrap();
        assert_eq!(walk.outcome, Outcome::Ended("success".into()));
        assert_eq!(walk.steps, 10);
        // The start, running and done per step, and the end.
        assert_eq!(host.runs.len(), 1 + 2 * 10 + 1);
        assert!(walk.run.node_states.iter().all(|s| s.is(Status::Success)));
        assert!(walk.run.completed_at.is_some());
    }

    #[test]
    fn parks_at_a_gate_and_walks_on_when_it_is_approved() {
        let mut host = Kept::default();
        let wf = chain(4, Some(2));
        let walk = start(&wf, "r".into(), &mut host).unwrap();
        assert_eq!(walk.outcome, Outcome::Parked);
        assert_eq!(walk.steps, 3);
        let gate = walk.run.node_state("g").unwrap();
        assert!(gate.is(Status::Waiting));
        assert_eq!(gate.message.as_deref(), Some("Go on?"));
        let walk = resume(&wf, walk.run, "g", true, &mut host).unwrap();
        assert_eq!(walk.outcome, Outcome::Ended("success".into()));
        assert_eq!(walk.steps, 2);
        assert!(walk.run.node_state("g").unwrap().approved_at.is_some());
    }

    #[test]
    fn a_rejected_gate_ends_the_run_before_what_follows() {
        let mut host = Kept::default();
        let wf = chain(3, Some(1));
        let walk = start(&wf, "r".into(), &mut host).unwrap();
        let walk = resume(&wf, walk.run, "g", false, &mut host).unwrap();
        assert_eq!(walk.outcome, Outcome::Ended("error".into()));
        assert!(walk.run.node_state("s2").unwrap().is(Status::Pending));
    }

    #[test]
    fn a_false_condition_skips_its_true_branch() {
        let wf = json!({
            "id": "w", "name": "W", "enabled": true,
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Start", "config": { "triggerType": "manual" } },
                { "id": "c", "type": "condition", "label": "C", "config": { "variable": "a", "operator": "equals", "value": "b" } },
                { "id": "y", "type": "condition", "label": "Y", "config": {} },
                { "id": "n", "type": "condition", "label": "N", "config": {} }
            ],
            "edges": [
                { "id": "e0", "source": "t", "target": "c" },
                { "id": "e1", "source": "c", "target": "y", "conditionBranch": "true" },
                { "id": "e2", "source": "c", "target": "n", "conditionBranch": "false" }
            ]
        });
        let walk = start(&wf, "r".into(), &mut Kept::default()).unwrap();
        assert_eq!(walk.outcome, Outcome::Ended("success".into()));
        assert!(walk.run.node_state("y").unwrap().is(Status::Skipped));
        assert!(walk.run.node_state("n").unwrap().is(Status::Success));
    }

    #[test]
    fn a_step_outside_the_slice_fails_the_run() {
        let wf = json!({
            "id": "w", "name": "W", "enabled": true,
            "nodes": [
                { "id": "t", "type": "trigger", "label": "Start", "config": {} },
                { "id": "a", "type": "launchAgent", "label": "Agent", "config": {} }
            ],
            "edges": [{ "id": "e", "source": "t", "target": "a" }]
        });
        let walk = start(&wf, "r".into(), &mut Kept::default()).unwrap();
        assert_eq!(walk.outcome, Outcome::Ended("error".into()));
    }
}
