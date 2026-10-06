//! `tools/tasks.ts`: tasks, kept in the config blob.

use serde_json::{json, Map, Value};

use super::data::{self, Update};
use super::workspaces::js_max;
use super::{failed, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;
use crate::time::now_iso;

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
    let project_name = args.str("project_name").unwrap_or_default();
    if data::find(cx, "projects", "name", project_name)
        .await?
        .is_none()
    {
        return Ok(failed(format!(
            "Error: project \"{project_name}\" not found"
        )));
    }
    // dbGetMaxTaskOrder: `Math.max(max, t.order ?? 0)`.
    let max = data::tasks(cx, Some(project_name), None)
        .await?
        .iter()
        .fold(0.0, |max, t| {
            let order = match t.get("order") {
                None | Some(Value::Null) => 0.0,
                other => json::to_number(other),
            };
            js_max(max, order)
        });
    let now = now_iso();
    let status = args.str("status").unwrap_or("todo").to_owned();
    let done = status == "done" || status == "cancelled";
    let task = object([
        ("id", Some(Value::from(uuid::Uuid::new_v4().to_string()))),
        ("projectName", args.get("project_name").cloned()),
        ("title", args.get("title").cloned()),
        (
            "description",
            Some(
                args.get("description")
                    .cloned()
                    .unwrap_or_else(|| json!("")),
            ),
        ),
        ("status", Some(Value::from(status))),
        ("order", Some(json::num(max + 1.0))),
        ("createdAt", Some(Value::from(now.clone()))),
        ("updatedAt", Some(Value::from(now.clone()))),
        ("branch", args.nonempty("branch").map(Value::from)),
        (
            "useWorktree",
            args.truthy("use_worktree").then(|| Value::Bool(true)),
        ),
        (
            "assignedAgent",
            args.nonempty("assigned_agent").map(Value::from),
        ),
        ("completedAt", done.then(|| Value::from(now))),
    ]);
    data::insert(cx, "tasks", task.clone()).await?;
    Ok(pretty(&task))
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
    let Some(task) = data::find(cx, "tasks", "id", id).await? else {
        return Ok(not_found(id));
    };
    let mut updates: Update = vec![("updatedAt", Some(Value::from(now_iso())))];
    for (arg, key) in [
        ("title", "title"),
        ("description", "description"),
        ("branch", "branch"),
        ("use_worktree", "useWorktree"),
        ("assigned_agent", "assignedAgent"),
        ("order", "order"),
    ] {
        if let Some(value) = args.get(arg) {
            updates.push((key, Some(value.clone())));
        }
    }
    if let Some(status) = args.get("status") {
        let was_done = terminal(task.get("status"));
        let is_done = terminal(Some(status));
        updates.push(("status", Some(status.clone())));
        if is_done && !was_done {
            updates.push(("completedAt", Some(Value::from(now_iso()))));
        }
        if !is_done && was_done {
            updates.push(("completedAt", None));
            updates.push(("archivedAt", None));
        }
    }
    data::update(cx, "tasks", "id", id, &updates).await?;
    let updated = data::find(cx, "tasks", "id", id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn delete_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    let Some(task) = data::find(cx, "tasks", "id", id).await? else {
        return Ok(not_found(id));
    };
    data::delete(cx, "tasks", "id", id).await?;
    Ok(text(format!(
        "Deleted task: {}",
        json::display(task.get("title"))
    )))
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
    let now = Value::from(now_iso());
    let updates: Update = vec![("archivedAt", Some(now.clone())), ("updatedAt", Some(now))];
    data::update(cx, "tasks", "id", id, &updates).await?;
    let updated = data::find(cx, "tasks", "id", id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn unarchive_task<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    if data::find(cx, "tasks", "id", id).await?.is_none() {
        return Ok(not_found(id));
    }
    let updates: Update = vec![
        ("archivedAt", None),
        ("updatedAt", Some(Value::from(now_iso()))),
    ];
    data::update(cx, "tasks", "id", id, &updates).await?;
    let updated = data::find(cx, "tasks", "id", id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
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
