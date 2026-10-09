//! `tools/workflows.ts` and `tools/describe-nodes.ts`: workflows, their runs,
//! the gates runs park on, and moving a workflow between machines.

use serde_json::{json, Map, Value};

use super::data::{self, Update};
use super::{error_of, failed, items, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;
use crate::workflow::graph::{self, is_sign_in_wait, name_of};
use crate::workflow::portability as portable;
use crate::zod;

/// `resolveWorkflowId`: `workflow_id`, or its older alias `id`.
fn resolve_workflow_id(args: &Args) -> Result<String, String> {
    let id = match args.get("workflow_id") {
        Some(id) => Some(id),
        None => args.get("id"),
    };
    let Some(id) = id.and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return Err("provide workflow_id".to_owned());
    };
    if let (Some(a), Some(b)) = (args.nonempty("workflow_id"), args.nonempty("id")) {
        if a != b {
            return Err("workflow_id and id disagree — pass only workflow_id".to_owned());
        }
    }
    Ok(id.to_owned())
}

fn not_found(id: &str) -> Value {
    failed(format!("Error: workflow \"{id}\" not found"))
}

fn graph_refusal(errors: &[String]) -> Value {
    failed(format!("Error: {}", errors.join("; ")))
}

/// `describeConfigIssues(node)`: each config problem as "field: why".
fn describe_config_issues(node: &Value) -> Vec<String> {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
    zod::node_config_issues(zod::registry(), kind, node.get("config"))
        .iter()
        .map(|issue| {
            let mut path = vec!["config".to_owned()];
            path.extend(issue.path.iter().map(zod::Seg::display));
            format!("{}: {}", path.join("."), issue.message)
        })
        .collect()
}

/// `checkNodeConfigs`: config problems on the nodes an update sends, split by
/// whether the caller made them. A config left exactly as stored is a warning.
fn check_node_configs(nodes: &[Value], stored: &[Value]) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for node in nodes {
        let issues = describe_config_issues(node);
        if issues.is_empty() {
            continue;
        }
        let before = stored.iter().find(|n| {
            json::strict_equals(n.get("id"), node.get("id"))
                && json::strict_equals(n.get("type"), node.get("type"))
        });
        let untouched = match (before.and_then(|b| b.get("config")), node.get("config")) {
            (Some(a), Some(b)) => json::deep_equal(a, b),
            (None, None) => before.is_some(),
            _ => false,
        };
        let line = format!("node \"{}\" {}", name_of(node), issues.join(", "));
        if untouched {
            warnings.push(line);
        } else {
            errors.push(line);
        }
    }
    (errors, warnings)
}

pub async fn list_workflows<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let mut workflows = data::list(cx, "workflows").await?;
    if let Some(workspace) = args.nonempty("workspace_id") {
        workflows.retain(|w| super::projects::in_workspace(w, workspace));
    }
    Ok(pretty(&Value::Array(workflows)))
}

/// The label a convenience-mode trigger node gets.
fn trigger_label(trigger: &Value) -> &'static str {
    match trigger.get("triggerType").and_then(Value::as_str) {
        Some("manual") => "Manual Trigger",
        Some("once") => "Schedule (Once)",
        Some("recurring") => "Schedule (Recurring)",
        Some("taskCreated") => "When Task Created",
        Some("taskStatusChanged") => "When Task Status Changes",
        _ => "Trigger",
    }
}

fn uuid() -> Value {
    Value::from(uuid::Uuid::new_v4().to_string())
}

/// `buildGraphFromFlat`: a trigger, then the agents one after another.
fn build_graph_from_flat(trigger: &Value, actions: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let trigger_id = uuid();
    let mut nodes = vec![json!({
        "id": trigger_id,
        "type": "trigger",
        "label": trigger_label(trigger),
        "config": trigger,
        "position": { "x": 0, "y": 0 }
    })];
    let mut edges = Vec::new();
    let mut prev = trigger_id;
    for (i, action) in actions.iter().enumerate() {
        let id = uuid();
        nodes.push(json!({
            "id": id,
            "type": "launchAgent",
            "label": format!("Launch {}", json::display(action.get("agentType"))),
            "config": action,
            "position": { "x": 0, "y": (i + 1) * 140 }
        }));
        edges.push(json!({ "id": uuid(), "source": prev, "target": id }));
        prev = id;
    }
    (nodes, edges)
}

pub async fn create_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let (nodes, edges) = match (args.get("nodes"), args.get("edges")) {
        (Some(Value::Array(nodes)), Some(Value::Array(edges))) => {
            let errors = graph::validate_graph(nodes, edges)?;
            if !errors.is_empty() {
                return Ok(graph_refusal(&errors));
            }
            (nodes.clone(), edges.clone())
        }
        _ => {
            let trigger = args
                .get("trigger")
                .cloned()
                .unwrap_or_else(|| json!({ "triggerType": "manual" }));
            build_graph_from_flat(&trigger, items(args.get("actions")))
        }
    };
    let workflow = object([
        ("id", Some(uuid())),
        ("name", args.get("name").cloned()),
        (
            "icon",
            Some(args.get("icon").cloned().unwrap_or_else(|| json!("Zap"))),
        ),
        (
            "iconColor",
            Some(
                args.get("icon_color")
                    .cloned()
                    .unwrap_or_else(|| json!("#6366f1")),
            ),
        ),
        ("nodes", Some(Value::Array(nodes))),
        ("edges", Some(Value::Array(edges))),
        (
            "enabled",
            Some(args.get("enabled").cloned().unwrap_or(Value::Bool(true))),
        ),
        (
            "staggerDelayMs",
            args.get("stagger_delay_ms")
                .filter(|v| json::truthy(Some(v)))
                .cloned(),
        ),
    ]);
    data::create_workflow(cx, &workflow).await?;
    Ok(pretty(&workflow))
}

pub async fn update_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = match resolve_workflow_id(args) {
        Ok(id) => id,
        Err(err) => return Ok(error_of(err)),
    };
    let Some(workflow) = data::find(cx, "workflows", "id", &id).await? else {
        return Ok(not_found(&id));
    };
    let stored_nodes = items(json::prop(Some(&workflow), "nodes")?);

    let mut warnings = Vec::new();
    if let Some(nodes) = args.get("nodes") {
        let (errors, legacy) = check_node_configs(items(Some(nodes)), stored_nodes);
        if !errors.is_empty() {
            return Ok(graph_refusal(&errors));
        }
        warnings = legacy;
    }
    if args.get("nodes").is_some() || args.get("edges").is_some() {
        let nodes = items(args.get("nodes").or(workflow.get("nodes")));
        let edges = items(args.get("edges").or(workflow.get("edges")));
        let errors = graph::validate_graph(nodes, edges)?;
        if !errors.is_empty() {
            return Ok(graph_refusal(&errors));
        }
    }

    let mut updates: Update = Vec::new();
    for (arg, key) in [
        ("name", "name"),
        ("nodes", "nodes"),
        ("edges", "edges"),
        ("icon", "icon"),
        ("icon_color", "iconColor"),
        ("enabled", "enabled"),
        ("stagger_delay_ms", "staggerDelayMs"),
    ] {
        if let Some(value) = args.get(arg) {
            updates.push((key, Some(value.clone())));
        }
    }
    data::update_workflow(cx, &id, &data::merged(&Value::Null, &updates)).await?;

    let saved = json::pretty(&data::merged(&workflow, &updates));
    Ok(text(if warnings.is_empty() {
        saved
    } else {
        format!(
            "Warning: saved, but these configs were stored before and break a current rule, so they were kept as they are:\n  - {}\n\n{saved}",
            warnings.join("\n  - ")
        )
    }))
}

pub async fn delete_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = match resolve_workflow_id(args) {
        Ok(id) => id,
        Err(err) => return Ok(error_of(err)),
    };
    let Some(workflow) = data::find(cx, "workflows", "id", &id).await? else {
        return Ok(not_found(&id));
    };
    data::delete_workflow(cx, &id).await?;
    Ok(text(format!(
        "Deleted workflow: {}",
        json::display(workflow.get("name"))
    )))
}

/// `run.nodeStates.<method>(...)`, which a run without them throws on.
fn node_states<'a>(run: &'a Value, method: &str) -> Result<&'a [Value], String> {
    let states = json::prop(Some(run), "nodeStates")?;
    match states {
        Some(Value::Array(list)) => Ok(list),
        None => Err(format!(
            "Cannot read properties of undefined (reading '{method}')"
        )),
        Some(Value::Null) => Err(format!(
            "Cannot read properties of null (reading '{method}')"
        )),
        Some(_) => Err(format!("run.nodeStates.{method} is not a function")),
    }
}

fn is_waiting(state: &Value) -> bool {
    state.get("status").and_then(Value::as_str) == Some("waiting")
}

/// `approvalNode(workflow, nodeId)`.
fn approval_node<'a>(workflow: Option<&'a Value>, node_id: Option<&Value>) -> Option<&'a Value> {
    items(json::field(workflow, "nodes")).iter().find(|n| {
        json::strict_equals(n.get("id"), node_id)
            && n.get("type").and_then(Value::as_str) == Some("approval")
    })
}

/// `askedBy(node)`: what an approval node asks, if anything.
fn asked_by(node: Option<&Value>) -> Option<String> {
    let message = json::field(json::field(node, "config"), "message")?.as_str()?;
    let trimmed = json::trim(message);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// `annotateWaitingGates`: each waiting node says what it asks.
fn annotate_waiting_gates(runs: &[Value], workflows: &[Value]) -> Result<Vec<Value>, String> {
    runs.iter()
        .map(|run| {
            let states = node_states(run, "some")?;
            if !states.iter().any(is_waiting) {
                return Ok(run.clone());
            }
            let workflow = workflows
                .iter()
                .find(|w| json::strict_equals(w.get("id"), run.get("workflowId")));
            let annotated: Vec<Value> = states
                .iter()
                .map(|state| {
                    if !is_waiting(state) {
                        return state.clone();
                    }
                    let mut out = match state {
                        Value::Object(map) => map.clone(),
                        _ => Map::new(),
                    };
                    if is_sign_in_wait(state) {
                        out.insert(
                            "asks".into(),
                            json!("Sign in to its connection in the Vorn app, and this step runs again"),
                        );
                        return Value::Object(out);
                    }
                    let asks = match state.get("message") {
                        None | Some(Value::Null) => {
                            asked_by(approval_node(workflow, state.get("nodeId"))).map(Value::from)
                        }
                        Some(message) => Some(message.clone()),
                    };
                    match asks.filter(|a| json::truthy(Some(a))) {
                        Some(asks) => {
                            out.insert("asks".into(), asks);
                            Value::Object(out)
                        }
                        None => state.clone(),
                    }
                })
                .collect();
            let mut out = match run {
                Value::Object(map) => map.clone(),
                _ => Map::new(),
            };
            out.insert("nodeStates".into(), Value::Array(annotated));
            Ok(Value::Object(out))
        })
        .collect()
}

/// Reads the definitions only when some run is parked on a gate.
async fn with_gates<R: Rpc>(cx: &Cx<'_, R>, runs: Value) -> Result<Value, String> {
    let list = items(Some(&runs));
    let mut parked = false;
    for run in list {
        if node_states(run, "some")?.iter().any(is_waiting) {
            parked = true;
            break;
        }
    }
    if !parked {
        return Ok(runs);
    }
    let workflows = data::list(cx, "workflows").await?;
    Ok(Value::Array(annotate_waiting_gates(list, &workflows)?))
}

pub async fn list_workflow_runs<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    if args.truthy("workflow_id") && args.truthy("task_id") {
        return Ok(failed("Error: provide workflow_id or task_id, not both"));
    }
    let limit = args.get("limit").cloned().unwrap_or_else(|| json!(20));
    if let Some(task) = args.nonempty("task_id") {
        let runs = data::workflow_runs_by_task(cx, task, limit).await?;
        return Ok(pretty(&with_gates(cx, runs).await?));
    }
    if let Some(workflow) = args.nonempty("workflow_id") {
        let runs = data::workflow_runs(cx, workflow, limit).await?;
        return Ok(pretty(&with_gates(cx, runs).await?));
    }
    let waiting = data::runs_with_waiting_gates(cx).await?;
    let take = limit.as_f64().unwrap_or(20.0) as usize;
    let parked: Vec<Value> = items(Some(&waiting)).iter().take(take).cloned().collect();
    Ok(pretty(&with_gates(cx, Value::Array(parked)).await?))
}

/// `runById`: recent history first, then every parked run.
async fn run_by_id<R: Rpc>(cx: &Cx<'_, R>, run_id: &str) -> Result<Option<Value>, String> {
    let recent = data::all_workflow_runs(cx, 500).await?;
    let found = items(Some(&recent))
        .iter()
        .find(|r| r.get("runId").and_then(Value::as_str) == Some(run_id))
        .cloned();
    if found.is_some() {
        return Ok(found);
    }
    let parked = data::runs_with_waiting_gates(cx).await?;
    Ok(items(Some(&parked))
        .iter()
        .find(|r| r.get("runId").and_then(Value::as_str) == Some(run_id))
        .cloned())
}

/// ` of "<name>"` for a run that carries its workflow's name.
fn of_workflow(run: &Value) -> String {
    if json::truthy(run.get("workflowName")) {
        format!(" of \"{}\"", json::display(run.get("workflowName")))
    } else {
        String::new()
    }
}

pub async fn stop_workflow_run<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let run_id = args.str("run_id").unwrap_or_default();
    let Some(run) = run_by_id(cx, run_id).await? else {
        return Ok(failed(format!(
            "Error: no run \"{run_id}\" in the last 500. Check list_workflow_runs."
        )));
    };
    if run.get("status").and_then(Value::as_str) != Some("running") {
        return Ok(text(format!(
            "Run {run_id} already finished ({}) — nothing to stop.",
            json::display(run.get("status"))
        )));
    }
    if let Err(err) = cx
        .call("workflow:stopRun", Some(json!({ "runId": run_id })))
        .await
    {
        return Ok(error_of(err));
    }
    let live = node_states(&run, "filter")?
        .iter()
        .filter(|n| {
            matches!(
                n.get("status").and_then(Value::as_str),
                Some("running" | "waiting")
            )
        })
        .count();
    Ok(text(format!(
        "Asked to stop run {run_id}{} — {live} node(s) were still live.\n\nThe run is stopped by the instance holding it, so confirm with list_workflow_runs.",
        of_workflow(&run)
    )))
}

/// `resolveGateTarget`: which gate a decision answers.
fn resolve_gate_target(
    run: &Value,
    node_id: Option<&str>,
    decision: &str,
) -> Result<Result<Value, String>, String> {
    let parked: Vec<&Value> = node_states(run, "filter")?
        .iter()
        .filter(|n| is_waiting(n))
        .collect();
    let (answerable, sign_ins): (Vec<&Value>, Vec<&Value>) = parked
        .iter()
        .partition(|n| decision == "reject" || !is_sign_in_wait(n));
    let id_of = |n: &&Value| n.get("nodeId").cloned().unwrap_or(Value::Null);
    let waiting: Vec<Value> = answerable.iter().map(id_of).collect();
    let sign_ins: Vec<Value> = sign_ins.iter().map(id_of).collect();
    if let Some(node_id) = node_id.filter(|s| !s.is_empty()) {
        let wanted = Value::from(node_id);
        if sign_ins
            .iter()
            .any(|v| json::strict_equals(Some(v), Some(&wanted)))
        {
            return Ok(Err(format!(
                "node \"{node_id}\" is waiting for a sign-in in the Vorn app, not for an approval"
            )));
        }
        if waiting
            .iter()
            .any(|v| json::strict_equals(Some(v), Some(&wanted)))
        {
            return Ok(Ok(wanted));
        }
        return Ok(Err(if waiting.is_empty() {
            format!("node \"{node_id}\" is not waiting, and neither is any other node in this run")
        } else {
            format!(
                "node \"{node_id}\" is not waiting. Waiting: {}",
                json::join(&waiting, ", ")
            )
        }));
    }
    match waiting.as_slice() {
        [one] => Ok(Ok(one.clone())),
        [] => Ok(Err(if sign_ins.is_empty() {
            "no node in this run is waiting on a gate".to_owned()
        } else {
            "this run is waiting for a sign-in in the Vorn app, not for an approval".to_owned()
        })),
        many => Ok(Err(format!(
            "{} nodes are waiting — pass node_id: {}",
            many.len(),
            json::join(many, ", ")
        ))),
    }
}

pub async fn resolve_gate<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let run_id = args.str("run_id").unwrap_or_default();
    let decision = args.str("decision").unwrap_or_default();
    let comments: Vec<Value> = items(args.get("comments"))
        .iter()
        .filter(|c| {
            c.get("comment")
                .and_then(Value::as_str)
                .is_some_and(|s| !json::trim(s).is_empty())
        })
        .cloned()
        .collect();
    let Some(run) = run_by_id(cx, run_id).await? else {
        return Ok(failed(format!(
            "Error: no run \"{run_id}\" in the recent history, and none parked on a gate. Check list_workflow_runs."
        )));
    };
    if run.get("status").and_then(Value::as_str) != Some("running") {
        return Ok(text(format!(
            "Run {run_id} already finished ({}) — no gate to answer.",
            json::display(run.get("status"))
        )));
    }
    let target = match resolve_gate_target(&run, args.str("node_id"), decision)? {
        Ok(target) => target,
        Err(err) => return Ok(error_of(err)),
    };
    let comment = args
        .str("comment")
        .map(json::trim)
        .filter(|s| !s.is_empty());
    let edited = args.str("edited").map(json::trim).filter(|s| !s.is_empty());
    if decision == "changes" && comment.is_none() && comments.is_empty() {
        return Ok(failed(
            "Error: changes needs a comment saying what to change.",
        ));
    }

    let workflows = data::list(cx, "workflows").await?;
    let workflow = workflows
        .iter()
        .find(|w| json::strict_equals(w.get("id"), run.get("workflowId")));
    let gate_node = approval_node(workflow, Some(&target));
    let state_message = node_states(&run, "find")?
        .iter()
        .find(|n| json::strict_equals(n.get("nodeId"), Some(&target)))
        .and_then(|n| n.get("message"))
        .filter(|m| !m.is_null())
        .cloned();
    let asked = state_message.or_else(|| asked_by(gate_node).map(Value::from));

    let params = object([
        ("runId", Some(Value::from(run_id))),
        ("nodeId", Some(target.clone())),
        ("decision", Some(Value::from(decision))),
        ("comment", comment.map(Value::from)),
        ("edited", edited.map(Value::from)),
        (
            "comments",
            (decision == "changes" && !comments.is_empty()).then(|| Value::Array(comments)),
        ),
    ]);
    match cx.call("workflow:resolveGate", Some(params)).await {
        Err(err) => return Ok(error_of(err)),
        Ok(answer) => {
            if json::field(Some(&answer), "accepted") == Some(&Value::Bool(false)) {
                let reason = answer.get("reason").filter(|r| json::truthy(Some(r)));
                return Ok(failed(match reason {
                    Some(reason) => format!("Error: {}", json::display(Some(reason))),
                    None if decision == "changes" => "Error: this gate takes no changes now: it has no step to redo from, or its rounds are used up. Approve or reject it instead.".to_owned(),
                    None => "Error: the gate did not take that answer. Check list_workflow_runs.".to_owned(),
                }));
            }
        }
    }

    let answered = match decision {
        "approve" => "Approved",
        "reject" => "Rejected",
        _ => "Sent back",
    };
    let gate = gate_node
        .and_then(|n| n.get("label"))
        .filter(|l| !l.is_null())
        .unwrap_or(&target);
    let asked = asked
        .filter(|a| json::truthy(Some(a)))
        .map(|a| format!("\n\nWhat it asked: {}", json::display(Some(&a))))
        .unwrap_or_default();
    Ok(text(format!(
        "{answered} \"{}\" on run {run_id}{}.{asked}\n\nThe decision went out; the instance holding the run acts on it, so a desktop has to be open. Confirm with list_workflow_runs.",
        json::display(Some(gate)),
        of_workflow(&run)
    )))
}

pub async fn get_workflow_schedule<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let run = async {
        if args.str("info") == Some("next_run") {
            let Some(id) = args.nonempty("workflow_id") else {
                return Ok::<Value, String>(failed("Error: workflow_id is required for next_run"));
            };
            let next = cx
                .call("scheduler:getNextRun", Some(Value::from(id)))
                .await?;
            return Ok(text(if json::truthy(Some(&next)) {
                json::pretty(&json!({ "nextRun": next }))
            } else {
                "No scheduled run (workflow may be manual or disabled)".to_owned()
            }));
        }
        let log = cx
            .call("scheduler:getLog", args.get("workflow_id").cloned())
            .await?;
        Ok(pretty(&log))
    };
    Ok(run.await.unwrap_or_else(error_of))
}

/// `resolveWorkflowInputs`: supplied values matched against the declared ones,
/// as the run dialog does for a person.
fn resolve_workflow_inputs(
    defs: &[Value],
    supplied: &Map<String, Value>,
) -> (Map<String, Value>, Vec<String>) {
    let mut errors = Vec::new();
    let mut known: Vec<Value> = Vec::new();
    for def in defs {
        let key = def.get("key").cloned().unwrap_or(Value::Null);
        if !known
            .iter()
            .any(|k| json::strict_equals(Some(k), Some(&key)))
        {
            known.push(key);
        }
    }
    for key in supplied.keys() {
        if !known.iter().any(|k| k.as_str() == Some(key.as_str())) {
            let declared = json::join(&known, ", ");
            errors.push(format!(
                "unknown input \"{key}\" — this workflow declares: {}",
                if declared.is_empty() {
                    "(none)"
                } else {
                    &declared
                }
            ));
        }
    }

    let mut values = Map::new();
    for def in defs {
        let key = json::display(def.get("key"));
        let raw = match supplied.get(&key) {
            Some(v) => Some(v),
            None => def.get("defaultValue"),
        };
        let shown = || raw.map_or_else(|| "undefined".to_owned(), json::stringify);
        let kind = def.get("type").and_then(Value::as_str);
        if kind == Some("boolean") {
            match raw {
                None => {
                    values.insert(key, Value::Bool(false));
                }
                Some(Value::Bool(b)) => {
                    values.insert(key, Value::Bool(*b));
                }
                Some(Value::String(s)) if s == "true" || s == "false" => {
                    values.insert(key, Value::Bool(s == "true"));
                }
                Some(_) => errors.push(format!(
                    "input \"{key}\" must be a boolean, got {}",
                    shown()
                )),
            }
            continue;
        }
        let blank = match raw {
            None => true,
            Some(Value::String(s)) => json::trim(s).is_empty(),
            Some(_) => false,
        };
        if blank {
            if json::truthy(def.get("required")) {
                errors.push(format!(
                    "missing required input \"{key}\" ({})",
                    json::display(def.get("label"))
                ));
            }
            continue;
        }
        let raw = raw.expect("a blank value was handled above");
        match kind {
            Some("number") => {
                if raw.is_boolean() {
                    errors.push(format!("input \"{key}\" must be a number, got {}", shown()));
                    continue;
                }
                let n = match raw {
                    Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                    other => json::string_to_number(&json::display(Some(other))),
                };
                if n.is_finite() {
                    values.insert(key, json::num(n));
                } else {
                    errors.push(format!(
                        "input \"{key}\" must be a finite number, got {}",
                        shown()
                    ));
                }
            }
            Some("select") => {
                let allowed: Vec<Value> = items(def.get("options"))
                    .iter()
                    .map(|o| o.get("value").cloned().unwrap_or(Value::Null))
                    .collect();
                let text = json::display(Some(raw));
                if allowed.is_empty() {
                    errors.push(format!(
                        "input \"{key}\" is a select but declares no options"
                    ));
                } else if !allowed.iter().any(|a| a.as_str() == Some(text.as_str())) {
                    errors.push(format!(
                        "input \"{key}\" must be one of: {}",
                        json::join(&allowed, ", ")
                    ));
                } else {
                    values.insert(key, Value::from(text));
                }
            }
            _ => {
                values.insert(key, Value::from(json::display(Some(raw))));
            }
        }
    }
    (values, errors)
}

pub async fn execute_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("workflow_id").unwrap_or_default();
    let Some(workflow) = data::find(cx, "workflows", "id", id).await? else {
        return Ok(not_found(id));
    };
    let mut trigger = None;
    for node in items(json::prop(Some(&workflow), "nodes")?) {
        if json::prop(Some(node), "type")?.and_then(Value::as_str) == Some("trigger") {
            trigger = node.get("config");
            break;
        }
    }
    let defs = if json::field(trigger, "triggerType").and_then(Value::as_str) == Some("manual") {
        items(json::field(trigger, "inputs"))
    } else {
        &[]
    };
    let supplied = match args.get("inputs") {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };

    let inputs = if !defs.is_empty() {
        let (values, errors) = resolve_workflow_inputs(defs, &supplied);
        if !errors.is_empty() {
            return Ok(failed(format!(
                "Error: invalid inputs\n  - {}",
                errors.join("\n  - ")
            )));
        }
        // Always an object once inputs are declared, so `{{inputs.x}}`
        // expands to empty rather than staying as raw template text.
        Some(Value::Object(values))
    } else if !supplied.is_empty() {
        Some(Value::Object(supplied))
    } else {
        None
    };

    let params = object([
        ("workflowId", Some(Value::from(id))),
        ("inputs", inputs.clone()),
    ]);
    if let Err(err) = cx.call("workflow:runManual", Some(params)).await {
        return Ok(error_of(err));
    }
    let disabled = if workflow.get("enabled") == Some(&Value::Bool(false)) {
        " (workflow is disabled; manual runs still execute)"
    } else {
        ""
    };
    let shown = match &inputs {
        Some(inputs) => format!("\ninputs: {}", json::pretty(inputs)),
        None => "\nno inputs".to_owned(),
    };
    Ok(text(format!(
        "Queued \"{}\"{disabled}{shown}\n\nRun history: list_workflow_runs with workflow_id {id}",
        json::display(workflow.get("name"))
    )))
}

/// `listPortableConnections`: connections for naming and rebinding
/// requirements, or none when they cannot be read.
async fn portable_connections<R: Rpc>(cx: &Cx<'_, R>) -> Vec<Value> {
    match cx.call("connection:list", Some(json!({}))).await {
        Ok(list) => items(Some(&list)).to_vec(),
        Err(_) => Vec::new(),
    }
}

pub async fn export_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = match resolve_workflow_id(args) {
        Ok(id) => id,
        Err(err) => return Ok(error_of(err)),
    };
    let Some(workflow) = data::find(cx, "workflows", "id", &id).await? else {
        return Ok(not_found(&id));
    };
    let projects = data::list(cx, "projects").await?;
    // Every node's project name is read before the first is picked, so a node
    // without a config throws wherever it sits.
    let mut names = Vec::new();
    for node in items(json::prop(Some(&workflow), "nodes")?) {
        let config = json::prop(Some(node), "config")?;
        names.push(json::prop(config, "projectName")?);
    }
    let project_name = names
        .into_iter()
        .flatten()
        .find(|name| matches!(name, Value::String(s) if !s.is_empty()));
    let project = projects
        .iter()
        .find(|p| json::strict_equals(p.get("name"), project_name));
    let Some(project) = project else {
        return Ok(failed(format!(
            "Error: no project named \"{}\" is registered, so this workflow's paths cannot be made relative to anything.",
            project_name.map_or_else(|| "(none)".to_owned(), |n| json::display(Some(n)))
        )));
    };

    let connections = portable_connections(cx).await;
    let path = project
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let exported = portable::to_portable(&workflow, path, &connections);
    let residual = portable::residual_absolute_paths(&exported);
    let unnamed = items(exported.get("requires"))
        .iter()
        .filter(|r| {
            r.get("kind").and_then(Value::as_str) == Some("connection")
                && r.get("connectorId").and_then(Value::as_str) == Some("")
        })
        .count();
    let mut out = json::pretty(&exported);
    if !residual.is_empty() {
        out.push_str(&format!(
            "\n\nWarning: these still hold a machine-specific path and will not travel: {}",
            residual.join(", ")
        ));
    }
    if unnamed > 0 {
        out.push_str(&format!(
            "\n\nWarning: {unnamed} step(s) point at a connection this install could not name, so an import cannot rebind them automatically."
        ));
    }
    Ok(text(out))
}

/// What `String(err)` prints for a `JSON.parse` failure, approximately: V8
/// words each failure its own way, which serde_json does not reproduce.
fn json_parse_error(err: &serde_json::Error) -> String {
    format!("SyntaxError: {err}")
}

pub async fn import_workflow<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let source = args.str("workflow").unwrap_or_default();
    let parsed: Value = match serde_json::from_str(source) {
        Ok(v) => json::js_order(v),
        Err(err) => {
            let shown: String = json::utf16_prefix(&json_parse_error(&err), 200).to_owned();
            return Ok(failed(format!(
                "Error: workflow is not valid JSON — {shown}"
            )));
        }
    };
    let version = json::field(Some(&parsed), "version");
    if !json::strict_equals(version, Some(&json::num(portable::PORTABLE_FORMAT_VERSION))) {
        return Ok(failed(format!(
            "Error: unsupported format version {}; this build reads version 1",
            json::display(version)
        )));
    }
    let (Some(Value::Array(nodes)), Some(Value::Array(edges))) =
        (parsed.get("nodes"), parsed.get("edges"))
    else {
        return Ok(failed("Error: workflow is missing name, nodes or edges"));
    };
    if !json::truthy(parsed.get("name")) {
        return Ok(failed("Error: workflow is missing name, nodes or edges"));
    }

    let project_name = args.str("project_name").unwrap_or_default();
    let Some(project) = data::find(cx, "projects", "name", project_name).await? else {
        return Ok(failed(format!(
            "Error: no project named \"{project_name}\". Create it first so its path is known."
        )));
    };

    let errors = graph::validate_graph(nodes, edges)?;
    if !errors.is_empty() {
        return Ok(graph_refusal(&errors));
    }

    let bundle = match args.str("bundle") {
        Some(bundle) => bundle.to_owned(),
        None => portable::slugify_value(project.get("name"))?,
    };
    let mut file = match &parsed {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let slug = match parsed.get("slug") {
        None | Some(Value::Null) => Value::from(portable::slugify_value(parsed.get("name"))?),
        Some(slug) => slug.clone(),
    };
    file.insert("slug".into(), slug.clone());
    let file = Value::Object(file);

    let connections = portable_connections(cx).await;
    let project_path = project
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resolved = portable::from_portable(
        &file,
        &bundle,
        project.get("name").unwrap_or(&Value::Null),
        project_path,
        &connections,
    );
    let unresolved = portable::unresolved_requirements(&file, &connections);

    let known = data::list(cx, "workflows").await?;
    let slug_text = json::display(Some(&slug));
    let id = portable::imported_workflow_id_for(&bundle, &slug_text, file.get("name"), &known);
    let existing = known
        .iter()
        .find(|w| w.get("id").and_then(Value::as_str) == Some(id.as_str()));
    // A file cannot ask to be running; one that already ran keeps its answer.
    let enabled = match existing {
        Some(w) => w.get("enabled").cloned(),
        None => Some(Value::Bool(false)),
    };
    let mut definition = match resolved {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    definition.insert("id".into(), Value::from(id.clone()));
    match &enabled {
        Some(v) => {
            definition.insert("enabled".into(), v.clone());
        }
        None => {
            definition.shift_remove("enabled");
        }
    }

    let definition = Value::Object(definition);
    if existing.is_some() {
        data::update_workflow(cx, &id, &definition).await?;
    } else {
        data::create_workflow(cx, &definition).await?;
    }

    let pending: Vec<String> = unresolved
        .iter()
        .map(|r| {
            let like = if json::truthy(r.get("name")) {
                format!(" like \"{}\"", json::display(r.get("name")))
            } else {
                String::new()
            };
            let node = json::display(r.get("nodeId"));
            if r.get("kind").and_then(Value::as_str) == Some("httpProfile") {
                format!("{node} needs an HTTP profile{like}")
            } else {
                let connector = if json::truthy(r.get("connectorId")) {
                    json::display(r.get("connectorId"))
                } else {
                    "connector".to_owned()
                };
                format!("{node} needs a {connector} connection{like}")
            }
        })
        .collect();
    let pending = pending.join("; ");

    let mut out = format!(
        "{} \"{}\" as {id}, resolved against {}",
        if existing.is_some() {
            "Updated"
        } else {
            "Imported"
        },
        json::display(definition.get("name")),
        json::display(project.get("path"))
    );
    if existing.is_none() && !json::truthy(definition.get("enabled")) {
        out.push_str(". It is disabled; enable it when ready");
    }
    if !pending.is_empty() {
        out.push_str(&format!("\n\nStill to connect: {pending}"));
    }
    Ok(text(out))
}

/// `describe_workflow_nodes`: the reference, or the part of it for some types.
pub fn describe_workflow_nodes(args: &Args) -> Value {
    let reference = crate::workflow_nodes();
    let types = items(args.get("types"));
    if types.is_empty() {
        return pretty(reference);
    }
    let mut out = match reference {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let all = reference.get("nodeTypes");
    let mut chosen = Map::new();
    for t in types {
        let key = json::display(Some(t));
        let doc = json::field(all, &key).cloned();
        // `Object.fromEntries` keeps an `undefined` value, which JSON then drops.
        match doc {
            Some(doc) => {
                chosen.insert(key, doc);
            }
            None => {
                chosen.shift_remove(&key);
            }
        }
    }
    out.insert("nodeTypes".into(), Value::Object(chosen));
    pretty(&Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(v: Value) -> Value {
        v
    }

    #[test]
    fn inputs_resolve_as_the_run_dialog_does() {
        let defs = [
            def(json!({ "key": "flag", "label": "Flag", "type": "boolean" })),
            def(json!({ "key": "n", "label": "N", "type": "number", "required": true })),
            def(
                json!({ "key": "pick", "label": "Pick", "type": "select", "options": [{ "value": "a" }], "defaultValue": "a" }),
            ),
            def(json!({ "key": "t", "label": "T", "type": "text" })),
        ];
        let supplied = json!({ "n": " 12 ", "t": 5 });
        let (values, errors) = resolve_workflow_inputs(&defs, supplied.as_object().unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            Value::Object(values),
            json!({ "flag": false, "n": 12, "pick": "a", "t": "5" })
        );

        let supplied = json!({ "zz": "1", "flag": "yes", "n": "Infinity", "pick": "b" });
        let (_, errors) = resolve_workflow_inputs(&defs, supplied.as_object().unwrap());
        assert_eq!(
            errors,
            [
                "unknown input \"zz\" — this workflow declares: flag, n, pick, t",
                "input \"flag\" must be a boolean, got \"yes\"",
                "input \"n\" must be a finite number, got \"Infinity\"",
                "input \"pick\" must be one of: a",
            ]
        );
    }

    #[test]
    fn a_gate_is_named_when_more_than_one_waits() {
        let run = json!({ "nodeStates": [
            { "nodeId": "a", "status": "waiting" },
            { "nodeId": "b", "status": "waiting" },
            { "nodeId": "c", "status": "waiting", "waitingFor": "signIn" }
        ]});
        assert_eq!(
            resolve_gate_target(&run, None, "approve").unwrap(),
            Err("2 nodes are waiting — pass node_id: a, b".to_owned())
        );
        assert_eq!(
            resolve_gate_target(&run, Some("c"), "approve").unwrap(),
            Err(
                "node \"c\" is waiting for a sign-in in the Vorn app, not for an approval"
                    .to_owned()
            )
        );
        assert_eq!(
            resolve_gate_target(&run, Some("c"), "reject").unwrap(),
            Ok(json!("c"))
        );
    }

    #[test]
    fn workflow_id_and_its_alias_must_agree() {
        let args = Args::new(Some(json!({ "workflow_id": "a", "id": "b" })));
        assert_eq!(
            resolve_workflow_id(&args).unwrap_err(),
            "workflow_id and id disagree — pass only workflow_id"
        );
        let args = Args::new(Some(json!({ "id": "b" })));
        assert_eq!(resolve_workflow_id(&args).unwrap(), "b");
        assert_eq!(
            resolve_workflow_id(&Args::new(None)).unwrap_err(),
            "provide workflow_id"
        );
    }
}
