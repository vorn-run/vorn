//! Workflow steps and triggers: the node types and trigger types the
//! Workflows extension declares on the public extension API (`[[contributes.
//! nodes]]` and `[[contributes.triggers]]` in its manifest), run here and
//! filled into the core engine's registry by vornd while it is installed.
//!
//! Its steps run in vornd with the run at hand ([`vorn_workflow::Step`]):
//! an agent ([`agent`]), a script ([`script`]) and an HTTP request
//! ([`http`]). Of its triggers, the core times `recurring`, `once` and
//! `webhook`, a person starts `manual`, and vornd fires `sessionRestored`
//! itself ([`triggers`]). Node types other extensions declare (Tasks',
//! Connectors', one installed from git) sit beside these in the same
//! registry.

pub mod agent;
pub mod declare;
pub mod http;
pub mod script;
pub mod triggers;

use std::sync::Arc;

use serde_json::{Map, Value};
use vorn_workflow::{Host, Step};

/// The id the Workflows extension goes by.
pub const ID: &str = "vorn.workflows";

/// Workflows' preference for how long a step may run, a duration choice.
pub const STEP_TIME_LIMIT: &str = "headlessStepTimeoutMinutes";

/// The minutes a duration choice's `m<minutes>` id names.
pub fn minutes(id: &str) -> Option<f64> {
    id.strip_prefix('m')?.parse::<u32>().ok().map(f64::from)
}

/// The id Tasks goes by, whose `createTaskFromItem` step's doc is kept here.
pub const TASKS_ID: &str = "vorn.tasks";

/// What `describe_workflow_nodes` tells of its node types, by id.
pub const NODE_DOCS: &str = include_str!("../generated/nodes.json");

/// What `describe_workflow_nodes` tells of the node types Tasks declares, by id.
pub const TASKS_NODE_DOCS: &str = include_str!("../generated/tasks-nodes.json");

/// The workflows a new database is given, each with the flag that records
/// it was given them once: `(file, flag, text)`.
pub const SEEDS: &[(&str, &str, &str)] = &[
    (
        "seeds/default-task.toml",
        "hasSeededDefaultTaskWorkflow",
        include_str!("../seeds/default-task.toml"),
    ),
    (
        "seeds/dev-server.toml",
        "hasSeededDevServerWorkflow",
        include_str!("../seeds/dev-server.toml"),
    ),
];

/// The node type a step of a type no extension declares runs as: an agent,
/// as such a step always ran.
pub const FALLBACK: &str = "launchAgent";

/// Its steps, by node type, as the engine runs them in vornd.
pub fn steps<H: Host>() -> Vec<(&'static str, Arc<dyn Step<H>>)> {
    vec![
        ("launchAgent", Arc::new(agent::Agent)),
        ("script", Arc::new(script::Script)),
        ("httpRequest", Arc::new(http::HttpRequest)),
    ]
}

/// Its node types' docs, by id.
pub fn node_docs() -> Map<String, Value> {
    serde_json::from_str(NODE_DOCS).unwrap_or_default()
}

/// The docs of the node types Vorn's own extension `id` declares, by id;
/// empty for any other, whose docs are built from its manifest.
pub fn docs_of(id: &str) -> Map<String, Value> {
    let text = match id {
        ID => NODE_DOCS,
        TASKS_ID => TASKS_NODE_DOCS,
        _ => return Map::new(),
    };
    serde_json::from_str(text).unwrap_or_default()
}

pub(crate) fn text_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// `value || undefined` for a string.
pub(crate) fn nonempty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Writes `key` when the value is there, as JSON drops `undefined`.
pub(crate) fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        map.insert(key.to_owned(), v);
    }
}

/// `ActionResult.output` when it is a plain object: an array would turn
/// into index keys in the step's outputs.
pub(crate) fn plain_object(result: &Value) -> Option<Value> {
    result.get("output").filter(|o| o.is_object()).cloned()
}

pub(crate) fn result_error(result: &Value) -> Option<String> {
    result
        .get("error")
        .filter(|e| vorn_work::is_truthy(Some(e)))
        .map(vorn_work::js::to_string)
}
