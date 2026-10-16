//! Workflows started by something happening: a trigger an extension fires
//! (a task appearing or moving, a session coming back), a schedule falling
//! due or a webhook or connector item arriving at the core.

use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tracing::warn;
use vorn_work::graph::scheduler_context;
use vorn_work::model::{Context, Workflow};

use crate::engine::{workflows_of, Engine, Options};
use crate::host::{Host, Source};
use crate::registry::matches;

/// A run waiting for its turn.
pub(crate) type Queued = (Workflow, Context);

impl<H: Host> Engine<H> {
    /// The enabled workflows whose trigger is of type `kind`, each with its
    /// trigger's config.
    pub async fn armed(&self, kind: &str) -> Vec<(Workflow, Value)> {
        let Some(config) = self.config().await else {
            return Vec::new();
        };
        workflows_of(&config)
            .into_iter()
            .filter(|w| w.enabled && w.trigger_type() == Some(kind))
            .filter_map(|w| {
                let config = w.trigger()?.config.clone();
                Some((w, config))
            })
            .collect()
    }

    /// Starts `workflow` behind the caller.
    pub fn start(&self, workflow: Workflow, context: Option<Context>, source: Option<Source>) {
        let engine = self.clone();
        tokio::spawn(async move {
            let options = Options {
                source,
                target: None,
            };
            if let Err(err) = engine.execute(&workflow, context, options).await {
                warn!(workflow = %workflow.name, %err, "a triggered workflow did not start");
            }
        });
    }

    /// Fires trigger type `kind`: every enabled workflow it arms whose
    /// trigger config takes these `fields` ([`matches`]) starts with
    /// `context`. Answers how many started.
    pub async fn fire(&self, kind: &str, fields: &Map<String, Value>, context: Context) -> usize {
        let mut started = 0;
        for (workflow, trigger) in self.armed(kind).await {
            if !matches(&trigger, fields) {
                continue;
            }
            self.start(workflow, Some(context.clone()), None);
            started += 1;
        }
        started
    }

    /// Starts `workflow` once the runs started before it under `key` have
    /// ended: one at a time per key, in order.
    pub fn start_in_turn(&self, key: &str, workflow: Workflow, context: Context) {
        let mut turns = self
            .inner
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sender = turns.entry(key.to_owned()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Queued>();
            let engine = self.clone();
            tokio::spawn(async move {
                while let Some((workflow, context)) = rx.recv().await {
                    match engine
                        .execute(&workflow, Some(context), Options::default())
                        .await
                    {
                        Ok(started) => {
                            started.finish().await;
                        }
                        Err(err) => {
                            warn!(workflow = %workflow.name, %err, "a workflow started in turn did not start")
                        }
                    }
                }
            });
            tx
        });
        let _ = sender.send((workflow, context));
    }

    /// A schedule fired, or `workflow:runManual` asked.
    pub async fn run_scheduled(&self, workflow_id: &str, inputs: Option<Map<String, Value>>) {
        let Some(config) = self.config().await else {
            return;
        };
        let Some(workflow) = workflows_of(&config)
            .into_iter()
            .find(|w| w.id == workflow_id)
        else {
            return;
        };
        self.run_scheduled_workflow(workflow, inputs);
    }

    /// [`Self::run_scheduled`] for a workflow the caller has already read.
    pub fn run_scheduled_workflow(&self, workflow: Workflow, inputs: Option<Map<String, Value>>) {
        let context = scheduler_context(None, inputs);
        self.start(workflow, context, Some(Source::Scheduler));
    }

    /// An inbox row delivered. A row whose workflow is
    /// gone is deferred rather than dropped; a run already going takes the
    /// row's lease instead of a second run starting.
    pub async fn run_connector_item(
        &self,
        workflow_id: &str,
        item: Value,
        existing: Option<vorn_protocol::WorkflowExecution>,
    ) {
        let inbox = item
            .get("inboxId")
            .and_then(Value::as_f64)
            .map(|id| id as i64);
        let lease = item
            .get("inboxLeaseToken")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let config = self.config().await;
        let workflow = config
            .as_ref()
            .and_then(|c| workflows_of(c).into_iter().find(|w| w.id == workflow_id));
        let Some(workflow) = workflow else {
            if let (Some(id), Some(lease)) = (inbox, lease.filter(|l| !l.is_empty())) {
                self.inner
                    .host
                    .complete_inbox(id, &lease, "defer", None)
                    .await;
            }
            return;
        };
        if let Some(existing) = existing {
            let run_id = existing.run_id.clone();
            self.adopt_lease_on(existing.clone(), &item).await;
            self.rearm_gates(vec![existing.clone()], std::slice::from_ref(&workflow))
                .await;
            let current = self.snapshot(&run_id).await.unwrap_or(existing);
            self.reconcile(vec![current], std::slice::from_ref(&workflow))
                .await;
            return;
        }
        let context = scheduler_context(Some(item.clone()), None);
        let options = Options {
            source: Some(Source::Scheduler),
            target: None,
        };
        match self.execute(&workflow, context, options).await {
            Ok(started) => {
                let other = lease.is_some() && started.run.connector_inbox_lease_token != lease;
                let run_id = started.run.run_id.clone();
                if other {
                    self.adopt_lease(&run_id, &item).await;
                }
                started.finish().await;
            }
            Err(err) => warn!(%workflow_id, %err, "a delivered item did not complete"),
        }
    }

    /// Adopts a lease on a stored run this process may not hold.
    async fn adopt_lease_on(&self, stored: vorn_protocol::WorkflowExecution, item: &Value) {
        if !self.is_active(&stored.run_id) {
            self.inner.host.save_run(stored.clone()).await;
        }
        self.adopt_lease(&stored.run_id, item).await;
    }
}
