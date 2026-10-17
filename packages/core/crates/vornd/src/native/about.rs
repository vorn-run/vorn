//! What vornd says of itself and of this machine: the core that answers
//! (`core:status`) and whether a remote host can be logged in to
//! (`ssh:testConnection`).

use std::sync::Arc;

use serde_json::{json, Value};

use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["core:status", "ssh:testConnection"];

/// Answers `method`.
pub async fn answer(native: &Arc<Native>, method: &str, params: Value) -> Answer {
    match method {
        // vornd is the core now: it is loaded whenever it answers, with everything built in.
        "core:status" => Answer::Result(json!({
            "loaded": true,
            "version": env!("CARGO_PKG_VERSION"),
            "error": null,
            "missing": [],
        })),
        "ssh:testConnection" => {
            let host = match serde_json::from_value::<vorn_remote::Host>(params) {
                Ok(host) => host,
                Err(err) => return Answer::Error(err.to_string()),
            };
            let env = native.env.get();
            match tokio::task::spawn_blocking(move || vorn_remote::test(&host, &env)).await {
                Ok(tested) => Answer::Result(serde_json::to_value(tested).unwrap_or(Value::Null)),
                Err(err) => Answer::Error(err.to_string()),
            }
        }
        _ => Answer::Unanswered,
    }
}
