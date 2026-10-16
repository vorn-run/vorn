//! `get_my_context`: the project and task an agent works
//! in, read from the core's task list. The other task tools are the Tasks
//! built-in's, which adds them while it is on.

use serde_json::{json, Map, Value};

use super::data;
use super::{failed, object, pretty, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

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

/// `path.resolve(value)` for a value read out of the config, refused unless
/// it is a string.
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

pub async fn get_my_context<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    if let Some(task_id) = args.nonempty("task_id") {
        let Some(task) = data::task(cx, task_id).await? else {
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
            Some(Value::String(p)) => data::tasks(cx, Some(p), None).await?,
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

    let project_tasks: Vec<Value> = match project.get("name") {
        Some(Value::String(name)) => data::tasks(cx, Some(name), None).await?,
        name => data::list(cx, "tasks")
            .await?
            .into_iter()
            .filter(|t| json::strict_equals(t.get("projectName"), name))
            .collect(),
    };

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
