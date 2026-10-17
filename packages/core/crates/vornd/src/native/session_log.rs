//! The session-event log (`sessionEvent:*`): each terminal's and headless
//! agent's creation, renames and end, kept in `vorn.db` beside the sessions.

use std::sync::Arc;

use serde_json::{json, Value};
use vorn_store::Store;

use super::config::{blocking, with_store};
use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["sessionEvent:list", "sessionEvent:listBySession"];

/// `limit`, a hundred when it is not given.
fn limit(params: &Value) -> Value {
    params
        .get("limit")
        .filter(|l| !l.is_null())
        .cloned()
        .unwrap_or(json!(100))
}

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    if native.database().is_none() {
        // Only a test starts vornd without one.
        return Answer::Unanswered;
    }
    let (call, args) = match method {
        "sessionEvent:list" => {
            let kind = params.get("eventType").cloned().unwrap_or(Value::Null);
            ("listSessionEvents", json!([kind, limit(&params)]))
        }
        "sessionEvent:listBySession" => {
            let id = params.get("sessionId").cloned().unwrap_or(Value::Null);
            ("listSessionEventsBySession", json!([id, limit(&params)]))
        }
        _ => return Answer::Unanswered,
    };
    let n = Arc::clone(native);
    match blocking(method, move || {
        with_store(&n, |store| store.call(call, args))
    })
    .await
    {
        Ok(events) => Answer::Result(events),
        Err(message) => Answer::Error(message),
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Logs a session's lifecycle event, best effort.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lists_what_was_logged_by_kind_and_by_session() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vorn.db");
        drop(super::super::work::test_store(&db));
        let native = Native::new();
        native.set_database(db);
        log_event(
            &native,
            "s1",
            "created",
            Some(json!({ "agentType": "shell" })),
        );
        log_event(&native, "s2", "renamed", None);
        let by_session = json!({ "sessionId": "s1" });
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let listed = loop {
            let got = answer(&native, "sessionEvent:listBySession", by_session.clone()).await;
            if let Answer::Result(Value::Array(events)) = &got {
                if !events.is_empty() {
                    break events.clone();
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "never logged: {got:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(listed[0]["eventType"], "created");
        assert_eq!(listed[0]["metadata"]["agentType"], "shell");
        let renamed = answer(
            &native,
            "sessionEvent:list",
            json!({ "eventType": "renamed", "limit": 5 }),
        )
        .await;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut renamed = renamed;
        while !matches!(&renamed, Answer::Result(Value::Array(e)) if e.len() == 1) {
            assert!(std::time::Instant::now() < deadline, "{renamed:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
            renamed = answer(
                &native,
                "sessionEvent:list",
                json!({ "eventType": "renamed" }),
            )
            .await;
        }
        assert_eq!(
            super::super::projects::answer(&native, "project:list", Value::Null).await,
            Answer::Result(json!([]))
        );
    }
}
