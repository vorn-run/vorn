//! The workflow steps Tasks declares (`[[contributes.nodes]]`), run when a
//! workflow reaches one: `createTaskFromItem` puts the connector item a run
//! was started with on the board, through the Connectors built-in's
//! `connection.upsertFromItem`, which knows the item's connection.

use std::future::Future;

use serde_json::{json, Map, Value};
use vorn_work::js;

/// The step a node type answers, by its id; `None` for a type Tasks does not declare.
pub async fn run<F, Fut>(node_type: &str, request: &Value, call: F) -> Option<Value>
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    match node_type {
        "createTaskFromItem" => Some(create_task_from_item(request, call).await),
        _ => None,
    }
}

/// `createTaskFromItem`: the run's connector item as a task, created or
/// brought up to date, in the step's project or its connection's.
pub async fn create_task_from_item<F, Fut>(request: &Value, call: F) -> Value
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let Some(item) = request
        .pointer("/context/connectorItem")
        .filter(|i| i.is_object())
        .cloned()
    else {
        return json!({
            "status": "skipped",
            "error": "No connector item in context — this node only runs from a connectorPoll trigger.",
        });
    };
    let config = request.get("raw").unwrap_or(&Value::Null);
    let project = config
        .get("project")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty() && *p != "fromConnection");
    let mut params = Map::new();
    params.insert(
        "connectionId".into(),
        item.get("connectionId").cloned().unwrap_or(Value::Null),
    );
    params.insert("item".into(), item.clone());
    params.insert(
        "initialStatus".into(),
        config.get("initialStatus").cloned().unwrap_or(Value::Null),
    );
    if let Some(p) = project {
        params.insert("project".into(), Value::String(p.to_owned()));
    }
    match call("connection.upsertFromItem", Value::Object(params)).await {
        Ok(result) => {
            let created = vorn_work::is_truthy(result.get("created"));
            let task_id = result.get("taskId").map(js::to_string).unwrap_or_default();
            let title = item.get("title").map(js::to_string).unwrap_or_default();
            let snippet = if js::utf16_len(&title) > 60 {
                format!("{}...", js::head(&title, 57))
            } else {
                title
            };
            let external = item
                .get("externalId")
                .map_or_else(|| "undefined".into(), js::to_string);
            let summary = format!(
                "{} #{external} \"{snippet}\"",
                if created { "Imported" } else { "Updated" }
            );
            let url = item
                .get("externalUrl")
                .filter(|u| !u.is_null())
                .map_or_else(|| "(no url)".to_owned(), js::to_string);
            json!({
                "status": "success",
                "taskId": task_id,
                "logs": format!("{summary}\nSource: {url}\nTaskId: {task_id}"),
                "output": summary,
            })
        }
        Err(message) => json!({ "status": "error", "error": message }),
    }
}
