//! The MCP tools Tasks adds to Vorn's server: listing, reading, creating,
//! updating, archiving and deleting tasks, worded as `tools/tasks.ts` words
//! them. Vorn's server checks their arguments against the TypeScript's
//! schemas before they get here. Reads go to the core's task list a page at
//! a time, or to the one task by id; writes to the built-in's own methods.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde_json::{json, Value};
use vorn_mcp::json;
use vorn_mcp::rpc::{Caller, Rpc, RpcError};
use vorn_mcp::toolkit::{agents_off, failed, object, pretty, text, Args, Cx, Outcome};

use crate::builtin::{Answer, Tasks};

static TOOLS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/tools.json"))
        .expect("generated/tools.json is JSON")
});

/// How many tasks one read of the list takes.
const PAGE: u64 = 1000;

/// The tools as `tools/list` lists them, exactly as the TypeScript defines them.
pub fn listed() -> &'static [Value] {
    &TOOLS
}

/// The public API's name for a client's call the shared toolkit makes.
fn api_name(method: &str) -> &str {
    match method {
        "agent:list" => "agents.list",
        "task:get" => "task.get",
        other => other,
    }
}

/// The built-in, as its tools call it: its own methods in process, the rest the core's.
struct Own<'a>(&'a Tasks);

impl Rpc for Own<'_> {
    async fn call(
        &self,
        method: &str,
        params: Option<Value>,
        _timeout: Duration,
    ) -> Result<Value, RpcError> {
        let params = params.unwrap_or(Value::Null);
        match self.0.answer(method, params.clone()).await {
            Answer::Result(v) => Ok(v),
            Answer::Void => Ok(Value::Null),
            Answer::Error(message) => Err(RpcError::answered(&message)),
            // The toolkit's own check asks as a client would; the built-in asks the API.
            Answer::Unanswered => self
                .0
                .core(api_name(method), params)
                .await
                .map_err(|m| RpcError::answered(&m)),
        }
    }

    async fn notify(&self, _method: &str, _params: Option<Value>) -> Result<(), RpcError> {
        Ok(())
    }
}

/// Runs tool `name` with arguments Vorn's server already checked: a
/// `CallToolResult`, or `None` for a tool Tasks does not add.
pub async fn call(tasks: &Arc<Tasks>, name: &str, arguments: Value) -> Option<Value> {
    let rpc = Own(tasks);
    let caller = Caller::default();
    let cx = Cx {
        rpc: &rpc,
        caller: &caller,
    };
    let args = Args::new(Some(arguments));
    let outcome = match name {
        "list_tasks" => list_tasks(&cx, &args).await,
        "create_task" => create_task(&cx, &args).await,
        "get_task" => get_task(&cx, &args).await,
        "update_task" => update_task(&cx, &args).await,
        "delete_task" => delete_task(&cx, &args).await,
        "archive_task" => archive_task(&cx, &args).await,
        "unarchive_task" => unarchive_task(&cx, &args).await,
        _ => return None,
    };
    Some(outcome.unwrap_or_else(
        |message| json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    ))
}

/// `isTerminalTaskStatus`.
fn terminal(status: Option<&Value>) -> bool {
    matches!(status.and_then(Value::as_str), Some("done" | "cancelled"))
}

fn not_found(id: &str) -> Value {
    failed(format!("Error: task \"{id}\" not found"))
}

/// Calls a write method that answers `{ ok, ... }`: the answer, or `None`
/// when `ok` is false because the row it names is not there.
async fn write<R: Rpc>(
    cx: &Cx<'_, R>,
    method: &str,
    params: Value,
) -> Result<Option<Value>, String> {
    let answer = cx.call(method, Some(params)).await?;
    Ok(json::truthy(answer.get("ok")).then_some(answer))
}

/// The task `id`, with its description, or `None`.
async fn find<R: Rpc>(cx: &Cx<'_, R>, id: &str) -> Result<Option<Value>, String> {
    let task = cx.call("task:get", Some(json!({ "id": id }))).await?;
    Ok(task.is_object().then_some(task))
}

/// The tasks in `project` and `status`, all of them read a page at a time
/// from the core's list (`task.list`). The list's filters skip an empty
/// name, which matches no task here.
async fn tasks<R: Rpc>(
    cx: &Cx<'_, R>,
    project: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Value>, String> {
    if project == Some("") || status == Some("") {
        return Ok(Vec::new());
    }
    let mut all = Vec::new();
    let mut offset = Value::from(0);
    while !offset.is_null() {
        let mut params = json!({ "includeDescription": true, "offset": offset, "limit": PAGE });
        if let Some(p) = project {
            params["projectName"] = json!(p);
        }
        if let Some(s) = status {
            params["status"] = json!(s);
        }
        let page = cx.call("task.list", Some(params)).await?;
        all.extend(vorn_mcp::toolkit::items(page.get("tasks")).iter().cloned());
        offset = page.get("next").cloned().unwrap_or(Value::Null);
    }
    Ok(all)
}

/// `{ id, title, status, branch }` of a task, for listing beside another.
pub async fn list_tasks<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let mut tasks = tasks(cx, args.str("project_name"), args.str("status")).await?;
    if !args.truthy("include_archived") {
        tasks.retain(|t| !json::truthy(t.get("archivedAt")));
    }
    if let Some(workspace) = args.nonempty("workspace_id") {
        let projects = cx.call("project.list", None).await?;
        let projects = vorn_mcp::toolkit::items(Some(&projects)).to_vec();
        let names: Vec<&Value> = projects
            .iter()
            .filter(|p| in_workspace(p, workspace))
            .filter_map(|p| p.get("name"))
            .collect();
        // A Set's `has` is SameValueZero: strings and numbers alike.
        tasks.retain(|t| {
            names
                .iter()
                .any(|n| json::strict_equals(Some(n), t.get("projectName")))
        });
    }
    if let Some(agent) = args.nonempty("assigned_agent") {
        tasks.retain(|t| t.get("assignedAgent").and_then(Value::as_str) == Some(agent));
    }
    Ok(pretty(&Value::Array(tasks)))
}

/// `(p.workspaceId ?? 'personal') === workspaceId`.
fn in_workspace(project: &Value, workspace: &str) -> bool {
    match project.get("workspaceId") {
        None | Some(Value::Null) => workspace == "personal",
        Some(Value::String(w)) => w == workspace,
        Some(_) => false,
    }
}

pub async fn create_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let assigned: Vec<&str> = args.nonempty("assigned_agent").into_iter().collect();
    if let Some(off) = agents_off(cx, &assigned).await {
        return Ok(off);
    }
    let params = object(
        [
            ("projectName", "project_name"),
            ("title", "title"),
            ("description", "description"),
            ("status", "status"),
            ("branch", "branch"),
            ("useWorktree", "use_worktree"),
            ("assignedAgent", "assigned_agent"),
        ]
        .map(|(key, arg)| (key, args.get(arg).cloned())),
    );
    match write(cx, "task:create", params).await? {
        Some(answer) => Ok(pretty(answer.get("task").unwrap_or(&Value::Null))),
        None => Ok(failed(format!(
            "Error: project \"{}\" not found",
            args.str("project_name").unwrap_or_default()
        ))),
    }
}

pub async fn get_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    match find(cx, id).await? {
        Some(task) => Ok(pretty(&task)),
        None => Ok(not_found(id)),
    }
}

pub async fn update_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let assigned: Vec<&str> = args.nonempty("assigned_agent").into_iter().collect();
    if let Some(off) = agents_off(cx, &assigned).await {
        return Ok(off);
    }
    let id = args.str("id").unwrap_or_default();
    let params = object(
        [
            ("id", "id"),
            ("title", "title"),
            ("description", "description"),
            ("status", "status"),
            ("branch", "branch"),
            ("useWorktree", "use_worktree"),
            ("assignedAgent", "assigned_agent"),
        ]
        .map(|(key, arg)| (key, args.get(arg).cloned())),
    );
    let Some(answer) = write(cx, "task:update", params).await? else {
        return Ok(not_found(id));
    };
    let task = answer.get("task").cloned().unwrap_or(Value::Null);
    match args.get("order").and_then(Value::as_f64) {
        Some(order) => place(cx, &task, order).await,
        None => Ok(pretty(&task)),
    }
}

/// `task:update` ignores `order`, so a requested order becomes a place on the
/// board: before the first other task ordered after it, applied by
/// `task:reorder`, which permutes the slots the project's tasks already hold.
async fn place<R: Rpc>(cx: &Cx<'_, R>, task: &Value, order: f64) -> Outcome {
    let id = task.get("id").and_then(Value::as_str).unwrap_or_default();
    let order_of = |t: &Value| t.get("order").and_then(Value::as_f64).unwrap_or(0.0);
    let project = task.get("projectName").and_then(Value::as_str);
    let mut board = tasks(cx, project, None).await?;
    board.retain(|t| t.get("id").and_then(Value::as_str) != Some(id));
    board.sort_by(|a, b| order_of(a).total_cmp(&order_of(b)));
    let at = board.partition_point(|t| order_of(t) <= order);
    let mut ids: Vec<Value> = board.iter().filter_map(|t| t.get("id").cloned()).collect();
    ids.insert(at, Value::from(id));
    if write(cx, "task:reorder", json!({ "ids": ids }))
        .await?
        .is_none()
    {
        return Ok(not_found(id));
    }
    let placed = find(cx, id).await?;
    Ok(pretty(&placed.unwrap_or(Value::Null)))
}

pub async fn delete_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = find(cx, id).await? else {
        return Ok(not_found(id));
    };
    if write(cx, "task:delete", json!({ "id": id }))
        .await?
        .is_none()
    {
        return Ok(not_found(id));
    }
    Ok(text(format!(
        "Deleted task: {}",
        json::display(task.get("title"))
    )))
}

/// `task:archive`, then the task as it is stored now.
async fn set_archived<R: Rpc>(cx: &Cx<'_, R>, id: &str, archived: bool) -> Outcome {
    let params = json!({ "id": id, "archived": archived });
    if write(cx, "task:archive", params).await?.is_none() {
        return Ok(not_found(id));
    }
    let updated = find(cx, id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn archive_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = find(cx, id).await? else {
        return Ok(not_found(id));
    };
    if !terminal(task.get("status")) {
        return Ok(failed(format!(
            "Error: only done or cancelled tasks can be archived (status: {})",
            json::display(task.get("status"))
        )));
    }
    set_archived(cx, id, true).await
}

pub async fn unarchive_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    if find(cx, id).await?.is_none() {
        return Ok(not_found(id));
    }
    set_archived(cx, id, false).await
}

#[cfg(test)]
mod tests;
