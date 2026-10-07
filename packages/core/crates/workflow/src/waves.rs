//! One graph, run in waves (`graph-runner.ts`): every step whose
//! predecessors have settled runs, together, then the next wave. The main
//! run drives the workflow through this, and a loop its body once per pass,
//! so a condition inside a loop branches exactly as one outside it does.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::{info, warn};
use vorn_work::graph::{
    collect_skipped_branch, is_sign_in_wait, skip_entry_points, step_outputs, Graph,
};
use vorn_work::js::iso_now;
use vorn_work::model::{state, Context, Edge, Node, NodeKind, RunExt, StateExt, Status, Workflow};

use crate::engine::{lock, Active, Engine, Run};
use crate::host::Host;

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The steps one walk schedules.
pub(crate) struct Waves<'a> {
    pub(crate) nodes: Vec<&'a Node>,
    /// Edges among those steps, and from the roots.
    pub(crate) edges: Vec<Edge>,
    /// Treated as already complete: the loop, when it runs its body.
    pub(crate) roots: Vec<String>,
    /// Past this many waves the graph is spinning, and the run says so.
    pub(crate) max_waves: usize,
    pub(crate) stagger: Option<Duration>,
}

/// Which walk a step runs in.
#[derive(Clone, Copy)]
pub(crate) enum Mode<'a> {
    Main,
    /// A loop's pass, with its context and number.
    Pass {
        context: &'a Context,
        iteration: u32,
    },
}

/// What a wave knows of each step.
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
}

impl<H: Host> Engine<H> {
    /// Walks `waves`; returns the steps it skipped, which decide whether
    /// their failures count against the run.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_waves<'a>(
        &'a self,
        workflow: &'a Workflow,
        run: &'a Run,
        context: Option<&'a Context>,
        active: &'a Arc<Active>,
        waves: Waves<'a>,
        mode: Mode<'a>,
    ) -> BoxFuture<'a, Result<HashSet<String>, String>> {
        Box::pin(async move {
            let graph = Graph::of(&waves.edges);
            let roots: HashSet<&str> = waves.roots.iter().map(String::as_str).collect();
            let seen = Mutex::new(Seen::default());
            let mut wave = 0;
            while !active.cancel.is_cancelled() {
                let batch: Vec<&Node> = {
                    let mut s = seen.lock().unwrap_or_else(|e| e.into_inner());
                    *s = Seen::default();
                    for id in &roots {
                        s.completed.insert((*id).to_owned());
                    }
                    let r = lock(run);
                    for ns in &r.node_states {
                        match ns.status() {
                            Some(Status::Success | Status::Error) => {
                                s.completed.insert(ns.node_id.clone());
                            }
                            Some(Status::Skipped) => s.skip(ns.node_id.clone()),
                            _ => {}
                        }
                    }
                    waves
                        .nodes
                        .iter()
                        .copied()
                        .filter(|n| n.kind != NodeKind::Trigger && !roots.contains(n.id.as_str()))
                        .filter(|n| !s.settled(&n.id))
                        .filter(|n| !r.node_state(&n.id).is_some_and(|ns| ns.is(Status::Waiting)))
                        .filter(|n| {
                            let preds = graph.predecessors_of(&n.id);
                            preds.iter().all(|p| s.settled(p))
                                && preds.iter().any(|p| s.completed.contains(p))
                        })
                        .collect()
                };
                if batch.is_empty() {
                    break;
                }
                wave += 1;
                if wave > waves.max_waves {
                    return Err(format!(
                        "Stopped after {} waves: steps kept becoming ready again",
                        waves.max_waves
                    ));
                }
                info!(
                    wave,
                    steps = %batch.iter().map(|n| n.label.as_str()).collect::<Vec<_>>().join(", "),
                    "a wave of steps runs"
                );
                if let (true, Some(stagger)) = (wave > 1, waves.stagger) {
                    tokio::time::sleep(stagger).await;
                }
                let outputs = step_outputs(&lock(run), workflow);
                let stateless: Mutex<Vec<String>> = Mutex::default();
                let steps = batch.iter().map(|node| {
                    let outputs = &outputs;
                    let seen = &seen;
                    let graph = &graph;
                    let edges = &waves.edges;
                    let stateless = &stateless;
                    async move {
                        let result = match mode {
                            Mode::Main => {
                                self.execute_node(node, workflow, run, context, outputs, active)
                                    .await
                            }
                            Mode::Pass {
                                context: pass,
                                iteration,
                            } => {
                                self.run_pass_step(
                                    node, workflow, run, pass, outputs, active, iteration,
                                )
                                .await
                            }
                        };
                        if let Err(message) = result {
                            warn!(step = %node.label, %message, "a step failed");
                            lock(run).update(&node.id, |s| {
                                s.set_status(Status::Error);
                                s.completed_at = Some(iso_now());
                                s.error = Some(message);
                            });
                            self.persist(run);
                        }
                        self.settle_step(node, run, seen, graph, edges, stateless);
                    }
                });
                futures_util::future::join_all(steps).await;
                let stateless = stateless.into_inner().unwrap_or_else(|e| e.into_inner());
                if let Some(first) = stateless.into_iter().next() {
                    return Err(first);
                }
            }
            let s = seen.into_inner().unwrap_or_else(|e| e.into_inner());
            Ok(s.skipped_set)
        })
    }

    /// What one finished step means for the rest: nothing while it waits, a
    /// failure skips what it feeds, and a condition skips the branch it did
    /// not take.
    fn settle_step(
        &self,
        node: &Node,
        run: &Run,
        seen: &Mutex<Seen>,
        graph: &Graph,
        edges: &[Edge],
        stateless: &Mutex<Vec<String>>,
    ) {
        let mut s = seen.lock().unwrap_or_else(|e| e.into_inner());
        let (status, output) = {
            let mut r = lock(run);
            match r.node_state(&node.id) {
                Some(ns) => (ns.status(), ns.output.clone()),
                None => {
                    let error = format!(
                        "Step \"{}\" has no state in this run; its workflow changed under it",
                        node.label
                    );
                    let mut lost = state(&node.id, Status::Error);
                    lost.completed_at = Some(iso_now());
                    lost.error = Some(error.clone());
                    r.node_states.push(lost);
                    stateless
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(error);
                    return;
                }
            }
        };
        if status == Some(Status::Waiting) {
            return;
        }
        s.completed.insert(node.id.clone());

        if status == Some(Status::Error) && node.stops_run_on_error() {
            let entries = skip_entry_points(&node.id, edges, graph, |id| s.settled(id));
            for entry in entries {
                for id in collect_skipped_branch(&entry, graph, |id| s.settled(id)) {
                    s.skip(id);
                }
            }
            self.stamp_skipped(run, &s, true, |ns| {
                ns.error = Some(format!("Skipped: \"{}\" failed", node.label));
            });
            return;
        }
        if node.kind != NodeKind::Condition {
            return;
        }
        // A condition that failed answered nothing, so neither branch is the one it chose.
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
            for id in collect_skipped_branch(&edge.target, graph, |id| s.settled(id)) {
                s.skip(id);
            }
            any = true;
        }
        if any {
            self.stamp_skipped(run, &s, false, |ns| ns.skip_reason = Some("branch".into()));
        }
    }

    fn stamp_skipped(
        &self,
        run: &Run,
        seen: &Seen,
        only_pending: bool,
        extra: impl Fn(&mut vorn_protocol::NodeExecutionState),
    ) {
        {
            let mut r = lock(run);
            let now = iso_now();
            for id in &seen.skipped {
                r.update(id, |ns| {
                    if only_pending && !ns.is(Status::Pending) {
                        return;
                    }
                    ns.set_status(Status::Skipped);
                    ns.completed_at = Some(now.clone());
                    extra(ns);
                });
            }
        }
        self.persist(run);
    }

    /// A body step on a pass: marked running, run, a sign-in wait turned
    /// into a failure (a loop cannot wait), and its pass recorded.
    #[allow(clippy::too_many_arguments)]
    async fn run_pass_step(
        &self,
        node: &Node,
        workflow: &Workflow,
        run: &Run,
        pass: &Context,
        outputs: &vorn_work::template::StepOutputs,
        active: &Arc<Active>,
        iteration: u32,
    ) -> Result<(), String> {
        lock(run).update(&node.id, |s| {
            s.set_status(Status::Running);
            s.started_at = Some(iso_now());
        });
        self.persist(run);
        self.execute_node(node, workflow, run, Some(pass), outputs, active)
            .await?;
        {
            let mut r = lock(run);
            let wait = r.node_state(&node.id).is_some_and(is_sign_in_wait);
            r.update(&node.id, |s| {
                if wait {
                    s.set_status(Status::Error);
                    s.waiting_for = None;
                    s.completed_at = Some(iso_now());
                    s.error = Some("Its connection was signed out inside a loop, which cannot wait. Sign in, then run the workflow again.".into());
                }
                s.iteration = Some(f64::from(iteration));
            });
        }
        self.persist(run);
        Ok(())
    }
}
