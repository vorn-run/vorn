//! The workflow step loop as a Safe-tier component: each run change crosses back to vornd as JSON.

wit_bindgen::generate!({ world: "workflows", path: "../../wit" });

use vorn::ext_perf::bridge;
use vorn_ext_perf_guest::workflows::{self as walk, WorkflowExecution};

struct Bridge;

impl walk::Host for Bridge {
    fn persist(&mut self, run: &WorkflowExecution) {
        bridge::run_changed(&serde_json::to_string(run).unwrap_or_default());
    }

    fn log(&mut self, line: &str) {
        bridge::log(line);
    }
}

struct Workflows;

fn outcome(walked: Result<walk::Walk, String>) -> Result<Outcome, String> {
    let w = walked?;
    Ok(Outcome {
        status: w.outcome.as_str().to_owned(),
        steps: w.steps,
    })
}

impl Guest for Workflows {
    fn start(workflow: String, run_id: String) -> Result<Outcome, String> {
        let raw: serde_json::Value = serde_json::from_str(&workflow).map_err(|e| e.to_string())?;
        outcome(walk::start(&raw, run_id, &mut Bridge))
    }

    fn resume(
        workflow: String,
        run: String,
        node_id: String,
        approve: bool,
    ) -> Result<Outcome, String> {
        let raw: serde_json::Value = serde_json::from_str(&workflow).map_err(|e| e.to_string())?;
        let run: WorkflowExecution = serde_json::from_str(&run).map_err(|e| e.to_string())?;
        outcome(walk::resume(&raw, run, &node_id, approve, &mut Bridge))
    }
}

export!(Workflows);
