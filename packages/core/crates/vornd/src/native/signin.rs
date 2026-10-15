//! Sign-in windows, a service of the extension bridge: a program granted a
//! window ([`vorn_extensions::signin`]) calls through it at
//! `/connections/<key>/browser/fetch`, and the desktop that holds the window
//! makes the request. Whether a window is still signed in, forgetting one,
//! and waking the workflow steps that waited for one to sign in again are
//! here too, for whichever extension holds the window.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::Native;
use crate::bridge::Bridge;

/// One call through a signed-in window.
const WINDOW_CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// `POST /connections/<key>/browser/fetch`: a program calling through its
/// window. The status and body to answer with.
pub async fn window_fetch(
    native: &Arc<Native>,
    key: &str,
    bearer: Option<&str>,
    call: Option<&str>,
    body: &[u8],
) -> (u16, Value) {
    let admitted = match native.windows().admit(key, bearer, body) {
        Ok(a) => a,
        Err(refused) => return refused,
    };
    let record = |status: Value| {
        admitted.grant.record(
            call,
            json!({ "method": admitted.method, "path": admitted.path, "status": status }),
        );
    };
    let desktop = native.main_process();
    if !desktop.connected() {
        record(json!("app-offline"));
        return (
            503,
            json!({ "error": "Open Vorn on the desktop this connection signed in on" }),
        );
    }
    let asked = Bridge::request(
        desktop.as_ref(),
        "session:fetch",
        json!({ "connectionId": key, "origins": admitted.grant.origins(), "request": admitted.request }),
        WINDOW_CALL_TIMEOUT,
    )
    .await;
    match asked {
        Ok(answer) => {
            record(answer.get("status").cloned().unwrap_or(Value::Null));
            (200, answer)
        }
        Err(error) => {
            record(json!("failed"));
            (503, json!({ "error": error }))
        }
    }
}

/// `signin.check`: whether window `key` is still signed in, as the desktop
/// sees it with the sign-in's own check; `{signedIn: null}` with no desktop.
pub async fn check(native: &Arc<Native>, key: &str, browser: Value) -> Result<Value, String> {
    let desktop = native.main_process();
    if !desktop.connected() {
        return Ok(json!({ "signedIn": null }));
    }
    Bridge::request(
        desktop.as_ref(),
        "session:check",
        json!({ "connectionId": key, "browser": browser }),
        WINDOW_CALL_TIMEOUT,
    )
    .await
}

/// `signin.forget`: the desktop forgets window `key`'s cookies and storage.
pub async fn forget(native: &Arc<Native>, key: &str) -> Result<Value, String> {
    native.windows().revoke(key);
    Bridge::request(
        native.main_process().as_ref(),
        "session:forget",
        json!(key),
        WINDOW_CALL_TIMEOUT,
    )
    .await
}

/// `signin.completed`: window `key` signed in again, so the steps waiting
/// for it run again.
pub async fn completed(native: &Arc<Native>, key: &str) {
    if let Some(work) = native.work().cloned() {
        work.signed_in(key).await;
    }
}
