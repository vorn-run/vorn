//! `tools/workspaces.ts`: workspaces, kept in the config blob.

use serde_json::Value;

use super::data::{self, Update};
use super::{failed, object, pretty, text, Args, Cx, Outcome};
use crate::json;
use crate::rpc::Rpc;

/// `Math.max(a, b)`: NaN wins.
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

pub async fn list_workspaces<R: Rpc>(cx: &Cx<'_, R>) -> Outcome {
    Ok(pretty(&Value::Array(data::list(cx, "workspaces").await?)))
}

pub async fn create_workspace<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let existing = data::list(cx, "workspaces").await?;
    let max = existing
        .iter()
        .fold(0.0, |max, w| js_max(max, json::to_number(w.get("order"))));
    let workspace = object([
        ("id", Some(Value::from(uuid::Uuid::new_v4().to_string()))),
        ("name", args.get("name").cloned()),
        ("order", Some(json::num(max + 1.0))),
        ("icon", args.nonempty("icon").map(Value::from)),
        ("iconColor", args.nonempty("icon_color").map(Value::from)),
    ]);
    data::insert(cx, "workspaces", workspace.clone()).await?;
    Ok(pretty(&workspace))
}

pub async fn update_workspace<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    if data::find(cx, "workspaces", "id", id).await?.is_none() {
        return Ok(failed(format!("Error: workspace \"{id}\" not found")));
    }
    let mut updates: Update = Vec::new();
    for (arg, key) in [
        ("name", "name"),
        ("icon", "icon"),
        ("icon_color", "iconColor"),
        ("order", "order"),
    ] {
        if let Some(value) = args.get(arg) {
            updates.push((key, Some(value.clone())));
        }
    }
    data::update(cx, "workspaces", "id", id, &updates).await?;
    let updated = data::find(cx, "workspaces", "id", id).await?;
    Ok(pretty(&updated.unwrap_or(Value::Null)))
}

pub async fn delete_workspace<R: Rpc>(cx: &Cx<'_, R>, args: &Args) -> Outcome {
    let id = args.str("id").unwrap_or_default();
    if id == "personal" {
        return Ok(failed("Error: cannot delete the default workspace"));
    }
    let Some(workspace) = data::find(cx, "workspaces", "id", id).await? else {
        return Ok(failed(format!("Error: workspace \"{id}\" not found")));
    };
    data::delete(cx, "workspaces", "id", id).await?;
    Ok(text(format!(
        "Deleted workspace: {}",
        json::display(workspace.get("name"))
    )))
}
