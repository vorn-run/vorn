//! What each kind of step does (`executeNode`): gates, conditions,
//! scripts, connector actions, HTTP requests, tasks from connector items,
//! loops, and agents, headless or in a terminal.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use vorn_work::graph::{
    self, append_bounded_log, evaluate_condition, loop_body_graph, loop_should_stop,
    loop_structure_error, output_or_logs, resolve_script_config, step_outputs, worktree_mode,
    worktree_origin, WorktreeMode, DEFAULT_STEP_TIMEOUT_MINUTES, LOOP_RESULT_OUTPUT_CHARS,
    MAX_LOOP_ITERATIONS,
};
use vorn_work::items::to_item_list;
use vorn_work::js::{self, iso_now};
use vorn_work::model::{
    Context, Edge, LoopPass, Node, NodeKind, RunExt, StateExt, Status, Workflow,
};
use vorn_work::prompt::{task_prompt, workflow_prompt};
use vorn_work::structured;
use vorn_work::template::{context_field, resolve, resolve_value, Field, StepOutputs};

use crate::engine::{lock, new_id, tasks_of, Active, Engine, Run};
use crate::host::{Host, Note};
use crate::waves::{BoxFuture, Mode, Waves};

/// How often a headless step's output is written while it runs.
const PERSIST_INTERVAL: Duration = Duration::from_secs(3);

/// What the engine did on a step's behalf, timed from the step's start.
struct Diagnostics {
    lines: Vec<String>,
    started: Instant,
}

impl Diagnostics {
    fn new() -> Diagnostics {
        Diagnostics {
            lines: Vec::new(),
            started: Instant::now(),
        }
    }

    fn note(&mut self, message: impl AsRef<str>) {
        let seconds = self.started.elapsed().as_secs_f64();
        self.lines
            .push(format!("[+{seconds:.1}s] {}", message.as_ref()));
    }

    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// How a headless step ended.
enum Outcome {
    Exit(i64),
    Timeout(u64),
    Stopped,
}

fn text_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// `value || undefined` for a string.
fn nonempty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Writes `key` when the value is there, as JSON drops `undefined`.
fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), v);
    }
}

/// `ActionResult.output` when it is a plain object: an array would turn
/// into index keys in the step's outputs.
fn plain_object(result: &Value) -> Option<Value> {
    result.get("output").filter(|o| o.is_object()).cloned()
}

fn result_error(result: &Value) -> Option<String> {
    result
        .get("error")
        .filter(|e| vorn_work::is_truthy(Some(e)))
        .map(js::to_string)
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
            match &node.kind {
                NodeKind::Condition => self.condition(node, run, context, outputs),
                NodeKind::Script => self.script(node, run, context, outputs).await,
                NodeKind::CallConnectorAction => {
                    self.connector_action(node, run, context, outputs).await
                }
                NodeKind::HttpRequest => self.http_request(node, run, context, outputs).await,
                NodeKind::CreateTaskFromItem => self.task_from_item(node, run, context).await,
                _ => {
                    return self
                        .agent(node, workflow, run, context, outputs, active)
                        .await
                }
            }
            Ok(())
        })
    }

    fn finish(
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

    async fn script(
        &self,
        node: &Node,
        run: &Run,
        context: Option<&Context>,
        outputs: &StepOutputs,
    ) {
        let script_run = new_id();
        let mut config = resolve_script_config(&node.config, context, Some(outputs));
        if let Some(map) = config.as_object_mut() {
            map.insert("runId".into(), Value::String(script_run.clone()));
        }
        let mut notes = self.inner.host.notes();
        let call = self.inner.host.call("script:execute", config);
        tokio::pin!(call);
        let mut streamed = String::new();
        let result = loop {
            tokio::select! {
                result = &mut call => break result,
                note = notes.recv() => match note {
                    Ok(Note::ScriptData { run_id, data }) if run_id == script_run => {
                        append_bounded_log(&mut streamed, &data);
                        lock(run).update(&node.id, |s| s.logs = Some(streamed.clone()));
                        self.publish(run);
                    }
                    Err(RecvError::Closed) => break (&mut call).await,
                    _ => {}
                },
            }
        };
        // Output that arrived with the answer.
        while let Ok(note) = notes.try_recv() {
            if let Note::ScriptData { run_id, data } = note {
                if run_id == script_run {
                    append_bounded_log(&mut streamed, &data);
                }
            }
        }
        match result {
            Ok(result) => {
                let success = result
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let output = result
                    .get("output")
                    .filter(|o| !o.is_null())
                    .map(js::to_string);
                let error = result_error(&result);
                let trailer = match &error {
                    Some(e) if streamed.is_empty() => format!("\nError: {e}"),
                    _ => String::new(),
                };
                let logs = if streamed.is_empty() {
                    output.clone().unwrap_or_else(|| "undefined".into())
                } else {
                    streamed
                };
                self.finish(run, node, |s| {
                    s.set_status(if success {
                        Status::Success
                    } else {
                        Status::Error
                    });
                    s.output = output;
                    s.logs = Some(logs + &trailer);
                    s.error = error;
                });
            }
            Err(message) => self.finish(run, node, |s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }

    async fn connector_action(
        &self,
        node: &Node,
        run: &Run,
        context: Option<&Context>,
        outputs: &StepOutputs,
    ) {
        let config = &node.config;
        let action = config.get("action").map(js::to_string).unwrap_or_default();
        let mut args = Map::new();
        for (k, v) in config
            .get("args")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let text = v.as_str().map_or_else(|| js::to_string(v), str::to_owned);
            args.insert(
                k.clone(),
                Value::String(resolve(&text, context, Some(outputs))),
            );
        }
        let mut params = Map::new();
        put(
            &mut params,
            "connectionId",
            config.get("connectionId").cloned(),
        );
        put(&mut params, "action", config.get("action").cloned());
        params.insert("args".into(), Value::Object(args));
        let params = Value::Object(params);
        match self
            .inner
            .host
            .call("connection:executeAction", params)
            .await
        {
            Ok(result) => {
                let success = result
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let logs = js::stringify_pretty(&result);
                let error = result_error(&result);
                if !success
                    && result.get("errorKind").and_then(Value::as_str) == Some("needs-sign-in")
                {
                    lock(run).update(&node.id, |s| {
                        s.set_status(Status::Waiting);
                        s.waiting_for = Some("signIn".into());
                        s.logs = Some(logs);
                        if error.is_some() {
                            s.error = error;
                        }
                    });
                    self.persist(run);
                    return;
                }
                let structured = plain_object(&result);
                self.finish(run, node, |s| {
                    s.set_status(if success {
                        Status::Success
                    } else {
                        Status::Error
                    });
                    s.output = Some(format!(
                        "{action} {}",
                        if success { "succeeded" } else { "failed" }
                    ));
                    s.logs = Some(logs);
                    if structured.is_some() {
                        s.structured_output = structured;
                    }
                    if error.is_some() {
                        s.error = error;
                    }
                });
            }
            Err(message) => self.finish(run, node, |s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }

    async fn http_request(
        &self,
        node: &Node,
        run: &Run,
        context: Option<&Context>,
        outputs: &StepOutputs,
    ) {
        let config = &node.config;
        let mut headers = Map::new();
        for (name, value) in config
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let text = value
                .as_str()
                .map_or_else(|| js::to_string(value), str::to_owned);
            headers.insert(
                name.to_owned(),
                Value::String(resolve(&text, context, Some(outputs))),
            );
        }
        let mut params = Map::new();
        put(
            &mut params,
            "profileConnectionId",
            config
                .get("profileConnectionId")
                .filter(|v| vorn_work::is_truthy(Some(v)))
                .cloned(),
        );
        put(&mut params, "method", config.get("method").cloned());
        params.insert(
            "url".into(),
            Value::String(resolve(
                node.text("url").unwrap_or(""),
                context,
                Some(outputs),
            )),
        );
        params.insert("headers".into(), Value::Object(headers));
        put(
            &mut params,
            "body",
            node.text("body")
                .filter(|b| !b.is_empty())
                .map(|b| Value::String(resolve(b, context, Some(outputs)))),
        );
        match self
            .inner
            .host
            .call("http:request", Value::Object(params))
            .await
        {
            Ok(result) => {
                let success = result
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let status = result
                    .get("output")
                    .and_then(|o| o.get("status"))
                    .map_or_else(|| "undefined".to_owned(), js::to_string);
                let structured = plain_object(&result);
                let error = result_error(&result);
                let logs = js::stringify_pretty(&result);
                self.finish(run, node, |s| {
                    s.set_status(if success {
                        Status::Success
                    } else {
                        Status::Error
                    });
                    s.output = Some(if success {
                        format!("HTTP {status}")
                    } else {
                        "Request failed".into()
                    });
                    s.logs = Some(logs);
                    if structured.is_some() {
                        s.structured_output = structured;
                    }
                    if error.is_some() {
                        s.error = error;
                    }
                });
            }
            Err(message) => self.finish(run, node, |s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }

    async fn task_from_item(&self, node: &Node, run: &Run, context: Option<&Context>) {
        let Some(item) = context.and_then(|c| c.connector_item.clone()) else {
            self.finish(run, node, |s| {
                s.set_status(Status::Skipped);
                s.error = Some("No connector item in context — this node only runs from a connectorPoll trigger.".into());
            });
            return;
        };
        let project = node
            .text("project")
            .filter(|p| !p.is_empty() && *p != "fromConnection");
        let mut params = Map::new();
        params.insert(
            "connectionId".into(),
            item.get("connectionId").cloned().unwrap_or(Value::Null),
        );
        params.insert("item".into(), item.clone());
        params.insert(
            "initialStatus".into(),
            node.config
                .get("initialStatus")
                .cloned()
                .unwrap_or(Value::Null),
        );
        put(
            &mut params,
            "project",
            project.map(|p| Value::String(p.to_owned())),
        );
        match self
            .inner
            .host
            .call("connection:upsertFromItem", Value::Object(params))
            .await
        {
            Ok(result) => {
                let created = vorn_work::is_truthy(result.get("created"));
                let task_id = result.get("taskId").map(js::to_string).unwrap_or_default();
                let title = item.get("title").map(js::to_string).unwrap_or_default();
                let snippet = if js::utf16_len(&title) > 60 {
                    format!("{}...", js::head(&title, 57))
                } else {
                    title
                };
                let external = item
                    .get("externalId")
                    .map_or_else(|| "undefined".into(), js::to_string);
                let summary = format!(
                    "{} #{external} \"{snippet}\"",
                    if created { "Imported" } else { "Updated" }
                );
                let url = item
                    .get("externalUrl")
                    .filter(|u| !u.is_null())
                    .map_or_else(|| "(no url)".to_owned(), js::to_string);
                self.finish(run, node, |s| {
                    s.set_status(Status::Success);
                    s.task_id = Some(task_id.clone());
                    s.logs = Some(format!("{summary}\nSource: {url}\nTaskId: {task_id}"));
                    s.output = Some(summary);
                });
            }
            Err(message) => self.finish(run, node, |s| {
                s.set_status(Status::Error);
                s.error = Some(message);
            }),
        }
    }

    /// A loop runs its body as a graph of its own, once per pass, until
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

    /// An agent step: resolves its task, project, worktree and prompt, then
    /// starts it headless and waits for it, or opens it in a terminal.
    async fn agent(
        &self,
        node: &Node,
        workflow: &Workflow,
        run: &Run,
        context: Option<&Context>,
        outputs: &StepOutputs,
        active: &Arc<Active>,
    ) -> Result<(), String> {
        let config = &node.config;
        let headless = vorn_work::is_truthy(config.get("headless"));
        info!(step = %node.label, headless, "an agent step starts");
        let mut prompt = text_of(config.get("prompt"));
        let mut task_id: Option<String> = None;
        let mut branch = text_of(config.get("branch"))
            .filter(|b| !b.is_empty())
            .and_then(|b| nonempty(resolve(&b, context, None)));
        let from_context = config.get("useWorktree").and_then(Value::as_str) == Some("fromContext");
        let mut use_worktree: Option<Value> = if from_context {
            context
                .and_then(|c| context_field("useWorktree", c))
                .map(|f| match f {
                    Field::Flag(b) => Value::Bool(b),
                    Field::Text(t) => Value::String(t),
                })
        } else {
            config.get("useWorktree").filter(|v| !v.is_null()).cloned()
        };
        let mut existing: Option<String> = None;
        let current = self.config().await;
        let empty = Value::Null;
        let cfg = current.as_ref().unwrap_or(&empty);

        match worktree_mode(config) {
            WorktreeMode::FromStep => {
                let slug = node
                    .text("worktreeFromStepSlug")
                    .filter(|s| !s.is_empty())
                    .ok_or("Worktree mode \"fromStep\" requires a source step slug".to_owned())?;
                let mut by_key: HashMap<&str, &Node> = HashMap::new();
                for n in &workflow.nodes {
                    by_key.insert(n.slug.as_deref().unwrap_or(&n.id), n);
                }
                let source = by_key
                    .get(slug)
                    .ok_or_else(|| format!("Worktree source step \"{slug}\" not found"))?;
                let path = lock(run)
                    .node_state(&source.id)
                    .and_then(|s| s.worktree_path.clone())
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| format!("Source step \"{slug}\" has no worktreePath"))?;
                existing = Some(path);
                use_worktree = None;
            }
            WorktreeMode::Existing => {
                let path = node
                    .text("existingWorktreePath")
                    .filter(|p| !p.is_empty())
                    .ok_or(
                        "Worktree mode \"existing\" requires an existingWorktreePath".to_owned(),
                    )?;
                existing = Some(path.to_owned());
                use_worktree = None;
            }
            WorktreeMode::FromContext => {
                if let Some(Field::Text(path)) =
                    context.and_then(|c| context_field("worktreePath", c))
                {
                    if !path.is_empty() {
                        existing = Some(path);
                        use_worktree = None;
                    }
                }
            }
            _ => {}
        }

        let tasks = tasks_of(cfg);
        let projects: &[Value] = cfg
            .get("projects")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let project_named = |name: &str| {
            projects
                .iter()
                .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
        };
        let mut resolved_task: Option<Value> = None;
        let mut apply_task =
            |task: &Value, branch: &mut Option<String>, use_worktree: &mut Option<Value>| {
                let project_name = task
                    .get("projectName")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                prompt = Some(match project_named(project_name) {
                    Some(project) => {
                        let siblings: Vec<Value> = tasks
                            .iter()
                            .filter(|t| {
                                t.get("projectName").and_then(Value::as_str) == Some(project_name)
                            })
                            .cloned()
                            .collect();
                        task_prompt(task, project, &siblings)
                    }
                    None => text_of(task.get("description")).unwrap_or_default(),
                });
                task_id = text_of(task.get("id"));
                if existing.is_none() {
                    *branch = text_of(task.get("branch"))
                        .filter(|b| !b.is_empty())
                        .or(branch.take());
                    *use_worktree = task
                        .get("useWorktree")
                        .filter(|v| vorn_work::is_truthy(Some(v)))
                        .cloned()
                        .or(use_worktree.take());
                }
            };
        let static_task = config
            .get("taskId")
            .filter(|v| !v.is_null())
            .map(js::to_string);
        let effective_task =
            static_task.or_else(|| context.and_then(Context::task_id).map(str::to_owned));
        if let Some(id) = effective_task.filter(|id| !id.is_empty()) {
            let found = tasks
                .iter()
                .find(|t| {
                    t.get("id").and_then(Value::as_str) == Some(id.as_str())
                        && !matches!(
                            t.get("status").and_then(Value::as_str),
                            Some("done" | "cancelled")
                        )
                })
                .cloned();
            if let Some(task) = found {
                apply_task(&task, &mut branch, &mut use_worktree);
                resolved_task = Some(task);
            }
        } else if vorn_work::is_truthy(config.get("taskFromQueue")) {
            let project = node.text("projectName").unwrap_or("");
            let mut queued: Vec<&Value> = tasks
                .iter()
                .filter(|t| {
                    t.get("projectName").and_then(Value::as_str) == Some(project)
                        && t.get("status").and_then(Value::as_str) == Some("todo")
                })
                .collect();
            queued.sort_by(|a, b| {
                let o = |t: &Value| t.get("order").and_then(Value::as_f64).unwrap_or(0.0);
                o(a).total_cmp(&o(b))
            });
            if let Some(task) = queued.first().map(|t| (*t).clone()) {
                apply_task(&task, &mut branch, &mut use_worktree);
                resolved_task = Some(task);
            }
        }

        let agent = self.effective_agent(config, context, resolved_task.as_ref(), cfg)?;

        let mut project_name = resolve(node.text("projectName").unwrap_or(""), context, None);
        let mut project_path = resolve(node.text("projectPath").unwrap_or(""), context, None);
        if project_name.is_empty() || project_path.is_empty() {
            let owner = context
                .and_then(|c| c.task.as_ref())
                .or(resolved_task.as_ref());
            if let Some(project) = owner
                .and_then(|t| t.get("projectName").and_then(Value::as_str))
                .and_then(project_named)
            {
                if project_name.is_empty() {
                    project_name = text_of(project.get("name")).unwrap_or_default();
                }
                if project_path.is_empty() {
                    project_path = text_of(project.get("path")).unwrap_or_default();
                }
            }
        }
        if !project_name.is_empty() && project_path.is_empty() {
            if let Some(project) = project_named(&project_name) {
                project_path = text_of(project.get("path")).unwrap_or_default();
            }
        }

        let schema = headless
            .then(|| {
                config
                    .get("outputSchema")
                    .filter(|s| vorn_work::is_truthy(Some(s)))
                    .cloned()
            })
            .flatten();
        let step_name = text_of(config.get("displayName"))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| node.label.clone());
        let prompt = prompt.map(|p| {
            if p.is_empty() {
                return p;
            }
            let p = resolve(&p, context, Some(outputs));
            if p.is_empty() {
                return p;
            }
            workflow_prompt(
                &workflow.id,
                &workflow.name,
                &step_name,
                &p,
                schema.as_ref(),
            )
        });
        let inherited = from_context;

        let mut payload = Map::new();
        put(&mut payload, "agentType", agent.clone().map(Value::String));
        payload.insert("projectName".into(), Value::String(project_name.clone()));
        payload.insert("projectPath".into(), Value::String(project_path.clone()));
        payload.insert("displayName".into(), Value::String(step_name));
        put(&mut payload, "branch", branch.map(Value::String));
        put(&mut payload, "useWorktree", use_worktree);
        put(
            &mut payload,
            "existingWorktreePath",
            existing.clone().map(Value::String),
        );
        put(
            &mut payload,
            "initialPrompt",
            prompt.clone().map(Value::String),
        );
        put(
            &mut payload,
            "promptDelayMs",
            config
                .get("promptDelayMs")
                .filter(|v| !v.is_null())
                .cloned(),
        );

        if !headless {
            put(
                &mut payload,
                "args",
                config.get("args").filter(|v| !v.is_null()).cloned(),
            );
            put(
                &mut payload,
                "model",
                config.get("model").filter(|v| !v.is_null()).cloned(),
            );
            let remote = project_named(&project_name).and_then(remote_host_of);
            put(&mut payload, "remoteHostId", remote.map(Value::String));
            let session = self
                .inner
                .host
                .call("terminal:create", Value::Object(payload))
                .await?;
            let session_id = session.get("id").map(js::to_string).unwrap_or_default();
            if let (Some(task), Some(agent)) = (&task_id, &agent) {
                self.take_task(task, &session_id, agent).await;
            }
            let worktree = text_of(session.get("worktreePath"));
            self.finish(run, node, |s| {
                s.set_status(Status::Success);
                s.session_id = Some(session_id.clone());
                s.logs = Some(format!("Terminal session created: {session_id}"));
                s.task_id = task_id.clone();
                s.worktree_origin =
                    worktree_origin(worktree.as_deref(), inherited).map(str::to_owned);
                s.worktree_path = worktree;
                s.worktree_name = text_of(session.get("worktreeName"));
                s.agent_type = agent.clone().map(Into::into);
                s.project_name = Some(project_name);
                s.project_path = Some(project_path);
            });
            return Ok(());
        }

        payload.insert("headless".into(), Value::Bool(true));
        payload.insert("workflowId".into(), Value::String(workflow.id.clone()));
        payload.insert("workflowName".into(), Value::String(workflow.name.clone()));
        put(
            &mut payload,
            "args",
            config.get("args").filter(|v| !v.is_null()).cloned(),
        );
        put(
            &mut payload,
            "model",
            config.get("model").filter(|v| !v.is_null()).cloned(),
        );

        let mut diag = Diagnostics::new();
        let at = existing
            .clone()
            .filter(|p| !p.is_empty())
            .or_else(|| nonempty(project_path.clone()))
            .unwrap_or_else(|| "(no path)".into());
        diag.note(format!(
            "Launching {} in {at}{}",
            agent.as_deref().unwrap_or("undefined"),
            match prompt.as_deref().filter(|p| !p.is_empty()) {
                Some(p) => format!(" with a {}-line prompt", p.split('\n').count()),
                None => " with no prompt".into(),
            }
        ));
        lock(run).update(&node.id, |s| s.diagnostics = Some(diag.text()));
        self.publish(run);

        let timeout_ms = self.step_timeout_ms(config, cfg);
        let result = self
            .wait_headless(
                node,
                run,
                active,
                Value::Object(payload),
                &mut diag,
                timeout_ms,
                task_id.as_deref(),
                agent.as_deref(),
                inherited,
                &project_name,
                &project_path,
            )
            .await;
        lock(run).update(&node.id, |s| s.diagnostics = Some(diag.text()));
        result
    }

    /// Starts a headless agent and waits for it to exit, time out or be
    /// stopped. An exit seen before its id is known is kept for it.
    #[allow(clippy::too_many_arguments)]
    async fn wait_headless(
        &self,
        node: &Node,
        run: &Run,
        active: &Arc<Active>,
        payload: Value,
        diag: &mut Diagnostics,
        timeout_ms: u64,
        task_id: Option<&str>,
        agent: Option<&str>,
        inherited: bool,
        project_name: &str,
        project_path: &str,
    ) -> Result<(), String> {
        let mut notes = self.inner.host.notes();
        let create = self.inner.host.call("headless:create", payload);
        tokio::pin!(create);
        let mut early_exits: HashMap<String, i64> = HashMap::new();
        let mut early_data: Vec<(String, String)> = Vec::new();
        let created = loop {
            tokio::select! {
                created = &mut create => break created,
                note = notes.recv() => match note {
                    Ok(Note::HeadlessExit { id, code }) => { early_exits.insert(id, code); }
                    Ok(Note::HeadlessData { id, data }) => early_data.push((id, data)),
                    Err(RecvError::Closed) => break (&mut create).await,
                    _ => {}
                },
            }
        };
        // What was said while the answer was on its way.
        while let Ok(note) = notes.try_recv() {
            match note {
                Note::HeadlessExit { id, code } => {
                    early_exits.insert(id, code);
                }
                Note::HeadlessData { id, data } => early_data.push((id, data)),
                Note::ScriptData { .. } => {}
            }
        }
        let session = match created {
            Ok(session) => session,
            Err(message) => {
                diag.note(format!("Could not start: {message}"));
                return Err(message);
            }
        };
        let session_id = session.get("id").map(js::to_string).unwrap_or_default();
        locked_insert(active, &session_id);
        let pid = session
            .get("pid")
            .filter(|p| vorn_work::is_truthy(Some(p)))
            .map_or_else(|| "unknown".to_owned(), js::to_string);
        let command = session
            .get("launchCommand")
            .filter(|c| vorn_work::is_truthy(Some(c)))
            .map(|c| format!(": {}", js::to_string(c)))
            .unwrap_or_default();
        diag.note(format!("Session {session_id} started (pid {pid}){command}"));
        if timeout_ms > 0 {
            diag.note(format!(
                "Will give up after {} min without an exit",
                (timeout_ms as f64 / 60_000.0).round()
            ));
        }
        let mut outcome = None;
        if let Some(code) = early_exits.get(&session_id) {
            diag.note(format!(
                "Agent had already exited (code {code}) before its id reached us"
            ));
            outcome = Some(Outcome::Exit(*code));
        }
        let worktree = text_of(session.get("worktreePath"));
        lock(run).update(&node.id, |s| {
            s.session_id = Some(session_id.clone());
            s.task_id = task_id.map(str::to_owned);
            s.worktree_origin = worktree_origin(worktree.as_deref(), inherited).map(str::to_owned);
            s.worktree_path = worktree.clone();
            s.worktree_name = text_of(session.get("worktreeName"));
            s.agent_type = agent.map(|a| a.to_owned().into());
            s.project_name = Some(project_name.to_owned());
            s.project_path = Some(project_path.to_owned());
            if let Some(conv) = session
                .get("agentSessionId")
                .filter(|c| vorn_work::is_truthy(Some(c)))
            {
                s.agent_session_id = Some(js::to_string(conv));
            }
        });
        self.persist(run);
        if let (Some(task), Some(agent)) = (task_id, agent) {
            self.take_task(task, &session_id, agent).await;
        }

        let mut logs = String::new();
        let mut bytes: usize = 0;
        let mut persisted_len = 0;
        let mut save_at: Option<tokio::time::Instant> = None;
        let deadline = (timeout_ms > 0)
            .then(|| tokio::time::Instant::now() + Duration::from_millis(timeout_ms));
        for (id, data) in early_data {
            if id == session_id {
                self.agent_spoke(node, run, diag, &mut logs, &mut bytes, &data);
                save_at.get_or_insert_with(|| tokio::time::Instant::now() + PERSIST_INTERVAL);
            }
        }
        let outcome = match outcome {
            Some(o) => o,
            None => loop {
                let save_sleep = async {
                    match save_at {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                let timeout_sleep = async {
                    match deadline {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    note = notes.recv() => match note {
                        Ok(Note::HeadlessExit { id, code }) if id == session_id => break Outcome::Exit(code),
                        Ok(Note::HeadlessData { id, data }) if id == session_id => {
                            self.agent_spoke(node, run, diag, &mut logs, &mut bytes, &data);
                            save_at.get_or_insert_with(|| tokio::time::Instant::now() + PERSIST_INTERVAL);
                        }
                        Err(RecvError::Closed) => break Outcome::Stopped,
                        _ => {}
                    },
                    () = save_sleep => {
                        save_at = None;
                        if logs.len() != persisted_len {
                            persisted_len = logs.len();
                            self.save(run);
                        }
                    }
                    () = timeout_sleep => break Outcome::Timeout(timeout_ms),
                    () = active.cancel.cancelled() => break Outcome::Stopped,
                }
            },
        };

        match outcome {
            Outcome::Stopped => {
                diag.note("Stopped by user");
                Ok(())
            }
            Outcome::Timeout(after) => {
                let minutes = (after as f64 / 60_000.0).round() as u64;
                let plural = if minutes == 1 { "" } else { "s" };
                let reason = if bytes == 0 {
                    format!("Step timed out after {minutes} minute{plural}. The agent was started but never produced any output, which usually means it never really ran or is waiting on input it will never get.")
                } else {
                    format!("Step timed out after {minutes} minute{plural} without the agent exiting, after {bytes} bytes of output.")
                };
                warn!(step = %node.label, %reason, session = %session_id, "an agent step timed out");
                diag.note(&reason);
                match self
                    .inner
                    .host
                    .call("headless:kill", Value::String(session_id.clone()))
                    .await
                {
                    Ok(_) => diag.note("Agent killed"),
                    Err(err) => diag.note(format!("Could not kill the agent: {err}")),
                }
                let text = diag.text();
                self.finish(run, node, |s| {
                    s.set_status(Status::Error);
                    s.output = Some(logs.clone());
                    s.logs = Some(logs);
                    s.error = Some(reason);
                    s.diagnostics = Some(text);
                });
                if let Some(task) = task_id {
                    self.reopen_task(task).await;
                }
                Ok(())
            }
            Outcome::Exit(code) => {
                diag.note(format!(
                    "Agent exited with code {code} after {bytes} bytes of output{}",
                    if bytes == 0 {
                        " — it produced nothing at all"
                    } else {
                        ""
                    }
                ));
                if code != 0 {
                    logs.push_str(&format!("\nProcess exited with code {code}"));
                }
                let schema = node
                    .config
                    .get("outputSchema")
                    .filter(|s| vorn_work::is_truthy(Some(s)));
                let (typed, schema_error) = match (code, schema) {
                    (0, Some(schema)) => match structured::extract(&logs, schema) {
                        Ok(obj) => (Some(Value::Object(obj)), None),
                        Err(e) => (None, Some(e)),
                    },
                    _ => (None, None),
                };
                if let Some(e) = &schema_error {
                    diag.note(format!("Output did not match the declared schema: {e}"));
                }
                let failed = code != 0 || schema_error.is_some();
                let text = diag.text();
                self.finish(run, node, |s| {
                    s.diagnostics = Some(text);
                    s.set_status(if failed {
                        Status::Error
                    } else {
                        Status::Success
                    });
                    s.output = Some(logs.clone());
                    s.logs = Some(logs);
                    if typed.is_some() {
                        s.structured_output = typed;
                    }
                    if code != 0 {
                        s.error = Some(format!("Exit code {code}"));
                    } else if let Some(e) = schema_error {
                        s.error = Some(e);
                    }
                });
                if failed {
                    if let Some(task) = task_id {
                        self.reopen_task(task).await;
                    }
                }
                Ok(())
            }
        }
    }

    fn agent_spoke(
        &self,
        node: &Node,
        run: &Run,
        diag: &mut Diagnostics,
        logs: &mut String,
        bytes: &mut usize,
        data: &str,
    ) {
        if *bytes == 0 {
            diag.note(format!(
                "First output from the agent ({} bytes)",
                js::utf16_len(data)
            ));
            let text = diag.text();
            lock(run).update(&node.id, |s| s.diagnostics = Some(text));
        }
        *bytes += js::utf16_len(data);
        append_bounded_log(logs, data);
        lock(run).update(&node.id, |s| s.logs = Some(logs.clone()));
        self.publish(run);
    }

    /// `resolveEffectiveAgent`: a concrete agent as it is; `fromTask` reads
    /// the trigger's task, then the step's, then the default.
    fn effective_agent(
        &self,
        config: &Value,
        context: Option<&Context>,
        task: Option<&Value>,
        cfg: &Value,
    ) -> Result<Option<String>, String> {
        let agent = config.get("agentType");
        if agent.and_then(Value::as_str) != Some("fromTask") {
            return Ok(agent.filter(|a| !a.is_null()).map(js::to_string));
        }
        if config.get("model").is_some_and(|m| !m.is_null()) {
            return Err(
                "A model needs a concrete agent. Clear the model or choose an agent for this step."
                    .into(),
            );
        }
        let assigned = |t: Option<&Value>| {
            t.and_then(|t| t.get("assignedAgent"))
                .filter(|a| !a.is_null())
                .map(js::to_string)
        };
        Ok(Some(
            assigned(context.and_then(|c| c.task.as_ref()))
                .or_else(|| assigned(task))
                .or_else(|| {
                    cfg.get("defaults")
                        .and_then(|d| d.get("defaultAgent"))
                        .filter(|a| !a.is_null())
                        .map(js::to_string)
                })
                .unwrap_or_else(|| "claude".into()),
        ))
    }

    /// The step's own ceiling, else the configured default; 0 disables it.
    fn step_timeout_ms(&self, config: &Value, cfg: &Value) -> u64 {
        if let Some(ms) = config.get("timeoutMs").and_then(Value::as_f64) {
            return ms.max(0.0) as u64;
        }
        let minutes = cfg
            .get("defaults")
            .and_then(|d| d.get("headlessStepTimeoutMinutes"))
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_STEP_TIMEOUT_MINUTES);
        if minutes > 0.0 {
            (minutes * 60_000.0) as u64
        } else {
            0
        }
    }

    async fn take_task(&self, task: &str, session: &str, agent: &str) {
        if let Some(moved) = self.inner.host.take_task(task, session, agent).await {
            if moved.from != "in_progress" {
                self.fire_task_status_changed(&moved.task, &moved.from, "in_progress")
                    .await;
            }
        }
    }

    async fn reopen_task(&self, task: &str) {
        if let Some(moved) = self.inner.host.reopen_task(task).await {
            if moved.from != "todo" {
                self.fire_task_status_changed(&moved.task, &moved.from, "todo")
                    .await;
            }
        }
    }
}

fn locked_insert(active: &Arc<Active>, session: &str) {
    active
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session.to_owned());
}

/// A project's first host that is not this machine.
fn remote_host_of(project: &Value) -> Option<String> {
    let ids = project
        .get("hostIds")
        .and_then(Value::as_array)
        .filter(|ids| !ids.is_empty())?;
    ids.iter()
        .filter_map(Value::as_str)
        .find(|id| *id != "local")
        .map(str::to_owned)
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
