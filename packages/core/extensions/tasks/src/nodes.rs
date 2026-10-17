//! The workflow step Tasks declares (`[[contributes.nodes]]`), run when a
//! workflow reaches one: `createTaskFromItem` puts the connector item a run
//! was started with on the board, through the Connectors built-in's
//! `connection.upsertFromItem`, which knows the item's connection.

use serde_json::{json, Map, Value};

use crate::js;

/// What `createTaskFromItem` asks `connection.upsertFromItem`, or the step's
/// answer when the run carries no connector item.
pub fn upsert_params(request: &Value) -> Result<Value, Value> {
    let Some(item) = request
        .pointer("/context/connectorItem")
        .filter(|i| i.is_object())
        .cloned()
    else {
        return Err(json!({
            "status": "skipped",
            "error": "No connector item in context — this node only runs from a connectorPoll trigger.",
        }));
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
    params.insert("item".into(), item);
    params.insert(
        "initialStatus".into(),
        config.get("initialStatus").cloned().unwrap_or(Value::Null),
    );
    if let Some(p) = project {
        params.insert("project".into(), Value::String(p.to_owned()));
    }
    Ok(Value::Object(params))
}

/// The step's answer to what `connection.upsertFromItem` answered for `params`.
pub fn answered(params: &Value, result: Result<Value, String>) -> Value {
    let item = params.get("item").unwrap_or(&Value::Null);
    match result {
        Ok(result) => {
            let created = js::truthy(result.get("created").unwrap_or(&Value::Null));
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

/// `createTaskFromItem`: the run's connector item as a task, created or
/// brought up to date, in the step's project or its connection's.
pub fn create_task_from_item(
    request: &Value,
    mut call: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Value {
    match upsert_params(request) {
        Ok(params) => {
            let result = call("connection.upsertFromItem", params.clone());
            answered(&params, result)
        }
        Err(skipped) => skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_the_runs_item_or_skips_a_run_without_one() {
        let skipped = create_task_from_item(&json!({ "context": {} }), |_, _| unreachable!());
        assert_eq!(skipped["status"], "skipped");
        let request = json!({
            "context": { "connectorItem": { "connectionId": "c", "externalId": 7, "title": "x".repeat(70) } },
            "raw": { "project": "app", "initialStatus": "todo" },
        });
        let mut asked = Vec::new();
        let answer = create_task_from_item(&request, |m, a| {
            asked.push((m.to_owned(), a));
            Ok(json!({ "taskId": "t1", "created": true }))
        });
        assert_eq!(asked[0].0, "connection.upsertFromItem");
        assert_eq!(asked[0].1["project"], "app");
        assert_eq!(answer["status"], "success");
        assert_eq!(answer["output"], format!("Imported #7 \"{}...\"", "x".repeat(57)));
        assert!(answer["logs"].as_str().unwrap().ends_with("(no url)\nTaskId: t1"));
        let failed = create_task_from_item(&request, |_, _| Err("gone".into()));
        assert_eq!(failed, json!({ "status": "error", "error": "gone" }));
    }
}
