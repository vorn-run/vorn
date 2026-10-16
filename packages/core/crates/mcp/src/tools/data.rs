//! Tasks, projects, workspaces and workflows. Projects and workspaces are
//! read through the config blob that `config:load` carries and written back
//! through `config:save`, load-change-save per call; tasks and workflows are
//! not in it, and are read and written through their own methods, so the
//! server applies its rules and arms a workflow's schedule as it stores the
//! change.
//! A save carries the revision it was based on, so the server keeps what
//! anyone else added in between.

use serde_json::{json, Map, Value};

use super::{pretty, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// `get_config`: the whole configuration, its tasks and workflows read
/// beside it and put back in their usual place.
pub async fn get_config<R: Rpc>(cx: &Cx<'_, R>) -> Outcome {
    let config = load(cx).await?;
    let Value::Object(config) = config else {
        return Ok(pretty(&config));
    };
    let (workflows, tasks) = (list(cx, "workflows").await?, list(cx, "tasks").await?);
    let mut whole = Map::new();
    for (key, value) in config {
        whole.insert(key.clone(), value);
        if key == "agentCommands" {
            whole.insert("workflows".into(), Value::Array(workflows.clone()));
        }
        if key == "remoteHosts" {
            whole.insert("tasks".into(), Value::Array(tasks.clone()));
        }
    }
    Ok(pretty(&Value::Object(whole)))
}

/// `configManager.loadConfig()`, as the server answers `config:load`.
pub async fn load<R: Rpc>(cx: &Cx<'_, R>) -> Result<Value, String> {
    Ok(cx.call("config:load", None).await?)
}

/// How many tasks one read of the core's task list takes.
const TASK_PAGE: u64 = 1000;

/// `(await loadConfig()).<list> ?? []`; tasks, with their descriptions, and
/// workflows from their own calls.
pub async fn list<R: Rpc>(cx: &Cx<'_, R>, key: &str) -> Result<Vec<Value>, String> {
    let read = match key {
        "tasks" => return task_pages(cx, json!({})).await,
        "workflows" => cx.call("workflow:list", None).await?,
        _ => {
            let config = load(cx).await?;
            return Ok(super::items(json::prop(Some(&config), key)?).to_vec());
        }
    };
    Ok(super::items(Some(&read)).to_vec())
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

/// Calls a write method that answers `{ ok, ... }`: the answer, or `None`
/// when `ok` is false because the row it names is not there.
pub async fn write<R: Rpc>(
    cx: &Cx<'_, R>,
    method: &str,
    params: Value,
) -> Result<Option<Value>, String> {
    let answer = cx.call(method, Some(params)).await?;
    Ok(json::truthy(answer.get("ok")).then_some(answer))
}

/// `rpcCall('workflow:create', { workflow })`.
pub async fn create_workflow<R: Rpc>(cx: &Cx<'_, R>, workflow: &Value) -> Result<(), String> {
    cx.call("workflow:create", Some(json!({ "workflow": workflow })))
        .await?;
    Ok(())
}

/// `dbUpdateWorkflow`: a workflow gone by the time it is written is an
/// error, not a save.
pub async fn update_workflow<R: Rpc>(
    cx: &Cx<'_, R>,
    id: &str,
    updates: &Value,
) -> Result<(), String> {
    let params = json!({ "id": id, "updates": updates });
    match write(cx, "workflow:update", params).await? {
        Some(_) => Ok(()),
        None => Err(workflow_gone(id)),
    }
}

/// `dbDeleteWorkflow`, failing the same way.
pub async fn delete_workflow<R: Rpc>(cx: &Cx<'_, R>, id: &str) -> Result<(), String> {
    match write(cx, "workflow:delete", json!({ "id": id })).await? {
        Some(_) => Ok(()),
        None => Err(workflow_gone(id)),
    }
}

fn workflow_gone(id: &str) -> String {
    format!("workflow \"{id}\" not found")
}

/// One field of an update: `Some` sets it, `None` leaves it out of the save.
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

/// The core's task list under `filter`, with descriptions, a page at a time.
async fn task_pages<R: Rpc>(cx: &Cx<'_, R>, filter: Value) -> Result<Vec<Value>, String> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let mut params = filter.clone();
        params["includeDescription"] = json!(true);
        params["offset"] = json!(offset);
        params["limit"] = json!(TASK_PAGE);
        let page = cx.call("task:list", Some(params)).await?;
        let rows = super::items(Some(&page));
        let n = rows.len() as u64;
        all.extend_from_slice(rows);
        if n < TASK_PAGE {
            return Ok(all);
        }
        offset += n;
    }
}

/// `dbListTasks(projectName, status)`: the list filters, then each name is
/// matched exactly, as the list skips an empty one.
pub async fn tasks<R: Rpc>(
    cx: &Cx<'_, R>,
    project: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Value>, String> {
    let mut filter = json!({});
    if let Some(p) = project.filter(|p| !p.is_empty()) {
        filter["projectName"] = json!(p);
    }
    if let Some(s) = status.filter(|s| !s.is_empty()) {
        filter["status"] = json!(s);
    }
    Ok(task_pages(cx, filter)
        .await?
        .into_iter()
        .filter(|t| {
            project.is_none_or(|p| field_is(t, "projectName", p))
                && status.is_none_or(|s| field_is(t, "status", s))
        })
        .collect())
}

/// `task:get`: the task `id`, with its description, or `None`.
pub async fn task<R: Rpc>(cx: &Cx<'_, R>, id: &str) -> Result<Option<Value>, String> {
    let task = cx.call("task:get", Some(json!({ "id": id }))).await?;
    Ok(task.is_object().then_some(task))
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
