//! The tools, each ported from its TypeScript namesake in `packages/mcp/src/tools`.
//!
//! A tool takes its arguments as the schema parsed them and answers with a
//! `CallToolResult`. `Err` is a tool that threw: the SDK turns the message
//! into an error result, and so does [`crate::server`]. Where the TypeScript
//! reads a field off something that may not be an object, the port does the
//! same through [`json::prop`], so even its TypeErrors read alike.

mod artifacts;
mod browser;
mod connectors;
mod data;
mod device;
mod projects;
mod sessions;
mod tasks;
mod workflows;
mod workspaces;

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::json;
use crate::rpc::{Caller, Rpc, RpcError, DEFAULT_TIMEOUT};

pub(crate) use connectors::connection_connector_id;

/// A tool's answer, or the message of what it threw.
pub(crate) type Outcome = Result<Value, String>;

/// What a tool runs with: the server to call, and who is calling.
pub(crate) struct Cx<'a, R> {
    pub rpc: &'a R,
    pub caller: &'a Caller,
}

impl<R: Rpc> Cx<'_, R> {
    /// `rpcCall(method, params)`.
    pub async fn call(&self, method: &str, params: Option<Value>) -> Result<Value, RpcError> {
        self.rpc.call(method, params, DEFAULT_TIMEOUT).await
    }

    /// `rpcCall(method, params, timeout)`.
    pub async fn call_for(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        self.rpc.call(method, params, timeout).await
    }

    /// `rpcNotify(method, params)`.
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        self.rpc.notify(method, params).await
    }
}

/// A tool's parsed arguments: an object, or nothing for a tool without a schema.
pub(crate) struct Args(Map<String, Value>);

impl Args {
    pub fn new(value: Option<Value>) -> Args {
        match value {
            Some(Value::Object(map)) => Args(map),
            _ => Args(Map::new()),
        }
    }

    /// `args.key`, `None` when it was not given.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    /// `if (args.key)`.
    pub fn truthy(&self, key: &str) -> bool {
        json::truthy(self.0.get(key))
    }

    /// A string argument when it is set and not empty: `args.key && ...`.
    pub fn nonempty(&self, key: &str) -> Option<&str> {
        self.str(key).filter(|s| !s.is_empty())
    }

    pub fn map(&self) -> &Map<String, Value> {
        &self.0
    }
}

/// An object of the entries that are set, in order: the way a JavaScript
/// object literal with `undefined` values reads once it is JSON.
pub(crate) fn object<'k>(entries: impl IntoIterator<Item = (&'k str, Option<Value>)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
            .collect(),
    )
}

/// A result of one text block.
pub(crate) fn text(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

/// A result of one text block, marked as an error.
pub(crate) fn failed(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true })
}

/// `{ content: [{ type: 'text', text: JSON.stringify(value, null, 2) }] }`.
pub(crate) fn pretty(value: &Value) -> Value {
    text(json::pretty(value))
}

/// `Error: ${err instanceof Error ? err.message : err}`.
pub(crate) fn error_of(err: impl Into<String>) -> Value {
    failed(format!("Error: {}", err.into()))
}

/// `value ?? []` as an array to walk: anything that is not one walks as empty.
pub(crate) fn items(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Runs the tool `name` with arguments its schema has already parsed.
pub(crate) async fn call<R: Rpc>(cx: &Cx<'_, R>, name: &str, args: Args) -> Outcome {
    match name {
        "get_config" => data::get_config(cx).await,

        "list_sessions" => sessions::list_sessions(cx, &args).await,
        "launch_session" => sessions::launch_session(cx, &args).await,
        "kill_session" => sessions::kill_session(cx, &args).await,
        "rename_session" => sessions::rename_session(cx, &args).await,
        "reorder_sessions" => sessions::reorder_sessions(cx, &args).await,
        "read_session_output" => sessions::read_session_output(cx, &args).await,
        "write_to_terminal" => sessions::write_to_terminal(cx, &args).await,
        "send_key" => sessions::send_key(cx, &args).await,
        "list_session_events" => sessions::list_session_events(cx, &args).await,

        "list_connectors" => connectors::list_connectors(cx, &args).await,
        "list_connections" => connectors::list_connections(cx, &args).await,
        "list_connector_actions" => connectors::list_connector_actions(cx, &args).await,
        "inspect_connector_package" => connectors::inspect_connector_package(cx, &args).await,
        "install_connector" => connectors::install_connector(cx, &args).await,
        "run_connector_action" => connectors::run_connector_action(cx, &args).await,
        "backfill_connection" => connectors::backfill_connection(cx, &args).await,

        "read_page" => browser::read_page(cx, &args).await,
        "get_page_text" => browser::get_page_text(cx, &args).await,
        "read_console_messages" => browser::read_console_messages(cx, &args).await,
        "read_network_requests" => browser::read_network_requests(cx, &args).await,
        "browser_screenshot" => browser::browser_screenshot(cx, &args).await,
        "browser_find" => browser::browser_find(cx, &args).await,
        "browser_interact" => browser::browser_interact(cx, &args).await,
        "open_browser_pane" => browser::open_browser_pane(cx, &args).await,
        "browser_tabs" => browser::browser_tabs(cx, &args).await,
        "browser_navigate" => browser::browser_navigate(cx, &args).await,
        "browser_history" => browser::browser_history(cx, &args).await,

        "device_list" => device::device_list(cx).await,
        "device_claim" => device::device_claim(cx, &args).await,
        "device_release" => device::device_release(cx).await,
        "read_screen" => device::read_screen(cx, &args).await,
        "device_find" => device::device_find(cx, &args).await,
        "device_interact" => device::device_interact(cx, &args).await,
        "device_screenshot" => device::device_screenshot(cx, &args).await,
        "device_launch" => device::device_launch(cx, &args).await,
        "device_terminate" => device::device_terminate(cx, &args).await,
        "device_install" => device::device_install(cx, &args).await,
        "device_open_url" => device::device_open_url(cx, &args).await,
        "device_logs" => device::device_logs(cx, &args).await,
        "open_device_pane" => device::open_device_pane(cx, &args).await,

        "publish_artifact" => artifacts::publish_artifact(cx, &args).await,
        "list_artifacts" => artifacts::list_artifacts(cx, &args).await,
        "read_artifact_comments" => artifacts::read_artifact_comments(cx, &args).await,
        "read_artifact" => artifacts::read_artifact(cx, &args).await,

        "list_projects" => projects::list_projects(cx, &args).await,
        "create_project" => projects::create_project(cx, &args).await,
        "update_project" => projects::update_project(cx, &args).await,
        "delete_project" => projects::delete_project(cx, &args).await,

        "list_tasks" => tasks::list_tasks(cx, &args).await,
        "create_task" => tasks::create_task(cx, &args).await,
        "get_task" => tasks::get_task(cx, &args).await,
        "update_task" => tasks::update_task(cx, &args).await,
        "delete_task" => tasks::delete_task(cx, &args).await,
        "archive_task" => tasks::archive_task(cx, &args).await,
        "unarchive_task" => tasks::unarchive_task(cx, &args).await,
        "get_my_context" => tasks::get_my_context(cx, &args).await,

        "list_workflows" => workflows::list_workflows(cx, &args).await,
        "create_workflow" => workflows::create_workflow(cx, &args).await,
        "update_workflow" => workflows::update_workflow(cx, &args).await,
        "delete_workflow" => workflows::delete_workflow(cx, &args).await,
        "list_workflow_runs" => workflows::list_workflow_runs(cx, &args).await,
        "stop_workflow_run" => workflows::stop_workflow_run(cx, &args).await,
        "resolve_gate" => workflows::resolve_gate(cx, &args).await,
        "get_workflow_schedule" => workflows::get_workflow_schedule(cx, &args).await,
        "execute_workflow" => workflows::execute_workflow(cx, &args).await,
        "export_workflow" => workflows::export_workflow(cx, &args).await,
        "import_workflow" => workflows::import_workflow(cx, &args).await,
        "describe_workflow_nodes" => Ok(workflows::describe_workflow_nodes(&args)),

        "list_workspaces" => workspaces::list_workspaces(cx).await,
        "create_workspace" => workspaces::create_workspace(cx, &args).await,
        "update_workspace" => workspaces::update_workspace(cx, &args).await,
        "delete_workspace" => workspaces::delete_workspace(cx, &args).await,

        // The protocol answers an unknown tool before it gets here; this is
        // a tool in the generated list that nothing here implements.
        other => Err(format!("MCP error -32602: Tool {other} not found")),
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;
