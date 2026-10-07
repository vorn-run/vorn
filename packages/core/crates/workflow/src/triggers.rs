//! Workflows started by something happening: a task appearing or moving, a
//! session coming back (`triggers.ts`), a schedule firing or a connector
//! item arriving (`dispatch.ts`).

use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tracing::warn;
use vorn_work::graph::scheduler_context;
use vorn_work::model::{Context, Workflow};

use crate::engine::{workflows_of, Engine, Options};
use crate::host::{Host, Source};

/// A restore-triggered run waiting for its project's turn.
pub(crate) type Queued = (Workflow, Context);

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl<H: Host> Engine<H> {
    /// The enabled workflows, and each one's trigger config.
    async fn armed(&self, kind: &str) -> Vec<(Workflow, Value)> {
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
    fn start(&self, workflow: Workflow, context: Option<Context>, source: Option<Source>) {
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

    /// `fireTaskCreatedTrigger`.
    pub async fn fire_task_created(&self, task: &Value) {
        let project = text(task, "projectName");
        for (workflow, trigger) in self.armed("taskCreated").await {
            if text(&trigger, "projectFilter").is_some_and(|f| Some(f) != project) {
                continue;
            }
            let mut t = Map::new();
            t.insert("type".into(), "taskCreated".into());
            let context = Context {
                task: Some(task.clone()),
                trigger: Some(Value::Object(t)),
                ..Context::default()
            };
            self.start(workflow, Some(context), None);
        }
    }

    /// `fireTaskStatusChangedTrigger`.
    pub async fn fire_task_status_changed(&self, task: &Value, from: &str, to: &str) {
        if from == to {
            return;
        }
        let project = text(task, "projectName");
        for (workflow, trigger) in self.armed("taskStatusChanged").await {
            if text(&trigger, "projectFilter").is_some_and(|f| Some(f) != project)
                || text(&trigger, "fromStatus").is_some_and(|f| f != from)
                || text(&trigger, "toStatus").is_some_and(|f| f != to)
            {
                continue;
            }
            let context = Context {
                task: Some(task.clone()),
                trigger: Some(
                    serde_json::json!({ "type": "taskStatusChanged", "fromStatus": from, "toStatus": to }),
                ),
                ..Context::default()
            };
            self.start(workflow, Some(context), None);
        }
    }

    /// `fireSessionRestoredTrigger`: cold restores by default, one run at a
    /// time per project unless the workflow says otherwise.
    pub async fn fire_session_restored(
        &self,
        session: &Value,
        restore: &str,
        environment: Option<Value>,
    ) {
        let project = text(session, "projectName").unwrap_or_default().to_owned();
        for (workflow, trigger) in self.armed("sessionRestored").await {
            if text(&trigger, "projectFilter").is_some_and(|f| f != project) {
                continue;
            }
            if text(&trigger, "restore").unwrap_or("cold") == "cold" && restore == "warm" {
                continue;
            }
            let mut t = Map::new();
            t.insert("type".into(), "sessionRestored".into());
            t.insert("restore".into(), restore.into());
            if let Some(env) = environment.clone().filter(|e| !e.is_null()) {
                t.insert("environment".into(), env);
            }
            let context = Context {
                source: Some(session.clone()),
                trigger: Some(Value::Object(t)),
                ..Context::default()
            };
            if text(&trigger, "concurrency") == Some("unbounded") {
                self.start(workflow, Some(context), None);
            } else {
                self.queue_restore(&project, workflow, context);
            }
        }
    }

    fn queue_restore(&self, project: &str, workflow: Workflow, context: Context) {
        let mut queues = self
            .inner
            .restore_queues
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let sender = queues.entry(project.to_owned()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Queued>();
            let engine = self.clone();
            tokio::spawn(async move {
                while let Some((workflow, context)) = rx.recv().await {
                    match engine.execute(&workflow, Some(context), Options::default()).await {
                        Ok(started) => {
                            started.finish().await;
                        }
                        Err(err) => warn!(workflow = %workflow.name, %err, "a restore workflow did not start"),
                    }
                }
            });
            tx
        });
        let _ = sender.send((workflow, context));
    }

    /// `runScheduled`: a schedule fired, or `workflow:runManual` asked.
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
        let context = scheduler_context(None, inputs);
        self.start(workflow, context, Some(Source::Scheduler));
    }

    /// `runConnectorItem`: an inbox row delivered. A row whose workflow is
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
