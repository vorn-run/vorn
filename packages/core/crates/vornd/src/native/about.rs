//! What vornd says of itself and of this machine: the core that answers
//! (`core:status`) and whether a remote host can be logged in to, which
//! the SSH built-in asks with `remote.test`.

use std::sync::Arc;

use serde_json::{json, Value};

use super::{Answer, Native};

/// Every call this module answers.
pub const METHODS: &[&str] = &["core:status"];

/// Answers `method`.
pub async fn answer(_native: &Arc<Native>, method: &str, _params: Value) -> Answer {
    match method {
        // vornd is the core now: it is loaded whenever it answers, with everything built in.
        "core:status" => Answer::Result(json!({
            "loaded": true,
            "version": env!("CARGO_PKG_VERSION"),
            "error": null,
            "missing": [],
        })),
        _ => Answer::Unanswered,
    }
}

/// Logs in to the remote host `host` describes (`vorn_remote::Host`'s
/// fields) and says how it went, `{success, message, durationMs}`, within
/// the test's own ten seconds. A `host` that is not one is the error.
pub(crate) async fn test_host(native: &Native, host: Value) -> Result<Value, String> {
    let host = serde_json::from_value::<vorn_remote::Host>(host).map_err(|e| e.to_string())?;
    let env = native.env.get();
    let tested = tokio::task::spawn_blocking(move || vorn_remote::test(&host, &env))
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(tested).map_err(|e| e.to_string())
}
