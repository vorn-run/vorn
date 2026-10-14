//! The spike's workflow runs: started, parked and resumed by vornd, walked by the step loop in a component or natively.

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{info, warn};
use vorn_ext_perf_guest::workflows::{self as walk, Outcome};
use vorn_ext_perf_host::Bridge;
use vorn_protocol::WorkflowExecution;
use vorn_workflow::{Completion, Host as _, Source};

use super::Work;
use crate::native::ext_spike::{self, Walker};
use crate::native::Answer;

/// Where the step loop's run changes land: told to viewers and written, in order, as the engine's `persist`.
struct RunBridge(Arc<Work>);

impl RunBridge {
    fn persist(&self, run: WorkflowExecution) {
        if ext_spike::get().is_some_and(|s| s.should_publish(&run)) {
            self.0.host().publish(&run);
        }
        if let Some(saves) = SAVES.get() {
            let _ = saves.send(run);
        }
    }
}

/// Runs waiting to be written, one at a time and in order as the engine's writer does, so other reads are not stuck behind them.
static SAVES: OnceLock<mpsc::UnboundedSender<WorkflowExecution>> = OnceLock::new();

/// Starts the writer on the runtime, once.
fn start_writer(work: &Work) {
    SAVES.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<WorkflowExecution>();
        let (db, data_dir) = (work.db.clone(), work.host().data_dir.clone());
        tokio::spawn(async move {
            while let Some(run) = rx.recv().await {
                let data_dir = data_dir.clone();
                db.run(move |store| match store.save_workflow_run(&run) {
                    Ok(trimmed) => {
                        for id in trimmed {
                            vorn_work::gates::remove(&data_dir, &id);
                        }
                    }
                    Err(err) => warn!(%err, run = %run.run_id, "a run could not be saved"),
                })
                .await;
            }
        });
        tx
    });
}

impl Bridge for RunBridge {
    fn db(&mut self, op: &str, _args: &str) -> Result<String, String> {
        Err(format!("{op}: the step loop reads nothing"))
    }

    fn run_changed(&mut self, run: &str) {
        match serde_json::from_str::<WorkflowExecution>(run) {
            Ok(run) => self.persist(run),
            Err(err) => warn!(%err, "the step loop sent a run that could not be read"),
        }
    }

    fn log(&mut self, line: &str) {
        info!("{line}");
    }
}

impl walk::Host for RunBridge {
    fn persist(&mut self, run: &WorkflowExecution) {
        RunBridge::persist(self, run.clone());
    }

    fn log(&mut self, line: &str) {
        info!("{line}");
    }
}

/// One walk to make.
enum Job {
    Start {
        workflow: Value,
        run_id: String,
    },
    Resume {
        workflow: Value,
        run: Box<WorkflowExecution>,
        node_id: String,
        approve: bool,
    },
}

fn walk_once(work: &Arc<Work>, walker: &Walker, job: &Job) -> Result<String, String> {
    match (walker, job) {
        (Walker::Native, Job::Start { workflow, run_id }) => {
            let w = walk::start(workflow, run_id.clone(), &mut RunBridge(Arc::clone(work)))?;
            Ok(w.outcome.as_str().to_owned())
        }
        (
            Walker::Native,
            Job::Resume {
                workflow,
                run,
                node_id,
                approve,
            },
        ) => {
            let w = walk::resume(
                workflow,
                (**run).clone(),
                node_id,
                *approve,
                &mut RunBridge(Arc::clone(work)),
            )?;
            Ok(w.outcome.as_str().to_owned())
        }
        (Walker::Wasm(component), job) => {
            let mut instance = component
                .instantiate(Box::new(RunBridge(Arc::clone(work))))
                .map_err(|e| e.to_string())?;
            let outcome = match job {
                Job::Start { workflow, run_id } => instance.start(&workflow.to_string(), run_id),
                Job::Resume {
                    workflow,
                    run,
                    node_id,
                    approve,
                } => {
                    let run = serde_json::to_string(&**run).map_err(|e| e.to_string())?;
                    instance.resume(&workflow.to_string(), &run, node_id, *approve)
                }
            };
            Ok(outcome.map_err(|e| e.to_string())??.status)
        }
    }
}

impl Work {
    /// Walks a fired workflow in the spike's step loop; false when the spike does not run workflows.
    pub(crate) fn spike_start(self: &Arc<Self>, workflow: Value) -> bool {
        if ext_spike::get().is_none_or(|s| s.walker.is_none()) {
            return false;
        }
        start_writer(self);
        let run_id = uuid::Uuid::new_v4().to_string();
        self.spike_walk(Job::Start { workflow, run_id });
        true
    }

    fn spike_walk(self: &Arc<Self>, job: Job) {
        let work = Arc::clone(self);
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let Some(walker) = ext_spike::get().and_then(|s| s.walker.as_ref()) else {
                return;
            };
            let workflow = match &job {
                Job::Start { workflow, .. } | Job::Resume { workflow, .. } => workflow,
            };
            let (id, name) = (
                workflow["id"].as_str().unwrap_or_default().to_owned(),
                workflow["name"].as_str().unwrap_or_default().to_owned(),
            );
            let status = match walk_once(&work, walker, &job) {
                Ok(status) => status,
                Err(err) => {
                    warn!(%err, workflow = %id, "the step loop failed");
                    return;
                }
            };
            if status == Outcome::Parked.as_str() {
                return;
            }
            runtime.spawn(async move {
                let completion = Completion {
                    workflow_id: id,
                    workflow_name: name,
                    completed_at: vorn_work::js::iso_now(),
                    status,
                    sessions_launched: 0,
                    source: Some(Source::Scheduler),
                };
                work.host().report_complete(completion).await;
            });
        });
    }

    /// `workflow:resolveGate` for a run the spike walks; `None` when the spike does not run workflows.
    pub(crate) async fn spike_resolve_gate(self: &Arc<Self>, params: &Value) -> Option<Answer> {
        ext_spike::get()?.walker.as_ref()?;
        start_writer(self);
        let text = |k: &str| {
            params
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (run_id, node_id, decision) = (text("runId"), text("nodeId"), text("decision"));
        let stored = {
            let id = run_id.clone();
            self.db
                .run(move |s| s.call("getWorkflowRun", json!([id])).ok())
                .await
                .flatten()
        };
        let Some(run) = stored.and_then(|v| serde_json::from_value::<WorkflowExecution>(v).ok())
        else {
            return Some(Answer::Result(json!({ "accepted": false })));
        };
        let Some(workflow) = self.workflow_row(&run.workflow_id).await else {
            return Some(Answer::Result(json!({ "accepted": false })));
        };
        let approve = decision == "approve";
        self.spike_walk(Job::Resume {
            workflow,
            run: Box::new(run),
            node_id: node_id.clone(),
            approve,
        });
        self.host().broadcast(
            "workflow:gateResolved",
            json!({ "runId": run_id, "nodeId": node_id, "decision": decision }),
        );
        Some(Answer::Result(json!({ "accepted": true })))
    }
}
