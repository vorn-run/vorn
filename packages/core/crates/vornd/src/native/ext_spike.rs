//! A benchmark spike: with `VORN_EXT_SPIKE=tasks,workflows`, Tasks and workflow runs leave the built-in code for a component.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{info, warn};
use vorn_ext_perf_host::{Bridge, Tasks, TasksComponent, WorkflowsComponent};
use vorn_protocol::WorkflowExecution;
use vorn_store::Store;

use super::config::{announce, blocking, options};
use super::{Answer, Native};

/// How the step loop runs: in the component, or the same code compiled into vornd.
pub enum Walker {
    Wasm(WorkflowsComponent),
    Native,
}

/// What the spike took over, read once from the environment.
pub struct Spike {
    tasks: Option<Mutex<Tasks>>,
    pub walker: Option<Walker>,
    /// A changed task goes out as one row; otherwise the whole configuration, as today.
    rows: bool,
    /// When each run was last told to viewers, and the steps it was on then.
    published: Mutex<HashMap<String, (Instant, String)>>,
}

/// Viewers are told a run whose steps did not move at most this often, as the engine does.
const LOG_ONLY_INTERVAL: Duration = Duration::from_secs(3);

fn read(dir: &str, name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(dir).join(name);
    std::fs::read(&path)
        .map_err(|e| warn!(%e, path = %path.display(), "the spike's component is missing"))
        .ok()
}

fn load() -> Option<Spike> {
    let wanted = std::env::var("VORN_EXT_SPIKE").ok()?;
    let dir = std::env::var("VORN_EXT_SPIKE_DIR").unwrap_or_default();
    let native = std::env::var("VORN_EXT_SPIKE_RUN").is_ok_and(|r| r == "native");
    let tasks = wanted.split(',').any(|w| w == "tasks").then(|| {
        let bytes = read(&dir, "tasks.cwasm")?;
        let component = TasksComponent::load(&bytes)
            .map_err(|e| warn!(%e, "tasks"))
            .ok()?;
        component
            .instantiate()
            .map_err(|e| warn!(%e, "tasks"))
            .ok()
            .map(Mutex::new)
    });
    let walker = wanted.split(',').any(|w| w == "workflows").then(|| {
        if native {
            return Some(Walker::Native);
        }
        let bytes = read(&dir, "workflows.cwasm")?;
        WorkflowsComponent::load(&bytes)
            .map(Walker::Wasm)
            .map_err(|e| warn!(%e, "workflows"))
            .ok()
    });
    let spike = Spike {
        tasks: tasks.flatten(),
        walker: walker.flatten(),
        rows: std::env::var("VORN_EXT_SPIKE_ANNOUNCE").map_or(true, |a| a != "config"),
        published: Mutex::default(),
    };
    info!(
        tasks = spike.tasks.is_some(),
        workflows = spike.walker.is_some(),
        native,
        rows = spike.rows,
        "the extension spike is on"
    );
    Some(spike)
}

/// The spike, when the environment turned it on.
pub fn get() -> Option<&'static Spike> {
    static SPIKE: OnceLock<Option<Spike>> = OnceLock::new();
    SPIKE.get_or_init(load).as_ref()
}

/// Storage calls answered from one store, opened for the call as the built-in path opens it.
struct StoreBridge(Store);

impl Bridge for StoreBridge {
    fn db(&mut self, op: &str, args: &str) -> Result<String, String> {
        let args: Value = serde_json::from_str(args).map_err(|e| format!("{op}: {e}"))?;
        self.0
            .call(op, args)
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    }
    fn run_changed(&mut self, _run: &str) {}
    fn log(&mut self, line: &str) {
        info!("{line}");
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A `task:*` call the component answers, or `None` to leave it to the built-in path.
pub async fn task(native: &Arc<Native>, method: &str, params: &Value) -> Option<Answer> {
    let spike = get()?;
    let tasks = spike.tasks.as_ref()?;
    if !vorn_ext_perf_guest::methods::METHODS.contains(&method) {
        return None;
    }
    let db = native.database()?.to_path_buf();
    let (m, p) = (method.to_owned(), params.to_string());
    let done = blocking(method, move || {
        let store = Store::open_beside(&db)
            .map_err(|e| e.to_string())?
            .ok_or("the database does not exist yet")?
            .with_defaults(options());
        let id = uuid::Uuid::new_v4().to_string();
        let mut tasks = tasks.lock().unwrap_or_else(|e| e.into_inner());
        let (answer, changed) = tasks
            .call(Box::new(StoreBridge(store)), &m, &p, &now_iso(), &id)
            .map_err(|e| e.to_string())??;
        let answer = answer
            .map(|a| serde_json::from_str::<Value>(&a))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok((answer, changed))
    })
    .await;
    Some(match done {
        Ok((answer, changed)) => {
            if changed && spike.rows {
                let id = answer
                    .as_ref()
                    .and_then(|a| a.pointer("/task/id"))
                    .or_else(|| params.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let row = answer.as_ref().and_then(|a| a.get("task")).cloned();
                native.broadcast(
                    "ext:taskRows",
                    json!({ "op": method, "id": id, "row": row }),
                );
            } else if changed {
                announce(native).await;
            }
            answer.map_or(Answer::Void, Answer::Result)
        }
        Err(message) => Answer::Error(message),
    })
}

impl Spike {
    /// Whether viewers hear of this version of the run, as the engine's `publish` decides.
    pub fn should_publish(&self, run: &WorkflowExecution) -> bool {
        let mut shape = run.status.clone();
        for ns in &run.node_states {
            shape.push('|');
            shape.push_str(&ns.node_id);
            shape.push(':');
            shape.push_str(&ns.status.0);
        }
        let now = Instant::now();
        let mut published = self.published.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, previous)) = published.get(&run.run_id) {
            if *previous == shape && now.duration_since(*at) < LOG_ONLY_INTERVAL {
                return false;
            }
        }
        if run.status == "running" {
            published.insert(run.run_id.clone(), (now, shape));
        } else {
            published.remove(&run.run_id);
        }
        true
    }
}
