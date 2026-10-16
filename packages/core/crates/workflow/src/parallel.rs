//! A `forEach` loop run in parallel (`parallel = true`): every item gets a
//! pass of the loop's body, at most `concurrency` passes at once. Each pass
//! runs the body's steps under ids of their own, `<step>#<n>`, so the run
//! records, and its view shows, each item's steps beside the others'.
//!
//! A failing item stops new items from starting; those already running
//! finish. Results are kept per item, in the list's order, whatever order
//! the items finished in; afterwards each body step holds what the last
//! item left, as after a loop run one item at a time.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use vorn_work::graph::{self, item_node_id, output_or_logs, Body};
use vorn_work::js::iso_now;
use vorn_work::model::{Context, Edge, LoopPass, Node, RunExt, StateExt, Status, Workflow};

use crate::engine::{lock, Active, Engine, Run};
use crate::host::Host;
use crate::steps::{cap, pass_result};
use crate::waves::{BoxFuture, Mode, Waves};

/// Passes at once when the loop names no limit.
pub const DEFAULT_CONCURRENCY: usize = 4;

/// The most passes at once, whatever a loop asks for.
pub const MAX_CONCURRENCY: usize = 16;

/// How many passes a loop runs at once: its `concurrency`, from 1 to
/// [`MAX_CONCURRENCY`], else [`DEFAULT_CONCURRENCY`].
pub fn concurrency(config: &Value) -> usize {
    match graph::js_number(config.get("concurrency")) {
        Some(n) if n.is_finite() && n >= 1.0 => (n.floor() as usize).min(MAX_CONCURRENCY),
        _ => DEFAULT_CONCURRENCY,
    }
}

/// Whether a loop runs its items side by side.
pub fn is_parallel(config: &Value) -> bool {
    config.get("mode").and_then(Value::as_str) == Some("forEach")
        && config.get("parallel").and_then(Value::as_bool) == Some(true)
}

/// What became of one item.
enum Pass {
    /// Stopped before it started.
    NotRun,
    Ran {
        walked: Result<(), String>,
        /// The body step that failed it, when one did and that ends the run.
        failed: Option<String>,
        exit: String,
        result: Value,
        lines: Vec<String>,
    },
}

/// The workflow as item `number`'s pass sees it: the body's steps under
/// that item's ids, so the outputs it reads are its own.
fn item_workflow(workflow: &Workflow, members: &HashSet<&str>, number: u32) -> Workflow {
    let rename = |id: &mut String| {
        if members.contains(id.as_str()) {
            *id = item_node_id(id, number);
        }
    };
    let mut wf = workflow.clone();
    for n in &mut wf.nodes {
        rename(&mut n.id);
    }
    for e in &mut wf.edges {
        rename(&mut e.source);
        rename(&mut e.target);
    }
    wf
}

impl<H: Host> Engine<H> {
    /// Runs loop `node`'s body once per item of `items`, several at once.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn execute_parallel(
        &self,
        node: &Node,
        workflow: &Workflow,
        run: &Run,
        context: Option<&Context>,
        active: &Arc<Active>,
        body: &Body<'_>,
        items: Vec<Value>,
    ) -> Result<(), String> {
        let count = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let limit = concurrency(&node.config);
        let members: HashSet<&str> = body.members.iter().map(|m| m.id.as_str()).collect();
        let fed: HashSet<&str> = body.edges.iter().map(|e| e.source.as_str()).collect();
        {
            let mut r = lock(run);
            for number in 1..=count {
                for m in &body.members {
                    let id = item_node_id(&m.id, number);
                    if r.node_state(&id).is_none() {
                        r.node_states
                            .push(graph::blank_pass_state(&id, Some(number)));
                    }
                }
            }
            r.update(&node.id, |s| {
                s.set_status(Status::Running);
                s.started_at = Some(iso_now());
                s.iteration = Some(0.0);
            });
        }
        self.persist(run);

        let stop = AtomicBool::new(false);
        // Built up front and boxed, so the stream holds plain futures; each starts when polled.
        let pending: Vec<BoxFuture<'_, (usize, Pass)>> = items
            .iter()
            .enumerate()
            .map(|(index, item)| -> BoxFuture<'_, (usize, Pass)> {
                let (members, fed, stop) = (&members, &fed, &stop);
                Box::pin(async move {
                    if active.cancel.is_cancelled() || stop.load(Ordering::Relaxed) {
                        return (index, Pass::NotRun);
                    }
                    let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
                    let wf = item_workflow(workflow, members, number);
                    let pass = self
                        .item_pass(node, &wf, run, context, active, body, fed, item, number, count)
                        .await;
                    if matches!(&pass, Pass::Ran { failed: Some(_), .. }) {
                        stop.store(true, Ordering::Relaxed);
                    }
                    (index, pass)
                })
            })
            .collect();
        let passes: Vec<(usize, Pass)> = stream::iter(pending)
            .buffer_unordered(limit)
            .collect()
            .await;
        let mut passes = passes;
        passes.sort_by_key(|(index, _)| *index);
        self.finish_parallel(node, run, active, body, &items, passes)
    }

    /// One item's pass, walked as the body's steps under that item's ids.
    #[allow(clippy::too_many_arguments)]
    async fn item_pass(
        &self,
        node: &Node,
        wf: &Workflow,
        run: &Run,
        context: Option<&Context>,
        active: &Arc<Active>,
        body: &Body<'_>,
        fed: &HashSet<&str>,
        item: &Value,
        number: u32,
        count: u32,
    ) -> Pass {
        let ids: Vec<String> = body
            .members
            .iter()
            .map(|m| item_node_id(&m.id, number))
            .collect();
        let steps: Vec<&Node> = ids.iter().filter_map(|id| wf.node(id)).collect();
        let renamed = |id: &str| item_node_id(id, number);
        let mut edges: Vec<Edge> = body
            .entries
            .iter()
            .map(|id| Edge {
                id: format!("{}->{}:entry", node.id, renamed(id)),
                source: node.id.clone(),
                target: renamed(id),
                branch: None,
            })
            .collect();
        edges.extend(body.edges.iter().map(|e| Edge {
            id: e.id.clone(),
            source: renamed(&e.source),
            target: renamed(&e.target),
            branch: e.branch.clone(),
        }));
        let mut pass = context.cloned().unwrap_or_default();
        pass.pass = Some(LoopPass {
            item: Some(item.clone()),
            index: number - 1,
            number,
            count: Some(count),
        });
        let walked = self
            .run_waves(
                wf,
                run,
                Some(&pass),
                active,
                Waves {
                    nodes: steps.clone(),
                    edges,
                    roots: vec![node.id.clone()],
                    max_waves: 50 * steps.len(),
                    stagger: None,
                },
                Mode::Pass {
                    context: &pass,
                    iteration: number,
                },
            )
            .await
            .map(|_| ());
        let stopped = active.cancel.is_cancelled();
        let mut r = lock(run);
        let now = iso_now();
        let mut lines = vec![format!("── item {number} of {count} ──")];
        for step in &steps {
            r.update(&step.id, |s| {
                if s.is(Status::Pending) || s.is(Status::Running) {
                    s.set_status(Status::Skipped);
                    s.completed_at = Some(now.clone());
                    s.iteration = Some(f64::from(number));
                    if stopped {
                        s.error = Some("Skipped: the run was stopped".into());
                    }
                }
            });
            let status = r
                .node_state(&step.id)
                .map_or_else(|| "unknown".to_owned(), |s| s.status.0.clone());
            lines.push(format!("  {}: {status}", step.label));
        }
        let failed = steps
            .iter()
            .find(|m| {
                r.node_state(&m.id).is_some_and(|s| s.is(Status::Error)) && m.stops_run_on_error()
            })
            .map(|m| m.label.clone());
        let exit = body
            .members
            .iter()
            .filter(|m| !fed.contains(m.id.as_str()))
            .filter_map(|m| r.node_state(&renamed(&m.id)))
            .find(|s| s.is(Status::Success) || s.is(Status::Error))
            .map(|s| cap(output_or_logs(s)).to_owned())
            .unwrap_or_default();
        let result = pass_result(&r, number, Some(item), &steps, failed.is_some());
        drop(r);
        self.persist(run);
        Pass::Ran {
            walked,
            failed,
            exit,
            result,
            lines,
        }
    }

    /// The loop's own state once every item is done or stopped.
    fn finish_parallel(
        &self,
        node: &Node,
        run: &Run,
        active: &Arc<Active>,
        body: &Body<'_>,
        items: &[Value],
        passes: Vec<(usize, Pass)>,
    ) -> Result<(), String> {
        let count = passes.len();
        let mut summary = Vec::new();
        let mut results = Vec::with_capacity(count);
        let mut outputs = Vec::with_capacity(count);
        let mut failed: Option<(String, usize)> = None;
        let mut walk_error: Option<String> = None;
        let mut last_ran: Option<u32> = None;
        let stopped = active.cancel.is_cancelled();
        let mut r = lock(run);
        let now = iso_now();
        for (index, pass) in passes {
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            match pass {
                Pass::Ran {
                    walked,
                    failed: f,
                    exit,
                    result,
                    lines,
                } => {
                    last_ran = Some(number);
                    summary.extend(lines);
                    results.push(result);
                    outputs.push(Value::String(exit));
                    if let (Some(label), None) = (f, &failed) {
                        failed = Some((label, index + 1));
                    }
                    if let (Err(e), None) = (walked, &walk_error) {
                        walk_error = Some(e);
                    }
                }
                Pass::NotRun => {
                    let why = if stopped {
                        "Skipped: the run was stopped"
                    } else {
                        "Skipped: an earlier item failed"
                    };
                    summary.push(format!("── item {number} of {count} ── not run"));
                    for m in &body.members {
                        r.update(&item_node_id(&m.id, number), |s| {
                            s.set_status(Status::Skipped);
                            s.completed_at = Some(now.clone());
                            s.error = Some(why.into());
                        });
                    }
                    let mut result = json!({ "index": index, "status": "skipped", "steps": {} });
                    if let Some(item) = items.get(index) {
                        result["item"] = item.clone();
                    }
                    results.push(result);
                    outputs.push(Value::String(String::new()));
                }
            }
        }
        // Each body step holds what the last item left, as a loop of one item at a time does.
        if let Some(number) = last_ran {
            for m in &body.members {
                let last = r.node_state(&item_node_id(&m.id, number)).cloned();
                if let Some(mut state) = last {
                    state.node_id = m.id.clone();
                    if let Some(s) = r.node_state_mut(&m.id) {
                        *s = state;
                    }
                }
            }
        }
        let passes = results
            .iter()
            .filter(|r| r.get("status").and_then(Value::as_str) != Some("skipped"))
            .count();
        let stop_reason = match (&failed, stopped) {
            (Some((label, n)), _) => format!("\"{label}\" failed on item {n}"),
            (None, true) => "the run was stopped".to_owned(),
            (None, false) => format!("went through all {count} item(s)"),
        };
        summary.push(format!(
            "Stopped after {passes} item(s), in parallel: {stop_reason}."
        ));
        drop(r);
        if let Some(error) = walk_error {
            return Err(error);
        }
        let failed = failed.is_some();
        self.finish(run, node, |s| {
            s.set_status(if failed {
                Status::Error
            } else {
                Status::Success
            });
            s.iteration = Some(passes as f64);
            s.output = Some(passes.to_string());
            s.logs = Some(summary.join("\n"));
            s.structured_output = Some(json!({
                "passes": passes,
                "count": count,
                "results": results,
                "outputs": outputs,
            }));
            if failed {
                s.error = Some(stop_reason);
            }
        });
        Ok(())
    }
}
