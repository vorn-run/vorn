//! The task board, the projects and the session-event log: `task:*`,
//! `project:*` and `sessionEvent:*`, read and written in `vorn.db`. A write
//! that changes the board tells every client the configuration changed, as
//! the board reads it from there.

use std::sync::Arc;

use serde_json::{json, Value};
use vorn_store::Store;
use vorn_work::task_images::TaskImages;
use vorn_work::tasks;

use super::config::{announce, blocking, with_store};
use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &[
    "task:list",
    "task:get",
    "task:setStatus",
    "task:create",
    "task:update",
    "task:delete",
    "task:reorder",
    "task:archive",
    "task:imageSave",
    "task:imageDelete",
    "task:imageGetPath",
    "task:imageCleanup",
    "task:imageUpload",
    "project:list",
    "project:detectMobile",
    "sessionEvent:list",
    "sessionEvent:listBySession",
];

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn text(params: &Value, key: &str) -> String {
    match params.get(key) {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => vorn_work::js::to_string(other),
    }
}

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    if native.database().is_none() {
        // Only a test starts vornd without one.
        return Answer::Unanswered;
    }
    if method.starts_with("task:image") {
        return images(native, method, &params);
    }
    if method == "project:detectMobile" {
        let path = text(&params, "projectPath");
        return match tokio::task::spawn_blocking(move || vorn_worktrees::mobile::detect(&path))
            .await
        {
            Ok(found) => Answer::Result(found),
            Err(err) => Answer::Error(err.to_string()),
        };
    }
    let n = Arc::clone(native);
    let m = method.to_owned();
    let done = blocking(method, move || {
        with_store(&n, |store| call(store, &m, &params))
    })
    .await;
    match done {
        Ok((value, changed)) => {
            if changed {
                announce(native).await;
            }
            match value {
                Some(value) => Answer::Result(value),
                None => Answer::Void,
            }
        }
        Err(message) => Answer::Error(message),
    }
}

/// The store's part of a call: its answer, and whether the board changed.
fn call(
    store: &mut Store,
    method: &str,
    params: &Value,
) -> vorn_store::Result<(Option<Value>, bool)> {
    let now = now_iso();
    let answered = |(v, changed): (Value, bool)| (Some(v), changed);
    Ok(match method {
        "task:list" => (Some(tasks::list(store, params)?), false),
        "task:get" => (Some(tasks::get(store, params)?), false),
        "task:setStatus" => answered(tasks::set_status(store, params, &now)?),
        "task:create" => answered(tasks::create(
            store,
            params,
            uuid::Uuid::new_v4().to_string(),
            &now,
        )?),
        "task:update" => answered(tasks::update(store, params, &now)?),
        "task:delete" => answered(tasks::delete(store, params)?),
        "task:reorder" => answered(tasks::reorder(store, params, &now)?),
        "task:archive" => answered(tasks::archive(store, params, &now)?),
        "project:list" => {
            let config = store.load_config()?;
            (
                Some(config.get("projects").cloned().unwrap_or_else(|| json!([]))),
                false,
            )
        }
        "sessionEvent:list" => {
            let limit = params
                .get("limit")
                .filter(|l| !l.is_null())
                .cloned()
                .unwrap_or(json!(100));
            let kind = params.get("eventType").cloned().unwrap_or(Value::Null);
            (
                Some(store.call("listSessionEvents", json!([kind, limit]))?),
                false,
            )
        }
        "sessionEvent:listBySession" => {
            let limit = params
                .get("limit")
                .filter(|l| !l.is_null())
                .cloned()
                .unwrap_or(json!(100));
            let id = params.get("sessionId").cloned().unwrap_or(Value::Null);
            (
                Some(store.call("listSessionEventsBySession", json!([id, limit]))?),
                false,
            )
        }
        _ => (None, false),
    })
}

fn images(native: &Native, method: &str, params: &Value) -> Answer {
    let Some(dir) = native.database().and_then(std::path::Path::parent) else {
        return super::no_database();
    };
    let images = TaskImages::new(dir);
    let task = text(params, "taskId");
    let done = match method {
        "task:imageSave" => images
            .save(&task, &text(params, "sourcePath"))
            .map(|n| json!(n)),
        "task:imageUpload" => images
            .upload(&task, &text(params, "base64"), &text(params, "filename"))
            .map(|n| json!(n)),
        "task:imageDelete" => images
            .delete(&task, &text(params, "filename"))
            .map(|()| Value::Null),
        "task:imageGetPath" => images
            .path(&task, &text(params, "filename"))
            .map(|p| json!(p)),
        "task:imageCleanup" => {
            let task = params
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| vorn_work::js::to_string(params));
            images.cleanup(&task).map(|()| Value::Null)
        }
        _ => return Answer::Unanswered,
    };
    match done {
        Ok(Value::Null) => Answer::Void,
        Ok(value) => Answer::Result(value),
        Err(message) => Answer::Error(message),
    }
}

/// Logs a session's lifecycle event (`logSessionEvent`): best effort, as the server's was.
pub fn log_event(native: &Native, session: &str, kind: &str, metadata: Option<Value>) {
    let Some(db) = native.database().map(std::path::Path::to_path_buf) else {
        return;
    };
    let mut event = json!({ "sessionId": session, "eventType": kind, "timestamp": now_iso() });
    if let Some(meta) = metadata {
        event["metadata"] = meta;
    }
    std::thread::spawn(move || {
        let Ok(Some(mut store)) = Store::open_beside(&db) else {
            return;
        };
        if let Err(err) = store.call("insertSessionEvent", json!([event])) {
            tracing::warn!(%err, "could not log a session event");
        }
    });
}
