//! The prompts an agent step is started with (`prompt-builder.ts`): a
//! task's, with its project and the other tasks beside it, and the
//! workflow's wrapper around the step's own words.

use serde_json::Value;

use crate::js;
use crate::structured;

fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `${record.key}`: absent reads `undefined`, as a template literal does.
fn shown(record: &Value, key: &str) -> String {
    record
        .get(key)
        .map_or_else(|| "undefined".to_owned(), js::to_string)
}

fn status_name(status: &str) -> &str {
    match status {
        "todo" => "To Do",
        "in_progress" => "In Progress",
        "in_review" => "In Review",
        "done" => "Done",
        "cancelled" => "Cancelled",
        other => other,
    }
}

/// `buildTaskPrompt`: the task, its project and branch, what else is going
/// on in the project, and the tools the agent has.
pub fn task_prompt(task: &Value, project: &Value, siblings: &[Value]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut push = |s: String| lines.push(s);
    push(format!("# Task: {}", shown(task, "title")));
    push(String::new());
    push(format!("**Project:** {}", shown(project, "name")));
    push(format!("**Project Path:** {}", shown(project, "path")));
    let branch = text(task, "branch");
    if !branch.is_empty() {
        push(format!("**Branch:** {branch}"));
    }
    if crate::is_truthy(task.get("useWorktree")) {
        push("**Worktree:** Yes (isolated git worktree)".to_owned());
    }
    push(format!(
        "**Task Status:** {}",
        status_name(&shown(task, "status"))
    ));
    push(format!("**Task ID:** {}", shown(task, "id")));
    push(String::new());

    let description = text(task, "description").trim();
    if !description.is_empty() {
        push("## Description".to_owned());
        push(String::new());
        push(description.to_owned());
        push(String::new());
    }

    let id = task.get("id");
    let others: Vec<&Value> = siblings.iter().filter(|t| t.get("id") != id).collect();
    let with_status = |s: &str| -> Vec<&Value> {
        others
            .iter()
            .copied()
            .filter(|t| text(t, "status") == s)
            .collect()
    };
    let (in_progress, in_review, todo) = (
        with_status("in_progress"),
        with_status("in_review"),
        with_status("todo"),
    );
    if !(in_progress.is_empty() && in_review.is_empty() && todo.is_empty()) {
        push("## Other Tasks in This Project".to_owned());
        push(String::new());
        let with_branch = |t: &Value| {
            let b = text(t, "branch");
            if b.is_empty() {
                format!("- {}", shown(t, "title"))
            } else {
                format!("- {} (branch: {b})", shown(t, "title"))
            }
        };
        for (heading, list, shown_at_most, branches) in [
            ("In Progress", &in_progress, 5, true),
            ("In Review", &in_review, 5, true),
            ("Queued", &todo, 3, false),
        ] {
            if list.is_empty() {
                continue;
            }
            push(format!("**{heading} ({}):**", list.len()));
            for t in list.iter().take(shown_at_most) {
                push(if branches {
                    with_branch(t)
                } else {
                    format!("- {}", shown(t, "title"))
                });
            }
            if list.len() > shown_at_most {
                push(format!("- ... and {} more", list.len() - shown_at_most));
            }
            push(String::new());
        }
    }

    for line in [
        "## Available Tools",
        "",
        "You are managed by Vorn. You have access to MCP tools for project management:",
        "- `get_my_context` — Get your current task and project context",
        "- `list_tasks` — List tasks in this project",
        "- `update_task` — Update task status/description when done",
        "- `get_diff` — See current git changes",
        "- `list_branches` — List git branches",
        "",
        "When you complete this task, update its status to \"in_review\" or \"done\" using `update_task`.",
        "",
    ] {
        push(line.to_owned());
    }
    lines.join("\n")
}

/// `buildWorkflowPrompt`: the step's prompt inside the workflow it belongs
/// to, with the output instructions when it declares a schema.
pub fn workflow_prompt(
    workflow_id: &str,
    workflow_name: &str,
    step: &str,
    prompt: &str,
    schema: Option<&Value>,
) -> String {
    let mut lines = vec![
        format!("# Workflow: {workflow_name}"),
        String::new(),
        format!("**Step:** {step}"),
        format!("**Workflow ID:** {workflow_id}"),
        String::new(),
        "## Task".to_owned(),
        String::new(),
        prompt.to_owned(),
        String::new(),
        "## Available Tools".to_owned(),
        String::new(),
        "You are managed by Vorn. You have access to MCP tools:".to_owned(),
        "- `get_my_context` — Get your current project context".to_owned(),
        format!(
            "- `list_workflow_runs` — See previous runs for this workflow (workflow_id: \"{workflow_id}\")"
        ),
        "- `list_tasks` — List tasks in this project".to_owned(),
        "- `update_task` — Update task status when done".to_owned(),
        String::new(),
    ];
    if let Some(schema) = schema.filter(|s| !s.is_null()) {
        lines.push(structured::instructions(schema));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_task_prompt_names_its_project_and_neighbours() {
        let task = json!({ "id": "t1", "title": "Fix", "status": "todo", "description": "  Do it.  ", "branch": "fix", "useWorktree": true });
        let project = json!({ "name": "app", "path": "/app" });
        let siblings: Vec<Value> = (0..7)
            .map(|i| json!({ "id": format!("s{i}"), "title": format!("S{i}"), "status": "in_progress", "branch": if i == 0 { "b" } else { "" } }))
            .chain([task.clone(), json!({ "id": "q", "title": "Q", "status": "todo" })])
            .collect();
        let prompt = task_prompt(&task, &project, &siblings);
        assert!(prompt.starts_with("# Task: Fix\n\n**Project:** app\n**Project Path:** /app\n**Branch:** fix\n**Worktree:** Yes (isolated git worktree)\n**Task Status:** To Do\n**Task ID:** t1\n\n## Description\n\nDo it.\n\n"));
        assert!(prompt.contains("**In Progress (7):**\n- S0 (branch: b)\n- S1\n"));
        assert!(prompt.contains("- ... and 2 more\n\n**Queued (1):**\n- Q\n\n## Available Tools"));
        assert!(prompt.ends_with("using `update_task`.\n"));
    }

    #[test]
    fn a_workflow_prompt_wraps_the_step() {
        let p = workflow_prompt("w1", "Nightly", "Review", "Look", None);
        assert!(p.starts_with(
            "# Workflow: Nightly\n\n**Step:** Review\n**Workflow ID:** w1\n\n## Task\n\nLook\n\n"
        ));
        assert!(p.ends_with("- `update_task` — Update task status when done\n"));
        let typed = workflow_prompt("w1", "N", "S", "P", Some(&json!({ "type": "object" })));
        assert!(typed.ends_with("```\n"));
        assert!(typed.contains("\n\n## Required Output\n"));
    }
}
