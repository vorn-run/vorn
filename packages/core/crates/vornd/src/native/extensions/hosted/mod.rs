//! The core's routes to extensions Vorn ships as ordinary installs: an
//! agent's `browser:*` and `device:*` calls, its MCP tools and a client's
//! `ssh:*` reach the installed component through the router, under the same
//! gate and lifecycle as any other. Uninstalled, each call says so.

pub mod browser;
pub mod device;
pub mod ssh;

use serde_json::Value;
use vorn_extensions::api::types::CallCtx;

use crate::native::{Answer, Native};

/// A core route's `method` (as `browser:openPane`), answered by installed
/// extension `id`'s action of the same name; uninstalled, it says how to get `name`.
pub async fn answer(
    native: &Native,
    id: &str,
    name: &str,
    method: &str,
    params: Value,
) -> Answer {
    let Some(ext) = native.extensions() else {
        return Answer::Error("vornd is not hosting extensions".to_owned());
    };
    if ext.router().program(id).is_none() {
        return Answer::Error(not_installed(name));
    }
    match ext.call(id, method, params, client()).await {
        Ok(v) => Answer::Result(v),
        Err(e) => Answer::Error(e.message),
    }
}

/// What a call is told while extension `name` is not installed.
pub fn not_installed(name: &str) -> String {
    format!("{name} is not installed. Install it in Settings → Extensions.")
}

fn client() -> CallCtx {
    CallCtx {
        caller: "client".to_owned(),
        project: None,
    }
}
