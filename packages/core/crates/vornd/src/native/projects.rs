//! The projects (`project:*`): the list the configuration keeps, and whether
//! one is a mobile app, which the device pane asks before it offers a simulator.

use std::sync::Arc;

use serde_json::{json, Value};

use super::config::{blocking, with_store};
use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["project:list", "project:detectMobile"];

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    if native.database().is_none() {
        // Only a test starts vornd without one.
        return Answer::Unanswered;
    }
    match method {
        "project:list" => {
            let n = Arc::clone(native);
            let listed = blocking(method, move || {
                with_store(&n, |store| {
                    let config = store.load_config()?;
                    Ok(config.get("projects").cloned().unwrap_or_else(|| json!([])))
                })
            });
            match listed.await {
                Ok(projects) => Answer::Result(projects),
                Err(message) => Answer::Error(message),
            }
        }
        "project:detectMobile" => {
            let path = match params.get("projectPath") {
                Some(Value::String(s)) => s.clone(),
                None | Some(Value::Null) => String::new(),
                Some(other) => vorn_work::js::to_string(other),
            };
            match tokio::task::spawn_blocking(move || vorn_worktrees::mobile::detect(&path)).await {
                Ok(found) => Answer::Result(found),
                Err(err) => Answer::Error(err.to_string()),
            }
        }
        _ => Answer::Unanswered,
    }
}
