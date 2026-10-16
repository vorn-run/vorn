//! What the engine runs itself (`executeNode`): approval gates, conditions
//! and loops, and every other step handed to the node type that runs it,
//! in vornd or by a call to its extension.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use tracing::{info, warn};
use vorn_work::graph::{
    self, evaluate_condition, loop_body_graph, loop_should_stop, loop_structure_error,
    output_or_logs, step_outputs, LOOP_RESULT_OUTPUT_CHARS, MAX_LOOP_ITERATIONS,
};
use vorn_work::items::to_item_list;
use vorn_work::js::{self, iso_now};
use vorn_work::model::{
    Context, Edge, LoopPass, Node, NodeKind, RunExt, StateExt, Status, Workflow,
};
use vorn_work::template::{resolve, resolve_value, StepOutputs};

use crate::engine::{lock, Active, Engine, Run};
use crate::host::Host;
use crate::registry::{Resolved, Runner};
use crate::step::StepCx;
use crate::waves::{BoxFuture, Mode, Waves};

fn text_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// `value || undefined` for a string.
fn nonempty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// A step's config with every string in it resolved against the run, as an
/// extension that runs the step is handed it.
fn resolve_config(config: &Value, context: Option<&Context>, outputs: &StepOutputs) -> Value {
    match config {
        Value::String(s) => Value::String(resolve(s, context, Some(outputs))),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_config(v, context, outputs))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), resolve_config(v, context, outputs)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// What an extension's answer sets on its step's state: each field it
/// names, as it names it.
fn apply_answer(s: &mut vorn_protocol::NodeExecutionState, answer: &Map<String, Value>) {
    let text = |k: &str| answer.get(k).filter(|v| !v.is_null()).map(js::to_string);
    if let Some(v) = text("output") {
        s.output = Some(v);
    }
    if let Some(v) = text("logs") {
        s.logs = Some(v);
    }
    if let Some(v) = text("error") {
        s.error = Some(v);
    }
    if let Some(v) = text("waitingFor") {
        s.waiting_for = Some(v);
    }
    if let Some(v) = text("taskId") {
        s.task_id = Some(v);
    }
    if let Some(v) = text("sessionId") {
        s.session_id = Some(v);
    }
    if let Some(v) = text("diagnostics") {
        s.diagnostics = Some(v);
    }
    if let Some(v) = answer.get("structuredOutput").filter(|v| !v.is_null()) {
        s.structured_output = Some(v.clone());
    }
}

impl<H: Host> Engine<H> {
    /// Runs one step, recording what came of it in the run.
    pub(crate) fn execute_node<'a>(
        &'a self,
        node: &'a Node,
        workflow: &'a Workflow,
        run: &'a Run,
        context: Option<&'a Context>,
        outputs: &'a StepOutputs,
        active: &'a Arc<Active>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            match node.kind {
                NodeKind::Loop => {
                    return self
                        .execute_loop(node, workflow, run, context, active)
                        .await
                }
                NodeKind::Approval => {
                    self.open_gate(node, run, context, outputs);
                    return Ok(());
                }
                _ => {}
            }
            lock(run).update(&node.id, |s| {
                s.set_status(Status::Running);
                s.started_at = Some(iso_now());
            });
            self.persist(run);
            let kind = match &node.kind {
                NodeKind::Step(kind) => kind.as_str(),
                // Triggers never run; a condition is decided here.
                _ => {
                    self.condition(node, run, context, outputs);
                    return Ok(());
                }
            };
            match self.inner.registry.resolve(kind) {
                Resolved::Run(_, Runner::Local(step)) => {
                    let cx = StepCx {
                        engine: self,
                        node,
                        workflow,
                        run,
                        context,
                        outputs,
                        active,
                    };
                    step.run(cx).await
                }
                Resolved::Run(declared, Runner::Call) => {
                    self.call_step(&declared.provider, node, workflow, run, context, outputs)
                        .await;
                    Ok(())
                }
                Resolved::Off(declared) => Err(format!(
                    "{} steps come from {}, which is off. Turn it on in Settings › Extensions.",
                    declared.id, declared.provider_name
                )),
                Resolved::Unknown => Err(format!("No extension runs \"{kind}\" steps")),
            }
        })
    }

    /// A step of an extension's node type, run by a call to it: its config
    /// resolved against the run goes out with the run's context, and what
    /// it answers is the step's state. A `waiting` answer parks the step.
    async fn call_step(
        &self,
        provider: &str,
        node: &Node,
        workflow: &Workflow,
        run: &Run,
        context: Option<&Context>,
        outputs: &StepOutputs,
    ) {
        let run_id = lock(run).run_id.clone();
        let request = json!({
            "runId": run_id,
            "workflowId": workflow.id,
            "workflowName": workflow.name,
            "node": { "id": node.id, "type": node.kind.step(), "label": node.label, "slug": node.slug },
            "config": resolve_config(&node.config, context, outputs),
            "raw": node.config,
            "context": context.map_or(Value::Null, |c| serde_json::to_value(c).unwrap_or(Value::Null)),
        });
        let kind = node.kind.step().unwrap_or_default();
        let answer = self.inner.host.run_node(provider, kind, request).await;
        let answer = match answer {
            Ok(Value::Object(map)) => map,
            Ok(_) => Map::new(),
            Err(message) => {
                self.finish(run, node, |s| {
                    s.set_status(Status::Error);
                    s.error = Some(message);
                });
                return;
            }
        };
        let status = answer
            .get("status")
            .and_then(Value::as_str)
            .and_then(Status::parse)
            .unwrap_or(Status::Success);
        if status == Status::Waiting {
            lock(run).update(&node.id, |s| {
                s.set_status(Status::Waiting);
                apply_answer(s, &answer);
            });
            self.persist(run);
            return;
        }
        self.finish(run, node, |s| {
            s.set_status(status);
            apply_answer(s, &answer);
        });
    }

    pub(crate) fn finish(
        &self,
        run: &Run,
        node: &Node,
        change: impl FnOnce(&mut vorn_protocol::NodeExecutionState),
    ) {
        lock(run).update(&node.id, |s| {
            s.completed_at = Some(iso_now());
            change(s);
        });
        self.persist(run);
    }

    /// An approval gate asks: it resolves its message, the text a reviewer
    /// may rewrite and its review page, then waits.
    fn open_gate(&self, node: &Node, run: &Run, context: Option<&Context>, outputs: &StepOutputs) {
        let (run_id, round) = {
            let r = lock(run);
            let existing = r.node_state(&node.id);
            if existing.is_some_and(|s| s.is(Status::Waiting)) {
                return;
            }
            (
                r.run_id.clone(),
                existing.and_then(|s| s.round).unwrap_or(1.0),
            )
        };
        let config = &node.config;
        info!(gate = %node.label, round, "an approval gate waits");
        let message = text_of(config.get("message"))
            .filter(|m| !m.is_empty())
            .and_then(|m| nonempty(resolve(&m, context, Some(outputs)).trim().to_owned()));
        let editable = text_of(config.get("edit"))
            .filter(|e| !e.trim().is_empty())
            .map(|e| match resolve_value(&e, context, Some(outputs)) {
                Value::String(s) => s,
                other => js::stringify_pretty(&other),
            });
        let page = text_of(config.get("view"))
            .filter(|v| !v.trim().is_empty())
            .map(|view| {
                let html = resolve(&view, context, Some(outputs));
                self.inner
                    .host
                    .gate_view(&run_id, &node.id, round as u32, &html)
            });
        match &page {
            Some(Err(why)) => {
                warn!(gate = %node.label, %why, "a gate's review page could not be kept")
            }
            Some(Ok(_)) => {
                self.inner
                    .host
                    .keep_gate_page(&run_id, &node.id, &node.label, round as u32)
            }
            None => {}
        }
        lock(run).update(&node.id, |s| {
            s.set_status(Status::Waiting);
            s.started_at = Some(iso_now());
            s.round = Some(round);
            s.message = message;
            s.editable_text = editable;
            s.edited_text = None;
            s.view_token = page.as_ref().and_then(|p| p.as_ref().ok().cloned());
            s.diagnostics = page.as_ref().and_then(|p| p.as_ref().err().cloned());
        });
        self.persist(run);
        let timeout = config
            .get("timeoutMs")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        self.schedule_gate_timeout(run, &node.id, timeout, 0.0);
    }

    fn condition(&self, node: &Node, run: &Run, context: Option<&Context>, outputs: &StepOutputs) {
        let config = &node.config;
        let resolved = resolve(node.text("variable").unwrap_or(""), context, Some(outputs));
        let value = resolve(node.text("value").unwrap_or(""), context, Some(outputs));
        let operator = config.get("operator").and_then(Value::as_str).unwrap_or("");
        let result = evaluate_condition(operator, &resolved, &value);
        info!(condition = %node.label, %resolved, %operator, %value, result, "a condition was evaluated");
        self.finish(run, node, |s| {
            s.set_status(Status::Success);
            s.output = Some(result.to_string());
        });
    }

    /// its condition holds, its passes run out or its items are walked.
    fn execute_loop<'a>(
        &'a self,
        node: &'a Node,
        workflow: &'a Workflow,
        run: &'a Run,
        context: Option<&'a Context>,
        active: &'a Arc<Active>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let config = &node.config;
            let body = loop_body_graph(&workflow.nodes, &workflow.edges, node);
            let fail = |message: String| {
                self.finish(run, node, |s| {
                    s.set_status(Status::Error);
                    s.error = Some(message);
                });
            };
            if let Some(invalid) = loop_structure_error(&workflow.nodes, &workflow.edges, node) {
                fail(invalid);
                return Ok(());
            }
            let for_each = config.get("mode").and_then(Value::as_str) == Some("forEach");
            let mut items = Vec::new();
            if for_each {
                let template = node.text("items").unwrap_or("");
                let outputs = step_outputs(&lock(run), workflow);
                match to_item_list(&resolve_value(template, context, Some(&outputs))) {
                    Ok(list) => items = list.items,
                    Err(why) => {
                        let named = node
                            .text("items")
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .unwrap_or("nothing");
                        fail(format!("{why} The loop reads {named} as its items."));
                        return Ok(());
                    }
                }
            }
            let max = if for_each {
                items.len() as u32
            } else {
                match graph::js_number(config.get("maxIterations")) {
                    Some(n) if n.is_finite() => {
                        (n.floor().max(1.0) as u32).min(MAX_LOOP_ITERATIONS)
                    }
                    _ => 1,
                }
            };
            let unit = if for_each { "item" } else { "pass" };
            let mut summary: Vec<String> = Vec::new();
            let mut stop_reason = if for_each {
                format!("went through all {max} item(s)")
            } else {
                format!("reached the {max}-pass limit")
            };
            let mut passes = 0;
            let mut results: Vec<Value> = Vec::new();
            let mut pass_outputs: Vec<Value> = Vec::new();
            let mut edges: Vec<Edge> = body
                .entries
                .iter()
                .map(|id| Edge {
                    id: format!("{}->{id}:entry", node.id),
                    source: node.id.clone(),
                    target: id.clone(),
                    branch: None,
                })
                .collect();
            edges.extend(body.edges.iter().cloned());
            let fed: std::collections::HashSet<&str> =
                body.edges.iter().map(|e| e.source.as_str()).collect();
            let exits: Vec<&Node> = body
                .members
                .iter()
                .copied()
                .filter(|m| !fed.contains(m.id.as_str()))
                .collect();

            lock(run).update(&node.id, |s| {
                s.set_status(Status::Running);
                s.started_at = Some(iso_now());
                s.iteration = Some(0.0);
            });
            self.persist(run);

            for iteration in 1..=max {
                if active.cancel.is_cancelled() {
                    stop_reason = "the run was stopped".into();
                    break;
                }
                passes = iteration;
                summary.push(if for_each {
                    format!("── item {iteration} of {max} ──")
                } else {
                    format!("── iteration {iteration} of at most {max} ──")
                });
                let item = for_each.then(|| items[(iteration - 1) as usize].clone());
                let mut pass = context.cloned().unwrap_or_default();
                pass.pass = Some(LoopPass {
                    item: item.clone(),
                    index: iteration - 1,
                    number: iteration,
                    count: Some(max),
                });
                {
                    let mut r = lock(run);
                    for m in &body.members {
                        if let Some(s) = r.node_state_mut(&m.id) {
                            *s = graph::blank_pass_state(&m.id, Some(iteration));
                        }
                    }
                }
                self.persist(run);

                self.run_waves(
                    workflow,
                    run,
                    Some(&pass),
                    active,
                    Waves {
                        nodes: body.members.clone(),
                        edges: edges.clone(),
                        roots: vec![node.id.clone()],
                        max_waves: 50 * body.members.len(),
                        stagger: None,
                    },
                    Mode::Pass {
                        context: &pass,
                        iteration,
                    },
                )
                .await?;

                let stopped = active.cancel.is_cancelled();
                {
                    let mut r = lock(run);
                    let now = iso_now();
                    for m in &body.members {
                        r.update(&m.id, |s| {
                            if s.is(Status::Pending) || s.is(Status::Running) {
                                s.set_status(Status::Skipped);
                                s.completed_at = Some(now.clone());
                                s.iteration = Some(f64::from(iteration));
                                if stopped {
                                    s.error = Some("Skipped: the run was stopped".into());
                                }
                            }
                        });
                        let status = r
                            .node_state(&m.id)
                            .map_or_else(|| "unknown".to_owned(), |s| s.status.0.clone());
                        summary.push(format!("  {}: {status}", m.label));
                    }
                }
                self.persist(run);

                let (failed_step, exit_text, result) = {
                    let r = lock(run);
                    let failed = body
                        .members
                        .iter()
                        .find(|m| {
                            r.node_state(&m.id).is_some_and(|s| s.is(Status::Error))
                                && m.stops_run_on_error()
                        })
                        .map(|m| m.label.clone());
                    let exit = exits
                        .iter()
                        .filter_map(|m| r.node_state(&m.id))
                        .find(|s| s.is(Status::Success) || s.is(Status::Error))
                        .map(|s| cap(output_or_logs(s)).to_owned())
                        .unwrap_or_default();
                    let result = pass_result(
                        &r,
                        iteration,
                        item.as_ref(),
                        &body.members,
                        failed.is_some(),
                    );
                    (failed, exit, result)
                };
                results.push(result);
                pass_outputs.push(Value::String(exit_text));
                if let Some(label) = failed_step {
                    stop_reason = format!("\"{label}\" failed on {unit} {iteration}");
                    break;
                }
                if active.cancel.is_cancelled() {
                    stop_reason = "the run was stopped".into();
                    break;
                }
                if let Some(until) = config.get("until").filter(|u| !u.is_null()) {
                    let outputs = step_outputs(&lock(run), workflow);
                    let text = |k: &str| until.get(k).and_then(Value::as_str).unwrap_or("");
                    let variable = resolve(text("variable"), Some(&pass), Some(&outputs));
                    let value = resolve(text("value"), Some(&pass), Some(&outputs));
                    if loop_should_stop(Some(until), &variable, &value) {
                        stop_reason = format!("the condition held after {unit} {iteration}");
                        break;
                    }
                    let named = until
                        .get("variable")
                        .map_or_else(|| "undefined".to_owned(), js::to_string);
                    summary.push(format!("  condition not met ({named} was \"{variable}\")"));
                }
            }

            if for_each && max == 0 {
                let mut r = lock(run);
                let now = iso_now();
                for m in &body.members {
                    r.update(&m.id, |s| {
                        s.set_status(Status::Skipped);
                        s.skip_reason = Some("branch".into());
                        s.completed_at = Some(now.clone());
                    });
                }
                stop_reason = "the list was empty".into();
            }
            let failed = {
                let r = lock(run);
                body.members.iter().any(|m| {
                    r.node_state(&m.id).is_some_and(|s| s.is(Status::Error))
                        && m.stops_run_on_error()
                })
            };
            summary.push(format!("Stopped after {passes} {unit}(s): {stop_reason}."));
            self.finish(run, node, |s| {
                s.set_status(if failed {
                    Status::Error
                } else {
                    Status::Success
                });
                s.iteration = Some(f64::from(passes));
                s.output = Some(passes.to_string());
                s.logs = Some(summary.join("\n"));
                s.structured_output = Some(json!({
                    "passes": passes,
                    "count": max,
                    "results": results,
                    "outputs": pass_outputs,
                }));
                if failed {
                    s.error = Some(stop_reason.clone());
                }
            });
            Ok(())
        })
    }
}

/// The start of a pass's output, where the answer is.
fn cap(text: &str) -> &str {
    js::head(text, LOOP_RESULT_OUTPUT_CHARS)
}

/// What one pass left behind, per step, for `{{steps.<loop>.results}}`.
fn pass_result(
    run: &vorn_protocol::WorkflowExecution,
    iteration: u32,
    item: Option<&Value>,
    body: &[&Node],
    failed: bool,
) -> Value {
    let mut steps = Map::new();
    for step in body {
        let Some(slug) = &step.slug else { continue };
        let state = run.node_state(&step.id);
        let mut entry = Map::new();
        if let Some(Value::Object(fields)) = state.and_then(|s| s.structured_output.as_ref()) {
            for (k, v) in fields {
                entry.insert(k.clone(), v.clone());
            }
        }
        entry.insert(
            "output".into(),
            Value::String(state.map_or("", |s| cap(output_or_logs(s))).to_owned()),
        );
        entry.insert(
            "status".into(),
            Value::String(state.map_or_else(|| "unknown".to_owned(), |s| s.status.0.clone())),
        );
        entry.insert(
            "error".into(),
            Value::String(state.and_then(|s| s.error.clone()).unwrap_or_default()),
        );
        steps.insert(slug.clone(), Value::Object(entry));
    }
    let mut out = Map::new();
    out.insert("index".into(), json!(iteration - 1));
    if let Some(item) = item {
        out.insert("item".into(), item.clone());
    }
    out.insert(
        "status".into(),
        json!(if failed { "error" } else { "success" }),
    );
    out.insert("steps".into(), Value::Object(steps));
    Value::Object(out)
}
