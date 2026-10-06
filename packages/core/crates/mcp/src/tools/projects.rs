//! `tools/projects.ts`: projects, kept in the config blob.

use serde_json::Value;

use super::data::{self, Update};
use super::{failed, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// `(p.workspaceId ?? 'personal') === id`: a row with no workspace is in the default one.
pub(crate) fn in_workspace(row: &Value, workspace: &str) -> bool {
    match row.get("workspaceId") {
        None | Some(Value::Null) => workspace == "personal",
        Some(value) => value.as_str() == Some(workspace),
    }
}

pub async fn list_projects<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let mut projects = data::list(cx, "projects").await?;
    if let Some(workspace) = args.nonempty("workspace_id") {
        projects.retain(|p| in_workspace(p, workspace));
    }
    Ok(pretty(&Value::Array(projects)))
}

pub async fn create_project<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let name = args.str("name").unwrap_or_default();
    if data::find(cx, "projects", "name", name).await?.is_some() {
        return Ok(failed(format!("Error: project \"{name}\" already exists")));
    }
    let project = object([
        ("name", args.get("name").cloned()),
        ("path", args.get("path").cloned()),
        (
            "preferredAgents",
            Some(
                args.get("preferred_agents")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            ),
        ),
        ("icon", args.nonempty("icon").map(Value::from)),
        ("iconColor", args.nonempty("icon_color").map(Value::from)),
    ]);
    data::insert(cx, "projects", project.clone()).await?;
    Ok(pretty(&project))
}

pub async fn update_project<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let name = args.str("name").unwrap_or_default();
    if data::find(cx, "projects", "name", name).await?.is_none() {
        return Ok(failed(format!("Error: project \"{name}\" not found")));
    }
    let mut updates: Update = Vec::new();
    for (arg, key) in [
        ("path", "path"),
        ("preferred_agents", "preferredAgents"),
        ("icon", "icon"),
        ("icon_color", "iconColor"),
    ] {
        if let Some(value) = args.get(arg) {
            updates.push((key, Some(value.clone())));
        }
    }
    data::update(cx, "projects", "name", name, &updates).await?;
    let updated = data::find(cx, "projects", "name", name).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn delete_project<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let name = args.str("name").unwrap_or_default();
    if data::find(cx, "projects", "name", name).await?.is_none() {
        return Ok(failed(format!("Error: project \"{name}\" not found")));
    }
    data::delete(cx, "projects", "name", name).await?;
    Ok(text(format!(
        "Deleted project: {}",
        json::display(args.get("name"))
    )))
}
