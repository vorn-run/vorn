//! Tasks, projects, workspaces and workflows, read and written as the
//! TypeScript's `data-access.ts` does: through the config blob that
//! `config:load` and `config:save` carry, load-change-save per call. A save
//! carries the revision it was based on, so the server keeps what anyone else
//! added in between.

use serde_json::{json, Map, Value};

use super::{pretty, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// `get_config`: the whole configuration.
pub async fn get_config<R: Rpc>(cx: &Cx<'_, R>) -> Outcome {
    Ok(pretty(&load(cx).await?))
}

/// `configManager.loadConfig()`, as the server answers `config:load`.
pub async fn load<R: Rpc>(cx: &Cx<'_, R>) -> Result<Value, String> {
    Ok(cx.call("config:load", None).await?)
}

/// `(await loadConfig()).<list> ?? []`.
pub async fn list<R: Rpc>(cx: &Cx<'_, R>, key: &str) -> Result<Vec<Value>, String> {
    let config = load(cx).await?;
    Ok(super::items(json::prop(Some(&config), key)?).to_vec())
}

/// Load, change one list, save: the shape every write takes.
async fn mutate<R: Rpc>(
    cx: &Cx<'_, R>,
    key: &str,
    change: impl FnOnce(Vec<Value>) -> Vec<Value>,
) -> Result<(), String> {
    let config = load(cx).await?;
    let current = super::items(json::prop(Some(&config), key)?).to_vec();
    // `{ ...config, [key]: ... }`: a spread of anything but an object adds no keys.
    let mut next = match config {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    next.insert(key.to_owned(), Value::Array(change(current)));
    cx.call("config:save", Some(Value::Object(next))).await?;
    Ok(())
}

/// One field of an update: `Some` sets it, `None` is the TypeScript's
/// `undefined`, which a save drops.
pub type Update = Vec<(&'static str, Option<Value>)>;

/// `{ ...row, ...updates }`.
pub fn merged(row: &Value, updates: &Update) -> Value {
    let mut out = match row {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    for (key, value) in updates {
        match value {
            Some(v) => {
                out.insert((*key).to_owned(), v.clone());
            }
            None => {
                out.shift_remove(*key);
            }
        }
    }
    Value::Object(out)
}

fn field_is(row: &Value, key: &str, value: &str) -> bool {
    row.get(key).and_then(Value::as_str) == Some(value)
}

/// Appends `row` to the list under `key`.
pub async fn insert<R: Rpc>(cx: &Cx<'_, R>, key: &str, row: Value) -> Result<(), String> {
    mutate(cx, key, |mut rows| {
        rows.push(row);
        rows
    })
    .await
}

/// Merges `updates` into each row whose `by` field is `id`.
pub async fn update<R: Rpc>(
    cx: &Cx<'_, R>,
    key: &str,
    by: &str,
    id: &str,
    updates: &Update,
) -> Result<(), String> {
    mutate(cx, key, |rows| {
        rows.into_iter()
            .map(|row| {
                if field_is(&row, by, id) {
                    merged(&row, updates)
                } else {
                    row
                }
            })
            .collect()
    })
    .await
}

/// Drops every row whose `by` field is `id`.
pub async fn delete<R: Rpc>(cx: &Cx<'_, R>, key: &str, by: &str, id: &str) -> Result<(), String> {
    mutate(cx, key, |rows| {
        rows.into_iter()
            .filter(|row| !field_is(row, by, id))
            .collect()
    })
    .await
}

/// The first row of `key` whose `by` field is `id`.
pub async fn find<R: Rpc>(
    cx: &Cx<'_, R>,
    key: &str,
    by: &str,
    id: &str,
) -> Result<Option<Value>, String> {
    Ok(list(cx, key)
        .await?
        .into_iter()
        .find(|row| field_is(row, by, id)))
}

/// `dbListTasks(projectName, status)`.
pub async fn tasks<R: Rpc>(
    cx: &Cx<'_, R>,
    project: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Value>, String> {
    Ok(list(cx, "tasks")
        .await?
        .into_iter()
        .filter(|t| {
            project.is_none_or(|p| field_is(t, "projectName", p))
                && status.is_none_or(|s| field_is(t, "status", s))
        })
        .collect())
}

/// `rpcCall('workflowRun:list', { workflowId, limit })`.
pub async fn workflow_runs<R: Rpc>(
    cx: &Cx<'_, R>,
    workflow: &str,
    limit: Value,
) -> Result<Value, String> {
    Ok(cx
        .call(
            "workflowRun:list",
            Some(json!({ "workflowId": workflow, "limit": limit })),
        )
        .await?)
}

/// `rpcCall('workflowRun:listByTask', { taskId, limit })`.
pub async fn workflow_runs_by_task<R: Rpc>(
    cx: &Cx<'_, R>,
    task: &str,
    limit: Value,
) -> Result<Value, String> {
    Ok(cx
        .call(
            "workflowRun:listByTask",
            Some(json!({ "taskId": task, "limit": limit })),
        )
        .await?)
}

/// `rpcCall('workflowRun:listWaiting')`.
pub async fn runs_with_waiting_gates<R: Rpc>(cx: &Cx<'_, R>) -> Result<Value, String> {
    Ok(cx.call("workflowRun:listWaiting", None).await?)
}

/// `rpcCall('workflowRun:listAll', { workspaceId: undefined, limit })`.
pub async fn all_workflow_runs<R: Rpc>(cx: &Cx<'_, R>, limit: u32) -> Result<Value, String> {
    Ok(cx
        .call("workflowRun:listAll", Some(json!({ "limit": limit })))
        .await?)
}
