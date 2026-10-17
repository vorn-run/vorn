//! The MCP tools Tasks adds to Vorn's server: listing, reading, creating,
//! updating, archiving and deleting tasks, worded as `tools/tasks.ts` words
//! them. Vorn's server checks their arguments against the TypeScript's
//! schemas before they get here. Reads go to the core's task list a page at
//! a time, or to the one task by id; writes to the extension's own methods.

use serde_json::{json, Value};
use vorn_extension_sdk::Host;
use vorn_mcp::json;
use vorn_mcp::rpc::RpcError;
use vorn_mcp::toolkit::{failed, items, object, pretty, text, Args, Outcome};

use crate::Tasks;

/// How many tasks one read of the list takes.
const PAGE: u64 = 1000;

/// The public API's name for a client's call the tools make.
fn api_name(method: &str) -> &str {
    match method {
        "agent:list" => "agents.list",
        "task:get" => "task.get",
        other => other,
    }
}

/// The extension, as its tools call it: its own methods, and the core's for the rest.
struct Cx<'a> {
    tasks: &'a mut Tasks,
    host: &'a mut dyn Host,
}

impl Cx<'_> {
    /// A call as a client makes one; a failure reads as the server's answer would.
    fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let params = params.unwrap_or(Value::Null);
        let answered = if method.starts_with("task:") && method != "task:get" {
            self.tasks.answer(self.host, method, &params)
        } else {
            self.host.call(api_name(method), params)
        };
        answered.map_err(|e| RpcError::answered(&e.message).into())
    }
}

/// Runs tool `name` with arguments Vorn's server already checked: a
/// `CallToolResult`, or `None` for a tool Tasks does not add.
pub fn call(tasks: &mut Tasks, host: &mut dyn Host, name: &str, arguments: Value) -> Option<Value> {
    let cx = &mut Cx { tasks, host };
    let args = Args::new(Some(arguments));
    let outcome = match name {
        "list_tasks" => list_tasks(cx, &args),
        "create_task" => create_task(cx, &args),
        "get_task" => get_task(cx, &args),
        "update_task" => update_task(cx, &args),
        "delete_task" => delete_task(cx, &args),
        "archive_task" => archive_task(cx, &args),
        "unarchive_task" => unarchive_task(cx, &args),
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

/// The error a tool answers when one of `agents` is no agent that is on in
/// Vorn, or `None` when every one is. A Vorn that cannot list its agents
/// lets the call through, and the write decides.
fn agents_off(cx: &mut Cx<'_>, agents: &[&str]) -> Option<Value> {
    if agents.is_empty() {
        return None;
    }
    let listed = cx.call("agent:list", None).ok()?;
    let on: Vec<&str> = items(Some(&listed))
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    let off = agents.iter().find(|a| !on.contains(a))?;
    let which = if on.is_empty() {
        "No agent is on; switch one on in Settings › Extensions.".to_owned()
    } else {
        format!("Agents that are on: {}.", on.join(", "))
    };
    Some(failed(format!(
        "Error: agent \"{off}\" is not on in Vorn. {which}"
    )))
}

/// Calls a write method that answers `{ ok, ... }`: the answer, or `None`
/// when `ok` is false because the row it names is not there.
fn write(cx: &mut Cx<'_>, method: &str, params: Value) -> Result<Option<Value>, String> {
    let answer = cx.call(method, Some(params))?;
    Ok(json::truthy(answer.get("ok")).then_some(answer))
}

/// The task `id`, with its description, or `None`.
fn find(cx: &mut Cx<'_>, id: &str) -> Result<Option<Value>, String> {
    let task = cx.call("task:get", Some(json!({ "id": id })))?;
    Ok(task.is_object().then_some(task))
}

/// The tasks in `project` and `status`, all of them read a page at a time
/// from the core's list (`task.list`). The list's filters skip an empty
/// name, which matches no task here.
fn tasks(
    cx: &mut Cx<'_>,
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
        let page = cx.call("task.list", Some(params))?;
        all.extend(items(page.get("tasks")).iter().cloned());
        offset = page.get("next").cloned().unwrap_or(Value::Null);
    }
    Ok(all)
}

/// `(p.workspaceId ?? 'personal') === workspaceId`.
fn in_workspace(project: &Value, workspace: &str) -> bool {
    match project.get("workspaceId") {
        None | Some(Value::Null) => workspace == "personal",
        Some(Value::String(w)) => w == workspace,
        Some(_) => false,
    }
}

fn list_tasks(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let mut tasks = tasks(cx, args.str("project_name"), args.str("status"))?;
    if !args.truthy("include_archived") {
        tasks.retain(|t| !json::truthy(t.get("archivedAt")));
    }
    if let Some(workspace) = args.nonempty("workspace_id") {
        let projects = cx.call("project.list", None)?;
        let names: Vec<&Value> = items(Some(&projects))
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

fn create_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let assigned: Vec<&str> = args.nonempty("assigned_agent").into_iter().collect();
    if let Some(off) = agents_off(cx, &assigned) {
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
    match write(cx, "task:create", params)? {
        Some(answer) => Ok(pretty(answer.get("task").unwrap_or(&Value::Null))),
        None => Ok(failed(format!(
            "Error: project \"{}\" not found",
            args.str("project_name").unwrap_or_default()
        ))),
    }
}

fn get_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    match find(cx, id)? {
        Some(task) => Ok(pretty(&task)),
        None => Ok(not_found(id)),
    }
}

fn update_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let assigned: Vec<&str> = args.nonempty("assigned_agent").into_iter().collect();
    if let Some(off) = agents_off(cx, &assigned) {
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
    let Some(answer) = write(cx, "task:update", params)? else {
        return Ok(not_found(id));
    };
    let task = answer.get("task").cloned().unwrap_or(Value::Null);
    match args.get("order").and_then(Value::as_f64) {
        Some(order) => place(cx, &task, order),
        None => Ok(pretty(&task)),
    }
}

/// `task:update` ignores `order`, so a requested order becomes a place on the
/// board: before the first other task ordered after it, applied by
/// `task:reorder`, which permutes the slots the project's tasks already hold.
fn place(cx: &mut Cx<'_>, task: &Value, order: f64) -> Outcome {
    let id = task.get("id").and_then(Value::as_str).unwrap_or_default();
    let order_of = |t: &Value| t.get("order").and_then(Value::as_f64).unwrap_or(0.0);
    let project = task.get("projectName").and_then(Value::as_str);
    let mut board = tasks(cx, project, None)?;
    board.retain(|t| t.get("id").and_then(Value::as_str) != Some(id));
    board.sort_by(|a, b| order_of(a).total_cmp(&order_of(b)));
    let at = board.partition_point(|t| order_of(t) <= order);
    let mut ids: Vec<Value> = board.iter().filter_map(|t| t.get("id").cloned()).collect();
    ids.insert(at, Value::from(id));
    if write(cx, "task:reorder", json!({ "ids": ids }))?.is_none() {
        return Ok(not_found(id));
    }
    let placed = find(cx, id)?;
    Ok(pretty(&placed.unwrap_or(Value::Null)))
}

fn delete_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = find(cx, id)? else {
        return Ok(not_found(id));
    };
    if write(cx, "task:delete", json!({ "id": id }))?.is_none() {
        return Ok(not_found(id));
    }
    Ok(text(format!(
        "Deleted task: {}",
        json::display(task.get("title"))
    )))
}

/// `task:archive`, then the task as it is stored now.
fn set_archived(cx: &mut Cx<'_>, id: &str, archived: bool) -> Outcome {
    let params = json!({ "id": id, "archived": archived });
    if write(cx, "task:archive", params)?.is_none() {
        return Ok(not_found(id));
    }
    let updated = find(cx, id)?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

fn archive_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = find(cx, id)? else {
        return Ok(not_found(id));
    };
    if !terminal(task.get("status")) {
        return Ok(failed(format!(
            "Error: only done or cancelled tasks can be archived (status: {})",
            json::display(task.get("status"))
        )));
    }
    set_archived(cx, id, true)
}

fn unarchive_task(cx: &mut Cx<'_>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    if find(cx, id)?.is_none() {
        return Ok(not_found(id));
    }
    set_archived(cx, id, false)
}
