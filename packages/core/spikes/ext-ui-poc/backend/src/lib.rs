//! Worktree cleanup's backend, shared by both models: inventory to keyed rows, and the removal plan.

use serde_json::{json, Value};

wit_bindgen::generate!({ world: "backend", path: "../wit" });

struct Backend;

thread_local! {
    static PROJECT_OF: std::cell::RefCell<std::collections::HashMap<String, String>> = Default::default();
}

fn status(e: &Value) -> &'static str {
    let flag = |k: &str| e.get(k).and_then(Value::as_bool).unwrap_or(false);
    let active = e["activeSessionIds"]
        .as_array()
        .is_some_and(|a| !a.is_empty());
    match () {
        _ if flag("isMain") => "main",
        _ if active => "active",
        _ if flag("isDirty") => "dirty",
        _ if flag("isMerged") => "merged",
        _ if e["idleDays"].as_u64().unwrap_or(0) >= 14 => "stale",
        _ => "open",
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

impl Guest for Backend {
    fn view() -> String {
        include_str!("../view.json").to_owned()
    }

    fn rows(inventory: String) -> String {
        let inv: Value = serde_json::from_str(&inventory).unwrap_or(Value::Null);
        let mut rows = Vec::new();
        for p in inv["projects"].as_array().into_iter().flatten() {
            for e in p["entries"].as_array().into_iter().flatten() {
                let status = status(e);
                rows.push(json!({
                    "id": e["path"],
                    "project": e["projectPath"],
                    "name": e["name"],
                    "branch": e["branch"],
                    "status": status,
                    "size": size(e["sizeBytes"].as_u64().unwrap_or(0)),
                    "bytes": e["sizeBytes"].as_u64().unwrap_or(0),
                    "removable": matches!(status, "merged" | "stale" | "open"),
                }));
            }
        }
        PROJECT_OF.with(|m| {
            let mut m = m.borrow_mut();
            m.clear();
            for r in &rows {
                m.insert(
                    r["id"].as_str().unwrap_or("").to_owned(),
                    r["project"].as_str().unwrap_or("").to_owned(),
                );
            }
        });
        Value::Array(rows).to_string()
    }

    fn remove_plan(ids: Vec<String>) -> Result<String, String> {
        if ids.is_empty() {
            return Err("nothing chosen".into());
        }
        let items = PROJECT_OF.with(|m| {
            let m = m.borrow();
            ids.iter()
                .map(|id| {
                    let project = m.get(id).ok_or_else(|| format!("unknown worktree {id}"))?;
                    Ok(json!({ "worktreePath": id, "projectPath": project, "force": false, "deleteBranch": false }))
                })
                .collect::<Result<Vec<Value>, String>>()
        })?;
        Ok(json!({ "items": items }).to_string())
    }
}

export!(Backend);
