//! `tools/tasks.ts`: tasks, read from the config blob and written through
//! the `task:*` methods, which apply the board's rules: where a new task
//! sits, and the dates a status change stamps or clears.

use serde_json::{json, Map, Value};

use super::data;
use super::{failed, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// `isTerminalTaskStatus`.
fn terminal(status: Option<&Value>) -> bool {
    matches!(status.and_then(Value::as_str), Some("done" | "cancelled"))
}

fn not_found(id: &str) -> Value {
    failed(format!("Error: task \"{id}\" not found"))
}

/// `path.resolve(p)` on POSIX, relative to the caller's directory.
pub(crate) fn resolve(cwd: &str, p: &str) -> String {
    let joined = if p.starts_with('/') {
        p.to_owned()
    } else {
        format!("{cwd}/{p}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `path.resolve(value)` for a value read out of the config, which Node
/// refuses unless it is a string.
fn resolve_value(cwd: &str, value: Option<&Value>) -> Result<String, String> {
    match value {
        Some(Value::String(s)) => Ok(resolve(cwd, s)),
        None => {
            Err("The \"paths[0]\" argument must be of type string. Received undefined".to_owned())
        }
        Some(Value::Null) => {
            Err("The \"paths[0]\" argument must be of type string. Received null".to_owned())
        }
        Some(other) => Err(format!(
            "The \"paths[0]\" argument must be of type string. Received type {} ({})",
            match other {
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                _ => "object",
            },
            json::display(Some(other))
        )),
    }
}

/// `{ id, title, status, branch }` of a task, for listing beside another.
fn brief(task: &Value) -> Value {
    object(["id", "title", "status", "branch"].map(|k| (k, task.get(k).cloned())))
}

pub async fn list_tasks<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let mut tasks = data::tasks(cx, args.str("project_name"), args.str("status")).await?;
    if !args.truthy("include_archived") {
        tasks.retain(|t| !json::truthy(t.get("archivedAt")));
    }
    if let Some(workspace) = args.nonempty("workspace_id") {
        let projects = data::list(cx, "projects").await?;
        let names: Vec<&Value> = projects
            .iter()
            .filter(|p| super::projects::in_workspace(p, workspace))
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

pub async fn create_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
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
    match data::write(cx, "task:create", params).await? {
        Some(answer) => Ok(pretty(answer.get("task").unwrap_or(&Value::Null))),
        None => Ok(failed(format!(
            "Error: project \"{}\" not found",
            args.str("project_name").unwrap_or_default()
        ))),
    }
}

pub async fn get_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    match data::find(cx, "tasks", "id", id).await? {
        Some(task) => Ok(pretty(&task)),
        None => Ok(not_found(id)),
    }
}

pub async fn update_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
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
    let Some(answer) = data::write(cx, "task:update", params).await? else {
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
    let mut board = data::tasks(cx, project, None).await?;
    board.retain(|t| t.get("id").and_then(Value::as_str) != Some(id));
    board.sort_by(|a, b| order_of(a).total_cmp(&order_of(b)));
    let at = board.partition_point(|t| order_of(t) <= order);
    let mut ids: Vec<Value> = board.iter().filter_map(|t| t.get("id").cloned()).collect();
    ids.insert(at, Value::from(id));
    if data::write(cx, "task:reorder", json!({ "ids": ids }))
        .await?
        .is_none()
    {
        return Ok(not_found(id));
    }
    let placed = data::find(cx, "tasks", "id", id).await?;
    Ok(pretty(&placed.unwrap_or(Value::Null)))
}

pub async fn delete_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = data::find(cx, "tasks", "id", id).await? else {
        return Ok(not_found(id));
    };
    if data::write(cx, "task:delete", json!({ "id": id }))
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
    if data::write(cx, "task:archive", params).await?.is_none() {
        return Ok(not_found(id));
    }
    let updated = data::find(cx, "tasks", "id", id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn archive_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = data::find(cx, "tasks", "id", id).await? else {
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
    if data::find(cx, "tasks", "id", id).await?.is_none() {
        return Ok(not_found(id));
    }
    set_archived(cx, id, false).await
}

pub async fn get_my_context<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    if let Some(task_id) = args.nonempty("task_id") {
        let Some(task) = data::find(cx, "tasks", "id", task_id).await? else {
            return Ok(not_found(task_id));
        };
        // dbGetProject compares with ===, and dbListTasks filters nothing out
        // for a task without a project name.
        let project = data::list(cx, "projects")
            .await?
            .into_iter()
            .find(|p| json::strict_equals(p.get("name"), task.get("projectName")));
        let siblings = match task.get("projectName") {
            None => data::tasks(cx, None, None).await?,
            Some(_) => data::list(cx, "tasks")
                .await?
                .into_iter()
                .filter(|t| json::strict_equals(t.get("projectName"), task.get("projectName")))
                .collect(),
        };
        let siblings: Vec<Value> = siblings
            .iter()
            .filter(|t| !json::strict_equals(t.get("id"), task.get("id")))
            .map(brief)
            .collect();
        let result = object([
            ("task", Some(task)),
            ("project", project.filter(|p| !p.is_null())),
            ("siblingTasks", Some(Value::Array(siblings))),
        ]);
        return Ok(pretty(&result));
    }

    let base = if cx.caller.cwd.is_empty() {
        "/"
    } else {
        &cx.caller.cwd
    };
    let cwd = match args.nonempty("cwd") {
        Some(cwd) => resolve(base, cwd),
        None => resolve(base, base),
    };
    let projects = data::list(cx, "projects").await?;
    let mut matched: Option<&Value> = None;
    let mut match_len = 0;
    for p in &projects {
        let path = resolve_value(base, p.get("path"))?;
        let len = json::utf16_len(&path);
        if cwd.starts_with(&path) && len > match_len {
            matched = Some(p);
            match_len = len;
        }
    }
    let Some(project) = matched else {
        return Ok(pretty(&json!({
            "message": "No matching project found for current directory.",
            "cwd": cwd,
            "hint": "Use list_projects to see available projects, or pass a task_id directly."
        })));
    };

    let project_tasks: Vec<Value> = data::list(cx, "tasks")
        .await?
        .into_iter()
        .filter(|t| json::strict_equals(t.get("projectName"), project.get("name")))
        .collect();

    let mut matched_task: Option<&Value> = None;
    for t in &project_tasks {
        if json::truthy(t.get("worktreePath")) {
            let worktree = resolve_value(base, t.get("worktreePath"))?;
            if cwd.starts_with(&worktree) {
                matched_task = Some(t);
                break;
            }
        }
    }
    if matched_task.is_none() {
        matched_task = project_tasks
            .iter()
            .find(|t| t.get("status").and_then(Value::as_str) == Some("in_progress"));
    }

    let mut result = Map::new();
    result.insert(
        "project".into(),
        object(["name", "path", "preferredAgents"].map(|k| (k, project.get(k).cloned()))),
    );
    result.insert("cwd".into(), Value::from(cwd));
    match matched_task {
        Some(task) => {
            result.insert("task".into(), task.clone());
            result.insert(
                "siblingTasks".into(),
                Value::Array(
                    project_tasks
                        .iter()
                        .filter(|t| !json::strict_equals(t.get("id"), task.get("id")))
                        .map(brief)
                        .collect(),
                ),
            );
        }
        None => {
            result.insert(
                "message".into(),
                "No specific task matched. Showing all project tasks.".into(),
            );
            result.insert(
                "tasks".into(),
                Value::Array(project_tasks.iter().map(brief).collect()),
            );
        }
    }
    Ok(pretty(&Value::Object(result)))
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn resolves_as_node_does_on_posix() {
        assert_eq!(resolve("/a/b", "/x/./y/../z/"), "/x/z");
        assert_eq!(resolve("/a/b", "c/../d"), "/a/b/d");
        assert_eq!(resolve("/a/b", ""), "/a/b");
        assert_eq!(resolve("/", "/.."), "/");
    }
}
