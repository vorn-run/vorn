//! Runs: starting one, walking it, parking it on a gate, resuming it on an
//! answer or a sign-in, stopping it, and closing it out.
//!
//! A run is one shared [`WorkflowExecution`] behind a lock that is never
//! held across an await. A run being walked is "active" and has a handle
//! with its stop signal and the sessions it started; a run parked on a gate
//! or read back after a restart is loaded from the host and becomes active
//! again when it resumes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use vorn_protocol::WorkflowExecution;
use vorn_work::claims::{Claim, Claims};
use vorn_work::graph::{
    self, collapse_loop_bodies, collect_skipped_branch, dedupe_fingerprint, gate_edit_refusal,
    gate_key, is_sign_in_wait, loop_body_owners, nodes_between, run_ended_in_error,
    seed_retry_states, webhook_trigger_from_item, worktree_mode, Graph, WorktreeMode, ABANDONED,
};
use vorn_work::js::iso_now;
use vorn_work::model::{state, Context, NodeKind, RunExt, StateExt, Status, Workflow};

use crate::host::{Completion, Host, Source};
use crate::waves::{Mode, Waves};

/// A run, shared by the walk and whoever answers for it.
pub type Run = Arc<Mutex<WorkflowExecution>>;

/// Reads a run under its lock. The lock is never held across an await.
pub(crate) fn lock(run: &Run) -> MutexGuard<'_, WorkflowExecution> {
    run.lock().unwrap_or_else(|e| e.into_inner())
}

/// How often an agent that only prints is published.
const LOG_ONLY_INTERVAL: Duration = Duration::from_secs(3);

/// How often a run holding a connector inbox row renews its lease.
pub const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(60);

/// How often a run restored while its agent still ran is looked at again.
pub const RESTORED_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// What a step reads after a restart abandoned its run.
const RELOAD_ABANDONED: &str = "Renderer reload abandoned this run; re-run to continue";

/// A run being walked.
pub(crate) struct Active {
    pub(crate) cancel: CancellationToken,
    /// Headless sessions this run started, so a stop can end them.
    pub(crate) sessions: Mutex<HashSet<String>>,
    pub(crate) run: Run,
}

/// How a run was asked for.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub source: Option<Source>,
    /// Run only this step and the ones above it.
    pub target: Option<String>,
}

/// A run that exists, handed back before its walk ends.
#[derive(Debug)]
pub struct Started {
    /// The run as it stood when it started.
    pub run: WorkflowExecution,
    /// The walk, which ends when the run ends or parks; none when nothing
    /// new was started.
    pub done: Option<JoinHandle<()>>,
}

impl Started {
    /// Waits for the walk.
    pub async fn finish(self) -> WorkflowExecution {
        if let Some(done) = self.done {
            let _ = done.await;
        }
        self.run
    }
}

/// A reviewer's answer at a gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
    Changes,
}

impl Decision {
    pub fn parse(text: &str) -> Option<Decision> {
        match text {
            "approve" => Some(Decision::Approve),
            "reject" => Some(Decision::Reject),
            "changes" => Some(Decision::Changes),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Approve => "approve",
            Decision::Reject => "reject",
            Decision::Changes => "changes",
        }
    }
}

/// A comment on a gate's review page: the words it is about, when any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GateComment {
    pub quote: Option<String>,
    pub comment: String,
}

impl GateComment {
    /// The comments with words in them, each quote kept only when it names
    /// some (`cleanComments`).
    pub fn clean(list: &Value) -> Vec<GateComment> {
        list.as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let comment = c.get("comment").and_then(Value::as_str)?.trim();
                if comment.is_empty() {
                    return None;
                }
                let quote = c
                    .get("quote")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|q| !q.is_empty());
                Some(GateComment {
                    quote: quote.map(str::to_owned),
                    comment: comment.to_owned(),
                })
            })
            .collect()
    }

    fn json(&self) -> Value {
        match &self.quote {
            Some(q) => json!({ "quote": q, "comment": self.comment }),
            None => json!({ "comment": self.comment }),
        }
    }
}

/// An answer at a gate, as a client sends it.
#[derive(Clone, Debug)]
pub struct Answer {
    pub decision: Decision,
    pub comment: Option<String>,
    pub edited: Option<String>,
    /// The review page's comments, raw.
    pub comments: Option<Value>,
}

enum Save {
    Run(Box<WorkflowExecution>),
    Flush(oneshot::Sender<()>),
}

pub(crate) struct Inner<H> {
    pub(crate) host: H,
    saves: mpsc::UnboundedSender<Save>,
    pub(crate) active: Mutex<HashMap<String, Arc<Active>>>,
    claims: Mutex<Claims>,
    gate_timers: Mutex<HashMap<String, AbortHandle>>,
    heartbeats: Mutex<HashMap<String, AbortHandle>>,
    monitors: Mutex<HashMap<String, AbortHandle>>,
    /// Runs whose connector item a person settled, so it is not retried.
    terminal_decisions: Mutex<HashSet<String>>,
    published: Mutex<HashMap<String, (Instant, String)>>,
    /// Restore-triggered runs, one at a time per project, in order.
    pub(crate) restore_queues:
        Mutex<HashMap<String, mpsc::UnboundedSender<crate::triggers::Queued>>>,
}

/// The workflow engine. Cloning shares it.
pub struct Engine<H: Host> {
    pub(crate) inner: Arc<Inner<H>>,
}

impl<H: Host> Clone for Engine<H> {
    fn clone(&self) -> Self {
        Engine {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Which steps a run is on: what a viewer redraws for.
fn shape(run: &WorkflowExecution) -> String {
    let mut out = run.status.clone();
    out.push('|');
    for (i, ns) in run.node_states.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&ns.node_id);
        out.push(':');
        out.push_str(&ns.status.0);
    }
    out
}

impl<H: Host> Engine<H> {
    /// An engine on `host`. Starts the task that writes runs in order, so it
    /// is made inside a runtime.
    pub fn new(host: H) -> Engine<H> {
        let (saves, mut queue) = mpsc::unbounded_channel::<Save>();
        let inner = Arc::new(Inner {
            host,
            saves,
            active: Mutex::default(),
            claims: Mutex::default(),
            gate_timers: Mutex::default(),
            heartbeats: Mutex::default(),
            monitors: Mutex::default(),
            terminal_decisions: Mutex::default(),
            published: Mutex::default(),
            restore_queues: Mutex::default(),
        });
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(save) = queue.recv().await {
                match save {
                    Save::Run(run) => {
                        let Some(inner) = weak.upgrade() else { return };
                        inner.host.save_run(*run).await;
                    }
                    Save::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Engine { inner }
    }

    pub fn host(&self) -> &H {
        &self.inner.host
    }

    /// Waits until every run handed to the writer is written.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.inner.saves.send(Save::Flush(tx)).is_ok() {
            let _ = rx.await;
        }
    }

    /// The run being walked, else the stored one.
    pub async fn run_by_id(&self, run_id: &str) -> Option<Run> {
        if let Some(active) = locked(&self.inner.active).get(run_id) {
            return Some(Arc::clone(&active.run));
        }
        self.flush().await;
        self.inner
            .host
            .load_run(run_id)
            .await
            .map(|r| Arc::new(Mutex::new(r)))
    }

    /// The run as clients get it, now.
    pub async fn snapshot(&self, run_id: &str) -> Option<WorkflowExecution> {
        Some(lock(&self.run_by_id(run_id).await?).clone())
    }

    /// Whether `run_id` is being walked.
    pub fn is_active(&self, run_id: &str) -> bool {
        locked(&self.inner.active).contains_key(run_id)
    }

    /// How many runs are being walked.
    pub fn active_count(&self) -> usize {
        locked(&self.inner.active).len()
    }

    /// `workflowRun:claim`.
    pub fn claim(&self, workflow_id: &str, params: Option<&str>, window_ms: Option<i64>) -> Claim {
        let now = vorn_work_now();
        locked(&self.inner.claims).claim(workflow_id, params, window_ms, now, new_id)
    }

    /// `workflowRun:release`.
    pub fn release(&self, workflow_id: &str, params: Option<&str>, run_id: &str) {
        locked(&self.inner.claims).release(workflow_id, params, run_id);
    }

    /// `publishRun`: a step moving goes out at once; output alone at most
    /// every few seconds.
    pub(crate) fn publish(&self, run: &Run) {
        let snapshot = {
            let r = lock(run);
            let shape = shape(&r);
            let now = Instant::now();
            let mut published = locked(&self.inner.published);
            if let Some((at, previous)) = published.get(&r.run_id) {
                if *previous == shape && now.duration_since(*at) < LOG_ONLY_INTERVAL {
                    return;
                }
            }
            if r.is_running() {
                published.insert(r.run_id.clone(), (now, shape));
            } else {
                published.remove(&r.run_id);
            }
            r.clone()
        };
        self.inner.host.publish(&snapshot);
    }

    /// Writes the run behind the walk, in order, after telling clients.
    pub(crate) fn persist(&self, run: &Run) {
        self.publish(run);
        self.save(run);
    }

    /// Writes the run without telling anyone.
    pub(crate) fn save(&self, run: &Run) {
        let snapshot = lock(run).clone();
        let _ = self.inner.saves.send(Save::Run(Box::new(snapshot)));
    }

    /// Writes the run and waits until it is written.
    pub(crate) async fn save_now(&self, run: &Run) {
        self.save(run);
        self.flush().await;
    }

    pub(crate) async fn config(&self) -> Option<Value> {
        self.inner.host.config().await
    }

    /// The definition a run follows: its own snapshot, or the current one
    /// for runs saved before snapshots.
    pub(crate) async fn definition_of(&self, run: &Run) -> Option<Workflow> {
        let (stored, workflow_id) = {
            let r = lock(run);
            (r.definition.clone(), r.workflow_id.clone())
        };
        if let Some(def) = stored.filter(|d| !d.is_null()) {
            return Workflow::from_json(&def);
        }
        let config = self.config().await?;
        workflows_of(&config)
            .into_iter()
            .find(|w| w.id == workflow_id)
    }

    /// `executeWorkflow`.
    pub async fn execute(
        &self,
        workflow: &Workflow,
        context: Option<Context>,
        options: Options,
    ) -> Result<Started, String> {
        // A connector poll is the scheduler's to fan out; a click on Run asks it.
        if workflow.trigger_type() == Some("connectorPoll")
            && context.as_ref().is_none_or(|c| c.connector_item.is_none())
            && options.source != Some(Source::Scheduler)
        {
            let inputs = context
                .as_ref()
                .and_then(|c| c.inputs.clone())
                .map_or(Value::Null, Value::Object);
            let _ = self
                .inner
                .host
                .call(
                    "workflow:runManual",
                    json!({ "workflowId": workflow.id, "inputs": inputs }),
                )
                .await;
            let mut runs = self.inner.host.runs_of(&workflow.id).await;
            runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
            if let Some(latest) = runs
                .into_iter()
                .next()
                .filter(WorkflowExecution::is_running)
            {
                return Ok(Started {
                    run: latest,
                    done: None,
                });
            }
            let mut pending = blank_run(format!("pending:{}", workflow.id), workflow);
            pending.node_states = workflow
                .nodes
                .iter()
                .map(|n| {
                    state(
                        &n.id,
                        if n.kind == NodeKind::Trigger {
                            Status::Success
                        } else {
                            Status::Pending
                        },
                    )
                })
                .collect();
            pending.definition = None;
            return Ok(Started {
                run: pending,
                done: None,
            });
        }

        let fingerprint = dedupe_fingerprint(context.as_ref());
        let dedupe = match &options.target {
            Some(target) => format!("{fingerprint}:target:{target}"),
            None => fingerprint,
        };
        let run = match self.claim_new_run(workflow, context.as_ref(), &options, dedupe) {
            Ok(run) => run,
            Err(holder) => {
                if let Some(existing) = self.snapshot(&holder).await {
                    return Ok(Started {
                        run: existing,
                        done: None,
                    });
                }
                return Err(format!(
                    "Workflow \"{}\" is already running for this trigger",
                    workflow.name
                ));
            }
        };
        Ok(self.walk(workflow.clone(), run, context, options.source))
    }

    /// Claims the trigger and writes the new run under one lock, so a second
    /// trigger told the run's id can always read the run back. The holder's
    /// run id when the trigger is already claimed.
    fn claim_new_run(
        &self,
        workflow: &Workflow,
        context: Option<&Context>,
        options: &Options,
        dedupe: String,
    ) -> Result<Run, String> {
        let mut claims = locked(&self.inner.claims);
        let claim = claims.claim(&workflow.id, Some(&dedupe), None, vorn_work_now(), new_id);
        if !claim.granted {
            warn!(workflow = %workflow.name, params = %dedupe, "a trigger already claimed was skipped");
            return Err(claim.run_id);
        }
        let slice: Option<HashSet<String>> = options.target.as_ref().map(|target| {
            std::iter::once(target.clone())
                .chain(
                    graph::ancestors(&workflow.nodes, &workflow.edges, target)
                        .into_iter()
                        .map(|n| n.id.clone()),
                )
                .collect()
        });
        let mut run = blank_run(claim.run_id, workflow);
        run.node_states = workflow
            .nodes
            .iter()
            .map(|n| {
                if n.kind == NodeKind::Trigger {
                    return state(&n.id, Status::Success);
                }
                if slice.as_ref().is_some_and(|s| !s.contains(&n.id)) {
                    let mut skipped = state(&n.id, Status::Skipped);
                    skipped.skip_reason = Some("target".into());
                    return skipped;
                }
                state(&n.id, Status::Pending)
            })
            .collect();
        if slice.is_some() {
            run.partial = Some(true);
        }
        if let Some(ctx) = context {
            run.trigger_task_id = ctx.task_id().map(str::to_owned);
            if let (Some(restore), Some(source)) = (ctx.trigger_text("restore"), &ctx.source) {
                let id = source.get("id").and_then(Value::as_str).unwrap_or_default();
                let label = source
                    .get("displayName")
                    .filter(|v| !v.is_null())
                    .map_or_else(|| id.chars().take(8).collect(), vorn_work::js::to_string);
                run.trigger_session = Some(json!({ "id": id, "label": label, "restore": restore }));
            }
            if let Some(item) = &ctx.connector_item {
                run.connector_item = Some(item.clone());
                run.connector_inbox_id = item.get("inboxId").and_then(Value::as_f64);
                run.connector_inbox_lease_token = item
                    .get("inboxLeaseToken")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            run.inputs = ctx.inputs.clone().map(Value::Object);
        }
        run.dedupe_params = Some(dedupe);
        info!(workflow = %workflow.name, run = %run.run_id, "a workflow run started");
        let run = Arc::new(Mutex::new(run));
        self.persist(&run);
        drop(claims);
        Ok(run)
    }

    /// Starts walking `run` behind the caller.
    fn walk(
        &self,
        workflow: Workflow,
        run: Run,
        context: Option<Context>,
        source: Option<Source>,
    ) -> Started {
        let snapshot = lock(&run).clone();
        let engine = self.clone();
        let done = tokio::spawn(async move {
            engine.run_execution(&workflow, &run, context, source).await;
        });
        Started {
            run: snapshot,
            done: Some(done),
        }
    }

    /// `retryRunFromFailure`: a new run where steps that succeeded keep
    /// their outputs and the walk starts again at the failure.
    pub async fn retry(
        &self,
        workflow: &Workflow,
        failed: &WorkflowExecution,
    ) -> Result<Started, String> {
        let context = self.context_from_run(failed).await;
        let params = format!(
            "{}:retry:{}",
            failed.dedupe_params.as_deref().unwrap_or("manual"),
            failed.run_id
        );
        let claim = self.claim(&workflow.id, Some(&params), None);
        if !claim.granted {
            if let Some(existing) = self.snapshot(&claim.run_id).await {
                return Ok(Started {
                    run: existing,
                    done: None,
                });
            }
            return Err("A retry of this run is already in flight".into());
        }
        let mut run = blank_run(claim.run_id, workflow);
        run.node_states = seed_retry_states(workflow, failed);
        run.partial = failed.partial;
        run.retry_of_run_id = Some(failed.run_id.clone());
        run.trigger_task_id = failed.trigger_task_id.clone();
        run.connector_item = context.as_ref().and_then(|c| c.connector_item.clone());
        run.dedupe_params = Some(params);
        run.inputs = failed.inputs.clone();
        let run = Arc::new(Mutex::new(run));
        self.persist(&run);
        Ok(self.walk(workflow.clone(), run, context, None))
    }

    /// `rerunWorkflowRun`: a fresh run with an earlier one's context.
    pub async fn rerun(
        &self,
        workflow: &Workflow,
        run: &WorkflowExecution,
    ) -> Result<Started, String> {
        let context = self.context_from_run(run).await;
        self.execute(
            workflow,
            context,
            Options {
                source: Some(Source::Manual),
                target: None,
            },
        )
        .await
    }

    /// The triggering task, as the configuration has it now.
    async fn task_by_id(&self, id: Option<&str>) -> Option<Value> {
        let id = id?;
        let config = self.config().await?;
        tasks_of(&config)
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            .cloned()
    }

    /// `contextFromRun`: a run's launch context, so a retry resolves the
    /// same templates. The inbox lease stays the original run's.
    pub async fn context_from_run(&self, run: &WorkflowExecution) -> Option<Context> {
        let task = self.task_by_id(run.trigger_task_id.as_deref()).await;
        let item = run
            .connector_item
            .clone()
            .filter(|v| !v.is_null())
            .map(|mut item| {
                if let Some(map) = item.as_object_mut() {
                    map.remove("inboxId");
                    map.remove("inboxLeaseToken");
                }
                item
            });
        let inputs = run.inputs.as_ref().and_then(Value::as_object).cloned();
        if task.is_none() && item.is_none() && inputs.is_none() {
            return None;
        }
        let trigger = webhook_trigger_from_item(item.as_ref());
        Some(Context {
            task,
            connector_item: item,
            inputs,
            trigger,
            ..Context::default()
        })
    }

    /// `rebuildContextForResume`: what the steps after a gate read.
    pub(crate) async fn resume_context(&self, run: &Run) -> Option<Context> {
        let (task_id, item, inputs) = {
            let r = lock(run);
            (
                r.trigger_task_id.clone(),
                r.connector_item.clone().filter(|v| !v.is_null()),
                r.inputs.as_ref().and_then(Value::as_object).cloned(),
            )
        };
        let task = self.task_by_id(task_id.as_deref()).await;
        if task.is_none() && item.is_none() && inputs.is_none() {
            return None;
        }
        let trigger = webhook_trigger_from_item(item.as_ref());
        Some(Context {
            task,
            connector_item: item,
            inputs,
            trigger,
            ..Context::default()
        })
    }

    /// `runExecution`: walks the run until it ends or parks on a gate, then
    /// closes it out.
    pub(crate) async fn run_execution(
        &self,
        workflow: &Workflow,
        run: &Run,
        context: Option<Context>,
        source: Option<Source>,
    ) {
        let run_id = lock(run).run_id.clone();
        let dedupe = lock(run)
            .dedupe_params
            .clone()
            .unwrap_or_else(|| "manual".into());
        let active = {
            let mut active = locked(&self.inner.active);
            if active.contains_key(&run_id) {
                warn!(run = %run_id, "a run already being walked was asked to walk again");
                return;
            }
            let handle = Arc::new(Active {
                cancel: CancellationToken::new(),
                sessions: Mutex::default(),
                run: Arc::clone(run),
            });
            active.insert(run_id.clone(), Arc::clone(&handle));
            handle
        };
        self.start_heartbeat(run);

        let owners = loop_body_owners(&workflow.nodes);
        let main: Vec<&vorn_work::model::Node> = workflow
            .nodes
            .iter()
            .filter(|n| !owners.contains_key(&n.id))
            .collect();
        let action_count = workflow
            .nodes
            .iter()
            .filter(|n| n.kind != NodeKind::Trigger)
            .count();
        let mut parked = false;

        let walked = self
            .run_waves(
                workflow,
                run,
                context.as_ref(),
                &active,
                Waves {
                    nodes: main,
                    edges: collapse_loop_bodies(&workflow.nodes, &workflow.edges),
                    roots: Vec::new(),
                    max_waves: 50 * action_count.max(1),
                    stagger: workflow.stagger_ms.map(Duration::from_millis),
                },
                Mode::Main,
            )
            .await;

        let ended = match walked {
            Ok(_) if active.cancel.is_cancelled() => None,
            Ok(mut skipped) => {
                let waiting = lock(run).node_states.iter().any(|s| s.is(Status::Waiting));
                if waiting {
                    parked = true;
                    self.persist(run);
                    None
                } else {
                    let now = iso_now();
                    {
                        let mut r = lock(run);
                        let pending_owned: Vec<String> = r
                            .node_states
                            .iter()
                            .filter(|s| s.is(Status::Pending) && owners.contains_key(&s.node_id))
                            .map(|s| s.node_id.clone())
                            .collect();
                        for id in pending_owned {
                            let owner = &owners[&id];
                            let loop_reason = r
                                .node_state(owner)
                                .and_then(|s| s.skip_reason.clone())
                                .filter(|s| !s.is_empty());
                            let label = workflow
                                .node(owner)
                                .map_or("the loop", |n| n.label.as_str())
                                .to_owned();
                            r.update(&id, |s| {
                                s.set_status(Status::Skipped);
                                s.completed_at = Some(now.clone());
                                match loop_reason {
                                    Some(reason) => s.skip_reason = Some(reason),
                                    None => {
                                        s.error =
                                            Some(format!("Skipped: \"{label}\" did not run it"))
                                    }
                                }
                            });
                            skipped.insert(id);
                        }
                    }
                    let stranded = {
                        let mut r = lock(run);
                        let mut any = false;
                        for s in r.node_states.iter_mut().filter(|s| s.is(Status::Pending)) {
                            s.set_status(Status::Error);
                            s.completed_at = Some(now.clone());
                            s.error = Some("Skipped: predecessor nodes did not complete".into());
                            any = true;
                        }
                        any
                    };
                    if stranded {
                        self.persist(run);
                    }
                    let mut r = lock(run);
                    let failed = run_ended_in_error(&r, workflow, Some(&skipped));
                    r.status = if failed { "error" } else { "success" }.into();
                    r.completed_at = Some(iso_now());
                    Some(())
                }
            }
            Err(message) => {
                warn!(run = %run_id, %message, "a run failed");
                let now = iso_now();
                let mut r = lock(run);
                r.status = "error".into();
                r.completed_at = Some(now.clone());
                for s in r
                    .node_states
                    .iter_mut()
                    .filter(|s| s.is(Status::Running) || s.is(Status::Pending))
                {
                    s.set_status(Status::Error);
                    s.completed_at = Some(now.clone());
                    s.error = Some(message.clone());
                }
                Some(())
            }
        };

        locked(&self.inner.active).remove(&run_id);
        if !parked {
            self.release(&workflow.id, Some(&dedupe), &run_id);
        }
        if ended.is_none() {
            return;
        }

        let agents = self.inner.host.agent_sessions().await;
        {
            let mut r = lock(run);
            for s in r.node_states.iter_mut() {
                if let (Some(sid), None) = (&s.session_id, &s.agent_session_id) {
                    if let Some(agent) = agents.get(sid) {
                        s.agent_session_id = Some(agent.clone());
                    }
                }
            }
            if r.connector_inbox_id.is_some() {
                let settled = r.status == "success"
                    || locked(&self.inner.terminal_decisions).contains(&run_id);
                r.connector_inbox_disposition =
                    Some(if settled { "processed" } else { "retry" }.into());
            }
        }
        self.persist(run);
        self.stop_heartbeat(&run_id);
        self.stop_monitor(&run_id);

        if workflow.auto_cleanup_worktrees {
            self.clean_worktrees(workflow, run).await;
        }

        let (completed_at, status, inbox) = {
            let r = lock(run);
            (
                r.completed_at.clone().unwrap_or_default(),
                r.status.clone(),
                inbox_of(&r),
            )
        };
        let decided = locked(&self.inner.terminal_decisions).contains(&run_id);
        let report = self.inner.host.report_complete(Completion {
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name.clone(),
            completed_at,
            status: status.clone(),
            sessions_launched: action_count,
            source,
        });
        match inbox {
            Some((id, lease, disposition)) => {
                let error = (status != "success" && !decided)
                    .then(|| format!("Workflow finished with status {status}"));
                let complete = self.inner.host.complete_inbox(
                    id,
                    &lease,
                    disposition.as_deref().unwrap_or("retry"),
                    error,
                );
                tokio::join!(report, complete);
            }
            None => report.await,
        }
        locked(&self.inner.terminal_decisions).remove(&run_id);
    }

    /// Removes the worktrees the run's agent steps made, unless something
    /// still uses one or it holds changes. Inherited ones are never touched.
    async fn clean_worktrees(&self, workflow: &Workflow, run: &Run) {
        let mut worktrees: Vec<(String, String)> = Vec::new();
        {
            let r = lock(run);
            for s in &r.node_states {
                let Some(path) = s.worktree_path.as_ref().filter(|p| !p.is_empty()) else {
                    continue;
                };
                if worktrees.iter().any(|(p, _)| p == path)
                    || s.worktree_origin.as_deref() == Some("inherited")
                {
                    continue;
                }
                let Some(node) = workflow.node(&s.node_id) else {
                    continue;
                };
                if node.kind != NodeKind::LaunchAgent
                    || worktree_mode(&node.config) != WorktreeMode::New
                {
                    continue;
                }
                let project = s
                    .project_path
                    .clone()
                    .filter(|p| !p.is_empty())
                    .or_else(|| node.text("projectPath").map(str::to_owned))
                    .unwrap_or_default();
                worktrees.push((path.clone(), project));
            }
        }
        let host = &self.inner.host;
        let cleanups = worktrees.into_iter().map(|(path, project)| async move {
            if project.is_empty() {
                return;
            }
            let active = host
                .call("worktree:activeSessions", Value::String(path.clone()))
                .await;
            let count = active
                .as_ref()
                .ok()
                .and_then(|a| a.get("count"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if active.is_err() {
                return;
            }
            if count > 0.0 {
                info!(worktree = %path, "a worktree still in use was kept");
                return;
            }
            match host
                .call("git:worktreeDirty", Value::String(path.clone()))
                .await
            {
                Ok(Value::Bool(false)) => {}
                Ok(v) if !vorn_work::is_truthy(Some(&v)) => {}
                _ => {
                    info!(worktree = %path, "a worktree with changes was kept");
                    return;
                }
            }
            let _ = host
                .call(
                    "git:removeWorktree",
                    json!({ "projectPath": project, "worktreePath": path, "force": false }),
                )
                .await;
        });
        futures_util::future::join_all(cleanups).await;
    }

    /// `stopWorkflowRun`: ends the run's agents, then closes it as
    /// cancelled. Worktrees stay: a stopped run usually did partial work.
    pub async fn stop(&self, run_id: &str) {
        let handle = locked(&self.inner.active).get(run_id).cloned();
        let run = match &handle {
            Some(h) => Some(Arc::clone(&h.run)),
            None => self.run_by_id(run_id).await,
        };
        let Some(run) = run else {
            warn!(run = %run_id, "no run to stop");
            return;
        };
        if !lock(&run).is_running() {
            info!(run = %run_id, "the run already ended");
            return;
        }
        if let Some(h) = &handle {
            h.cancel.cancel();
        }
        let mut sessions: Vec<String> = handle
            .as_ref()
            .map(|h| locked(&h.sessions).iter().cloned().collect())
            .unwrap_or_default();
        for s in &lock(&run).node_states {
            if let Some(id) = &s.session_id {
                if (s.is(Status::Running) || s.is(Status::Waiting)) && !sessions.contains(id) {
                    sessions.push(id.clone());
                }
            }
        }
        let host = &self.inner.host;
        futures_util::future::join_all(
            sessions
                .iter()
                .map(|id| host.call("headless:kill", Value::String(id.clone()))),
        )
        .await;

        let now = iso_now();
        let node_ids: Vec<String> = {
            let mut r = lock(&run);
            for s in r.node_states.iter_mut() {
                match s.status() {
                    Some(Status::Running | Status::Waiting) => s.set_status(Status::Error),
                    Some(Status::Pending) => s.set_status(Status::Skipped),
                    _ => continue,
                }
                s.completed_at = Some(now.clone());
                s.error = Some("Stopped by user".into());
            }
            r.status = "cancelled".into();
            r.completed_at = Some(now.clone());
            r.connector_inbox_disposition = Some("processed".into());
            r.node_states.iter().map(|s| s.node_id.clone()).collect()
        };
        for id in node_ids {
            self.clear_gate_timer(&gate_key(run_id, &id));
        }
        self.stop_heartbeat(run_id);
        self.stop_monitor(run_id);
        self.persist(&run);

        let workflow = self.definition_of(&run).await;
        let (workflow_id, dedupe, inbox) = {
            let r = lock(&run);
            (r.workflow_id.clone(), r.dedupe_params.clone(), inbox_of(&r))
        };
        self.release(&workflow_id, dedupe.as_deref(), run_id);
        let report = host.report_complete(Completion {
            workflow_name: workflow.map_or_else(|| workflow_id.clone(), |w| w.name),
            workflow_id,
            completed_at: now,
            status: "cancelled".into(),
            sessions_launched: sessions.len(),
            source: None,
        });
        match inbox {
            Some((id, lease, _)) => {
                tokio::join!(
                    report,
                    host.complete_inbox(
                        id,
                        &lease,
                        "processed",
                        Some("Workflow stopped by user".into())
                    )
                );
            }
            None => report.await,
        }
        info!(run = %run_id, "a run was stopped");
    }

    fn clear_gate_timer(&self, key: &str) {
        if let Some(timer) = locked(&self.inner.gate_timers).remove(key) {
            timer.abort();
        }
    }

    /// Rejects the gate once `timeout_ms` has passed since it started
    /// asking.
    pub(crate) fn schedule_gate_timeout(
        &self,
        run: &Run,
        node_id: &str,
        timeout_ms: f64,
        elapsed_ms: f64,
    ) {
        if timeout_ms.is_nan() || timeout_ms <= 0.0 {
            return;
        }
        let run_id = lock(run).run_id.clone();
        let key = gate_key(&run_id, node_id);
        self.clear_gate_timer(&key);
        let remaining = (timeout_ms - elapsed_ms).max(0.0);
        let engine = self.clone();
        let run = Arc::clone(run);
        let node = node_id.to_owned();
        let k = key.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(remaining as u64)).await;
            locked(&engine.inner.gate_timers).remove(&k);
            let reason = format!(
                "Approval timed out after {}ms",
                vorn_work::js::to_string(&vorn_work::js::number_value(timeout_ms))
            );
            engine
                .reject(&run, &node, Rejection::TimedOut(reason))
                .await;
        });
        locked(&self.inner.gate_timers).insert(key, timer.abort_handle());
    }

    /// The node's state, if it waits for an answer the caller may give.
    fn resolve_waiting(&self, run: &Run, node_id: &str, caller: Decision) -> bool {
        let r = lock(run);
        let Some(s) = r.node_state(node_id) else {
            warn!(node = %node_id, "a gate answered for a step the run does not have");
            return false;
        };
        if !s.is(Status::Waiting) {
            warn!(node = %node_id, status = %s.status.0, "a gate answered that is not asking");
            return false;
        }
        if caller == Decision::Approve && is_sign_in_wait(s) {
            warn!(node = %node_id, "a step waiting for a sign-in cannot be approved");
            return false;
        }
        let key = gate_key(&r.run_id, node_id);
        drop(r);
        self.clear_gate_timer(&key);
        true
    }

    /// The gate's feedback with this answer's entry added, when it carried
    /// a comment, a rewrite or page comments.
    fn with_feedback(
        run: &WorkflowExecution,
        node_id: &str,
        decision: Decision,
        comment: Option<&str>,
        edited: Option<&str>,
        comments: &[GateComment],
    ) -> Option<Value> {
        let s = run.node_state(node_id);
        let existing = s.and_then(|s| s.feedback.clone());
        let text = comment.map(str::trim).unwrap_or("");
        let rewrite = edited.map(str::trim).filter(|r| !r.is_empty());
        if text.is_empty() && rewrite.is_none() && comments.is_empty() {
            return existing;
        }
        let mut entry = serde_json::Map::new();
        entry.insert(
            "round".into(),
            s.and_then(|s| s.round)
                .map_or(json!(1), vorn_work::js::number_value),
        );
        entry.insert("decision".into(), json!(decision.as_str()));
        entry.insert("comment".into(), json!(text));
        entry.insert("at".into(), json!(iso_now()));
        if let Some(r) = rewrite {
            entry.insert("edited".into(), json!(r));
        }
        if !comments.is_empty() {
            entry.insert(
                "comments".into(),
                Value::Array(comments.iter().map(GateComment::json).collect()),
            );
        }
        let mut list = existing
            .and_then(|f| f.as_array().cloned())
            .unwrap_or_default();
        list.push(Value::Object(entry));
        Some(Value::Array(list))
    }

    /// `approveWorkflowGate`.
    pub(crate) async fn approve(
        &self,
        run: &Run,
        node_id: &str,
        comment: Option<&str>,
        edited: Option<&str>,
    ) {
        if !self.resolve_waiting(run, node_id, Decision::Approve) {
            return;
        }
        let Some(workflow) = self.definition_of(run).await else {
            warn!("a gate was approved on a run whose workflow is gone");
            return;
        };
        let now = iso_now();
        {
            let mut r = lock(run);
            let feedback =
                Self::with_feedback(&r, node_id, Decision::Approve, comment, edited, &[]);
            let rewrite = edited
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(str::to_owned);
            r.update(node_id, |s| {
                s.set_status(Status::Success);
                s.completed_at = Some(now.clone());
                s.approved_at = Some(now.clone());
                if let Some(rw) = rewrite {
                    s.edited_text = Some(rw);
                }
                s.feedback = feedback;
            });
        }
        self.persist(run);
        let context = self.resume_context(run).await;
        self.run_execution(&workflow, run, context, None).await;
    }

    /// `rejectWorkflowGate`: a person's answer, or the gate's own timeout.
    pub(crate) async fn reject(&self, run: &Run, node_id: &str, rejection: Rejection) {
        if !self.resolve_waiting(run, node_id, Decision::Reject) {
            return;
        }
        let Some(workflow) = self.definition_of(run).await else {
            warn!("a gate was rejected on a run whose workflow is gone");
            return;
        };
        let run_id = lock(run).run_id.clone();
        let (timed_out, note) = match &rejection {
            Rejection::TimedOut(reason) => (Some(reason.clone()), None),
            Rejection::Person(note) => (None, note.as_deref().map(str::trim).map(str::to_owned)),
        };
        if timed_out.is_none() {
            locked(&self.inner.terminal_decisions).insert(run_id.clone());
            lock(run).connector_inbox_disposition = Some("processed".into());
        }
        let now = iso_now();
        {
            let mut r = lock(run);
            let feedback =
                Self::with_feedback(&r, node_id, Decision::Reject, note.as_deref(), None, &[]);
            let error = timed_out
                .clone()
                .or_else(|| note.clone().filter(|n| !n.is_empty()))
                .unwrap_or_else(|| "Rejected by user".into());
            r.update(node_id, |s| {
                s.set_status(Status::Error);
                s.completed_at = Some(now.clone());
                s.error = Some(error);
                if timed_out.is_none() {
                    s.rejected_at = Some(now.clone());
                }
                s.feedback = feedback;
            });
            let graph = Graph::of(&workflow.edges);
            let terminal = |r: &WorkflowExecution, id: &str| {
                r.node_state(id)
                    .and_then(StateExt::status)
                    .is_some_and(Status::is_terminal)
            };
            for succ in graph.successors_of(node_id).to_vec() {
                let branch = collect_skipped_branch(&succ, &graph, |id| terminal(&r, id));
                for id in branch {
                    r.update(&id, |s| {
                        if s.is(Status::Pending) {
                            s.set_status(Status::Skipped);
                            s.completed_at = Some(now.clone());
                        }
                    });
                }
            }
        }
        self.persist(run);
        let context = self.resume_context(run).await;
        self.run_execution(&workflow, run, context, None).await;
    }

    /// `changesPlan`: the steps a request for changes sends back, or why
    /// the gate cannot take it.
    async fn changes_plan(
        &self,
        run: &Run,
        node_id: &str,
        comment: &str,
        comments: &[GateComment],
    ) -> Result<(String, HashSet<String>), String> {
        let workflow = self.definition_of(run).await;
        let node = workflow.as_ref().and_then(|w| w.node(node_id));
        let (Some(workflow), Some(node)) = (
            workflow.as_ref(),
            node.filter(|n| n.kind == NodeKind::Approval),
        ) else {
            return Err(format!("{node_id} is not an approval gate"));
        };
        let round = {
            let r = lock(run);
            match r.node_state(node_id) {
                Some(s) if s.is(Status::Waiting) && !is_sign_in_wait(s) => s.round,
                _ => return Err(format!("{node_id} is not asking")),
            }
        };
        if comment.trim().is_empty() && comments.is_empty() {
            return Err("a request for changes needs a comment".into());
        }
        if !graph::can_request_changes(&node.config, round) {
            return Err(format!("{node_id} takes no more changes"));
        }
        let from = node
            .config
            .get("feedback")
            .and_then(|f| f.get("from"))
            .map(vorn_work::js::to_string)
            .unwrap_or_default();
        let reset = nodes_between(&from, node_id, &workflow.edges);
        if reset.is_empty() {
            return Err(format!("{from} does not lead to {node_id}"));
        }
        Ok((from, reset))
    }

    /// `requestGateChanges`: every step from the gate's `from` to the gate
    /// runs again, and the gate asks again.
    pub(crate) async fn request_changes(
        &self,
        run: &Run,
        node_id: &str,
        comment: &str,
        edited: Option<&str>,
        comments: &[GateComment],
    ) {
        let (from, reset) = match self.changes_plan(run, node_id, comment, comments).await {
            Ok(plan) => plan,
            Err(refused) => {
                warn!(%refused, "a request for changes was refused");
                return;
            }
        };
        if !self.resolve_waiting(run, node_id, Decision::Changes) {
            return;
        }
        let Some(workflow) = self.definition_of(run).await else {
            return;
        };
        {
            let mut r = lock(run);
            let round = r.node_state(node_id).and_then(|s| s.round).unwrap_or(1.0) + 1.0;
            let feedback = Self::with_feedback(
                &r,
                node_id,
                Decision::Changes,
                Some(comment),
                edited,
                comments,
            );
            let rewrite = edited
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(str::to_owned);
            for s in r.node_states.iter_mut() {
                if reset.contains(&s.node_id) {
                    *s = graph::blank_pass_state(&s.node_id, None);
                }
            }
            r.update(node_id, |s| {
                s.round = Some(round);
                s.feedback = feedback;
                if let Some(rw) = rewrite {
                    s.edited_text = Some(rw);
                }
            });
            info!(run = %r.run_id, gate = %node_id, %from, round, "a gate sent the work back");
        }
        self.persist(run);
        let context = self.resume_context(run).await;
        self.run_execution(&workflow, run, context, None).await;
    }

    /// `applyGateDecision`: an answer from any client, acted on where the
    /// run is. A gate already answered sends the run out as it stands.
    pub async fn apply_gate_decision(&self, run_id: &str, node_id: &str, answer: Answer) {
        let Some(run) = self.run_by_id(run_id).await else {
            return;
        };
        let waiting = lock(&run)
            .node_state(node_id)
            .is_some_and(|s| s.is(Status::Waiting));
        if !waiting {
            self.publish(&run);
            return;
        }
        let comments = answer
            .comments
            .as_ref()
            .map(GateComment::clean)
            .unwrap_or_default();
        match answer.decision {
            Decision::Approve => {
                self.approve(
                    &run,
                    node_id,
                    answer.comment.as_deref(),
                    answer.edited.as_deref(),
                )
                .await
            }
            Decision::Reject => {
                self.reject(&run, node_id, Rejection::Person(answer.comment.clone()))
                    .await
            }
            Decision::Changes => {
                self.request_changes(
                    &run,
                    node_id,
                    answer.comment.as_deref().unwrap_or(""),
                    answer.edited.as_deref(),
                    &comments,
                )
                .await
            }
        }
    }

    /// What `workflow:resolveGate` answers before acting: a sign-in wait
    /// is never approved past, a request for changes the gate cannot take
    /// is refused, and so is a rewrite that broke the gate's list, with why.
    pub async fn check_gate(
        &self,
        run_id: &str,
        node_id: &str,
        answer: &Answer,
    ) -> Result<(), Option<String>> {
        if answer.decision == Decision::Approve && self.waits_for_sign_in(run_id, node_id).await {
            return Err(None);
        }
        if answer.decision == Decision::Changes {
            let comments = answer.comments.clone().unwrap_or(Value::Null);
            let comment = answer.comment.as_deref().unwrap_or("");
            if !self
                .takes_changes(run_id, node_id, comment, &comments)
                .await
            {
                return Err(None);
            }
        }
        if answer.decision != Decision::Reject {
            if let Some(reason) = self
                .edit_refusal(run_id, node_id, answer.edited.as_deref())
                .await
            {
                return Err(Some(reason));
            }
        }
        Ok(())
    }

    /// Whether a waiting step waits for its connection to sign in.
    pub async fn waits_for_sign_in(&self, run_id: &str, node_id: &str) -> bool {
        match self.run_by_id(run_id).await {
            Some(run) => lock(&run).node_state(node_id).is_some_and(is_sign_in_wait),
            None => false,
        }
    }

    /// Whether a request for changes with these comments would be taken.
    pub async fn takes_changes(
        &self,
        run_id: &str,
        node_id: &str,
        comment: &str,
        comments: &Value,
    ) -> bool {
        let Some(run) = self.run_by_id(run_id).await else {
            return false;
        };
        self.changes_plan(&run, node_id, comment, &GateComment::clean(comments))
            .await
            .is_ok()
    }

    /// Why the gate would refuse this rewrite.
    pub async fn edit_refusal(
        &self,
        run_id: &str,
        node_id: &str,
        edited: Option<&str>,
    ) -> Option<String> {
        let run = self.run_by_id(run_id).await;
        let editable = run.and_then(|r| {
            lock(&r)
                .node_state(node_id)
                .and_then(|s| s.editable_text.clone())
        });
        gate_edit_refusal(editable.as_deref(), edited)
    }

    /// The round of a gate's review page when `token` is this round's and
    /// the gate still asks.
    pub async fn gate_page_round(&self, run_id: &str, node_id: &str, token: &str) -> Option<u32> {
        let run = self.run_by_id(run_id).await?;
        let r = lock(&run);
        let s = r.node_state(node_id)?;
        let expected = s.view_token.as_deref()?;
        (s.is(Status::Waiting) && same_token(expected, token))
            .then(|| s.round.unwrap_or(1.0) as u32)
    }

    /// `resumeSignInWaits`: the steps that waited for `connection_id` run
    /// again; what ran before them is kept.
    pub async fn resume_sign_in_waits(&self, connection_id: &str, parked: Vec<WorkflowExecution>) {
        let mut resumes = Vec::new();
        for stored in parked {
            let run = locked(&self.inner.active)
                .get(&stored.run_id)
                .map(|a| Arc::clone(&a.run))
                .unwrap_or_else(|| Arc::new(Mutex::new(stored)));
            let Some(workflow) = self.definition_of(&run).await else {
                continue;
            };
            let waiting: Vec<String> = lock(&run)
                .node_states
                .iter()
                .filter(|s| is_sign_in_wait(s))
                .filter(|s| {
                    workflow
                        .node(&s.node_id)
                        .and_then(|n| n.text("connectionId"))
                        == Some(connection_id)
                })
                .map(|s| s.node_id.clone())
                .collect();
            if waiting.is_empty() {
                continue;
            }
            {
                let mut r = lock(&run);
                for id in &waiting {
                    r.update(id, |s| {
                        s.set_status(Status::Pending);
                        s.waiting_for = None;
                        s.error = None;
                        s.logs = None;
                        s.started_at = None;
                    });
                }
            }
            self.persist(&run);
            info!(steps = waiting.len(), "steps run again after a sign-in");
            let engine = self.clone();
            resumes.push(async move {
                let context = engine.resume_context(&run).await;
                engine.run_execution(&workflow, &run, context, None).await;
            });
        }
        futures_util::future::join_all(resumes).await;
    }

    /// `adoptConnectorInboxLease`: a run already going takes the lease of
    /// its item's newer delivery.
    pub async fn adopt_lease(&self, run_id: &str, item: &Value) {
        let Some(run) = self.run_by_id(run_id).await else {
            return;
        };
        let inbox = item.get("inboxId").and_then(Value::as_f64);
        let Some(lease) = item
            .get("inboxLeaseToken")
            .and_then(Value::as_str)
            .filter(|l| !l.is_empty())
        else {
            return;
        };
        if inbox.is_none() || lock(&run).connector_inbox_id != inbox {
            return;
        }
        self.stop_heartbeat(run_id);
        {
            let mut r = lock(&run);
            r.connector_item = Some(item.clone());
            r.connector_inbox_lease_token = Some(lease.to_owned());
        }
        self.publish(&run);
        self.save_now(&run).await;
        self.start_heartbeat(&run);
    }

    /// Renews the run's inbox lease every [`LEASE_RENEW_INTERVAL`] until the
    /// run ends or the lease is gone.
    pub(crate) fn start_heartbeat(&self, run: &Run) {
        let (run_id, inbox) = {
            let r = lock(run);
            (r.run_id.clone(), inbox_of(&r))
        };
        let Some((id, lease, _)) = inbox else { return };
        let mut beats = locked(&self.inner.heartbeats);
        if beats.contains_key(&run_id) {
            return;
        }
        let engine = self.clone();
        let rid = run_id.clone();
        let beat = tokio::spawn(async move {
            let mut every = tokio::time::interval(LEASE_RENEW_INTERVAL);
            every.tick().await;
            loop {
                every.tick().await;
                if !engine.inner.host.renew_inbox(id, &lease).await {
                    locked(&engine.inner.heartbeats).remove(&rid);
                    return;
                }
            }
        });
        beats.insert(run_id, beat.abort_handle());
    }

    pub(crate) fn stop_heartbeat(&self, run_id: &str) {
        if let Some(beat) = locked(&self.inner.heartbeats).remove(run_id) {
            beat.abort();
        }
    }

    fn stop_monitor(&self, run_id: &str) {
        if let Some(m) = locked(&self.inner.monitors).remove(run_id) {
            m.abort();
        }
    }

    /// Looks again every [`RESTORED_POLL_INTERVAL`] at a restored connector
    /// run whose agent still ran, so its item is settled when it ends.
    fn monitor_restored(&self, run: &Run, workflows: Vec<Workflow>) {
        let (run_id, has_inbox) = {
            let r = lock(run);
            (r.run_id.clone(), inbox_of(&r).is_some())
        };
        let mut monitors = locked(&self.inner.monitors);
        if !has_inbox || monitors.contains_key(&run_id) {
            return;
        }
        let engine = self.clone();
        let run = Arc::clone(run);
        let monitor = tokio::spawn(async move {
            let mut every = tokio::time::interval(RESTORED_POLL_INTERVAL);
            every.tick().await;
            loop {
                every.tick().await;
                Box::pin(engine.reconcile_one(&run, &workflows)).await;
            }
        });
        monitors.insert(run_id, monitor.abort_handle());
    }

    async fn acknowledge_reconciled(&self, run: &Run) {
        let (inbox, status, disposition) = {
            let r = lock(run);
            (
                inbox_of(&r),
                r.status.clone(),
                r.connector_inbox_disposition.clone(),
            )
        };
        let Some((id, lease, _)) = inbox else { return };
        let disposition = disposition.unwrap_or_else(|| {
            if status == "success" {
                "processed"
            } else {
                "retry"
            }
            .into()
        });
        let error = (status != "success" && disposition != "processed")
            .then(|| format!("Workflow recovered with status {status}"));
        self.inner
            .host
            .complete_inbox(id, &lease, &disposition, error)
            .await;
    }

    /// `reconcileRunningExecutions`: runs left `running` by an earlier
    /// process are closed from their sessions' exits, never resumed.
    pub async fn reconcile(&self, runs: Vec<WorkflowExecution>, workflows: &[Workflow]) {
        for stored in runs {
            let run = Arc::new(Mutex::new(stored));
            self.reconcile_one(&run, workflows).await;
        }
    }

    async fn reconcile_one(&self, run: &Run, workflows: &[Workflow]) {
        let run_id = lock(run).run_id.clone();
        let (completed, running) = {
            let r = lock(run);
            (r.completed_at.is_some(), r.is_running())
        };
        if completed && !running {
            self.stop_heartbeat(&run_id);
            self.stop_monitor(&run_id);
            self.acknowledge_reconciled(run).await;
            return;
        }
        self.start_heartbeat(run);
        let workflow = match self.definition_of(run).await {
            Some(w) => Some(w),
            None => {
                let id = lock(run).workflow_id.clone();
                workflows.iter().find(|w| w.id == id).cloned()
            }
        };

        let running_nodes: Vec<(String, Option<String>)> = lock(run)
            .node_states
            .iter()
            .filter(|s| s.is(Status::Running))
            .map(|s| (s.node_id.clone(), s.session_id.clone()))
            .collect();
        let host = &self.inner.host;
        let probes =
            futures_util::future::join_all(running_nodes.iter().map(|(_, session)| async move {
                let Some(session) = session.as_ref().filter(|s| !s.is_empty()) else {
                    return Probe::NoSession;
                };
                match host
                    .call(
                        "sessionEvent:listBySession",
                        json!({ "sessionId": session, "limit": 50 }),
                    )
                    .await
                {
                    Ok(Value::Array(events)) => match events
                        .into_iter()
                        .find(|e| e.get("eventType").and_then(Value::as_str) == Some("exited"))
                    {
                        Some(exit) => Probe::Exited(exit),
                        None => Probe::Running,
                    },
                    Ok(_) => Probe::Running,
                    Err(_) => Probe::Failed,
                }
            }))
            .await;

        let mut dirty = false;
        let mut still_running = false;
        let mut resolved = false;
        {
            let mut r = lock(run);
            for ((node_id, _), probe) in running_nodes.iter().zip(probes) {
                match probe {
                    Probe::NoSession => {
                        r.update(node_id, |s| {
                            s.set_status(Status::Error);
                            s.error = Some(ABANDONED.into());
                            s.completed_at = Some(iso_now());
                        });
                        dirty = true;
                        resolved = true;
                    }
                    Probe::Exited(exit) => {
                        let code = exit
                            .get("metadata")
                            .and_then(|m| m.get("exitCode"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                        let at = exit
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        r.update(node_id, |s| {
                            s.set_status(if code == 0.0 {
                                Status::Success
                            } else {
                                Status::Error
                            });
                            s.completed_at = at;
                            if code != 0.0 && s.error.as_deref().is_none_or(str::is_empty) {
                                s.error = Some(format!(
                                    "Exit code {}",
                                    vorn_work::js::to_string(&vorn_work::js::number_value(code))
                                ));
                            }
                        });
                        dirty = true;
                        resolved = true;
                    }
                    Probe::Running | Probe::Failed => still_running = true,
                }
            }
            if resolved && !still_running {
                let pending = r.node_states.iter().any(|s| s.is(Status::Pending));
                if pending {
                    for s in r.node_states.iter_mut().filter(|s| s.is(Status::Pending)) {
                        s.set_status(Status::Skipped);
                        s.error = Some(RELOAD_ABANDONED.into());
                    }
                    r.status = "error".into();
                } else {
                    let failed = match &workflow {
                        Some(w) => run_ended_in_error(&r, w, None),
                        None => run_ended_in_error(&r, &empty_workflow(), None),
                    };
                    r.status = if failed { "error" } else { "success" }.into();
                }
                r.completed_at = Some(iso_now());
                dirty = true;
            }
            let waiting = r.node_states.iter().any(|s| s.is(Status::Waiting));
            if running_nodes.is_empty() && !waiting && r.is_running() {
                let now = iso_now();
                for s in r.node_states.iter_mut().filter(|s| s.is(Status::Pending)) {
                    s.set_status(Status::Error);
                    s.completed_at = Some(now.clone());
                    s.error = Some(RELOAD_ABANDONED.into());
                }
                r.status = "error".into();
                r.completed_at = Some(now);
                dirty = true;
            }
        }
        if dirty {
            self.publish(run);
            self.save_now(run).await;
        }
        if !lock(run).is_running() {
            self.stop_heartbeat(&run_id);
            self.stop_monitor(&run_id);
            self.acknowledge_reconciled(run).await;
        } else if still_running {
            self.monitor_restored(run, workflows.to_vec());
        }
    }

    /// `rescheduleWaitingGateTimers`: gates left waiting by an earlier
    /// process time out as they would have.
    pub async fn rearm_gates(&self, runs: Vec<WorkflowExecution>, workflows: &[Workflow]) {
        let now = vorn_work_now() as f64;
        for stored in runs {
            let run = Arc::new(Mutex::new(stored));
            if lock(&run).is_running() {
                self.start_heartbeat(&run);
            }
            let workflow = match self.definition_of(&run).await {
                Some(w) => w,
                None => {
                    let id = lock(&run).workflow_id.clone();
                    match workflows.iter().find(|w| w.id == id) {
                        Some(w) => w.clone(),
                        None => continue,
                    }
                }
            };
            let waiting: Vec<(String, Option<String>)> = lock(&run)
                .node_states
                .iter()
                .filter(|s| s.is(Status::Waiting))
                .map(|s| (s.node_id.clone(), s.started_at.clone()))
                .collect();
            for (node_id, started) in waiting {
                let Some(node) = workflow
                    .node(&node_id)
                    .filter(|n| n.kind == NodeKind::Approval)
                else {
                    continue;
                };
                let timeout = node
                    .config
                    .get("timeoutMs")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                if timeout.is_nan() || timeout <= 0.0 {
                    continue;
                }
                let started_ms = started.as_deref().and_then(parse_ms).unwrap_or(now);
                self.schedule_gate_timeout(&run, &node_id, timeout, now - started_ms);
            }
        }
    }
}

/// Why a gate was rejected.
pub(crate) enum Rejection {
    Person(Option<String>),
    TimedOut(String),
}

enum Probe {
    NoSession,
    Exited(Value),
    Running,
    Failed,
}

/// The run's inbox row: id, lease and disposition so far.
fn inbox_of(r: &WorkflowExecution) -> Option<(i64, String, Option<String>)> {
    let id = r.connector_inbox_id?;
    let lease = r
        .connector_inbox_lease_token
        .clone()
        .filter(|l| !l.is_empty())?;
    Some((id as i64, lease, r.connector_inbox_disposition.clone()))
}

fn empty_workflow() -> Workflow {
    Workflow::from_json(&json!({ "id": "" })).expect("an id is all a workflow needs")
}

/// A run with no steps yet.
fn blank_run(run_id: String, workflow: &Workflow) -> WorkflowExecution {
    WorkflowExecution {
        run_id,
        workflow_id: workflow.id.clone(),
        started_at: iso_now(),
        status: "running".into(),
        node_states: Vec::new(),
        completed_at: None,
        connector_inbox_disposition: None,
        connector_inbox_id: None,
        connector_inbox_lease_token: None,
        connector_item: None,
        dedupe_params: None,
        definition: Some(workflow.raw.clone()),
        inputs: None,
        partial: None,
        retry_of_run_id: None,
        trigger_session: None,
        trigger_task_id: None,
    }
}

/// Unix milliseconds now.
pub(crate) fn vorn_work_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A new run or script id.
pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Compares tokens without stopping at the first difference.
pub fn same_token(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The configuration's workflows.
pub fn workflows_of(config: &Value) -> Vec<Workflow> {
    config
        .get("workflows")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Workflow::from_json).collect())
        .unwrap_or_default()
}

/// The configuration's tasks.
pub(crate) fn tasks_of(config: &Value) -> &[Value] {
    config
        .get("tasks")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `new Date(text).getTime()` for the times the engine writes.
fn parse_ms(text: &str) -> Option<f64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(|t| t.as_millisecond() as f64)
}
