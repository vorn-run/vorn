//! The Device in vornd: the component Vorn ships
//! (`packages/core/extensions/device`) as an entry of the built-ins'
//! registry, and the core's route to it. An agent's `device:*` calls and its
//! MCP device tools reach the component through the router, under the same
//! gate and lifecycle as an installed component; it drives the machine's
//! simulators through the app with `video.call`. While its switch is off
//! each call is refused with [`OFF`].
//!
//! A simulator an agent claims while it is down is started here, not in
//! the component: the component answers one call at a time, and a cold
//! boot takes tens of seconds, which would hold up every other session's
//! device calls and the panes. The component answers `{booting}` and holds
//! the simulator; vornd asks the app to start it and tells the component
//! with its [`BOOTED`] action, whose answer is the agent's.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::wasm::Component;
use crate::bridge::Bridge;
use crate::native::{Answer, Native};

/// The id the Device goes by.
pub const ID: &str = "vorn.device";

/// What a call to a device method is told while the Device is off.
pub const OFF: &str = "Device is off. Turn it on in Settings → Extensions.";

/// Every `device:*` call it answers, each by the action of the same name.
pub const METHODS: &[&str] = &[
    "device:list",
    "device:claim",
    "device:release",
    "device:openPane",
    "device:launch",
    "device:terminate",
    "device:openUrl",
    "device:install",
    "device:screenshot",
    "device:chrome",
    "device:readScreen",
    "device:find",
    "device:interact",
    "device:logs",
];

/// The component, built for wasm32-wasip2 by vornd's build script. It is
/// started in the background as vornd starts, which checks the claims kept
/// from before and keeps the first pane from waiting. A session that ends
/// lets its simulator go, and switched off it lets every one go.
pub fn component() -> Component {
    Component {
        manifest: include_str!("../../../../../../extensions/device/vorn-extension.toml"),
        events: &[vorn_extensions::events::SESSION_EXITED],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/device.wasm")),
        on_by_default: true,
        start: Some("start"),
        stop: Some("stop"),
    }
}

/// The component's action that hears how starting an agent's claim went.
const BOOTED: &str = "booted";
/// How long a simulator an agent claimed may take to start: a first boot
/// after a new runtime is installed takes minutes.
const BOOT_LIMIT: Duration = Duration::from_secs(180);
/// How long the agent's call waits for it, under the router's minute.
/// Past that it is told the simulator is still starting; the claim still
/// lands once it is up, as it did when the call itself timed out.
const ANSWER_WITHIN: Duration = Duration::from_secs(55);

/// Answers one of its `device:*` calls; while it is off, [`OFF`].
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    let first = super::wasm::answer(native, ID, OFF, method, params.clone()).await;
    let booting = match &first {
        Answer::Result(v) => v.get("booting").and_then(Value::as_str),
        _ => None,
    };
    let (Some(udid), Some(ext)) = (booting.map(str::to_owned), native.extensions().cloned()) else {
        return first;
    };
    let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
    let action = method.strip_prefix("device:").unwrap_or(method).to_owned();
    let desktop = Arc::clone(native.main_process());
    let still = format!("{udid} is still starting; it is this session's once it is up.");
    let late = if action == "openPane" {
        Answer::Error(still.clone())
    } else {
        Answer::Result(json!({ "ok": false, "reason": "booting", "message": still }))
    };
    // A task of its own, so the claim lands even once the agent stops waiting.
    let starting = tokio::spawn(async move {
        let asked = json!({ "udid": udid, "view": session });
        let started = Bridge::request(desktop.as_ref(), "video:boot", asked, BOOT_LIMIT).await;
        let mut told = json!({ "session": session, "udid": udid, "method": action });
        match started {
            Ok(v) => told["bootedHere"] = v["bootedHere"].clone(),
            Err(e) => told["error"] = json!(e),
        }
        match ext.builtins().call(ID, BOOTED, told).await {
            Some(Ok(v)) => Answer::Result(v),
            Some(Err(e)) => Answer::Error(e.message),
            None => Answer::Unanswered,
        }
    });
    match tokio::time::timeout(ANSWER_WITHIN, starting).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(e)) => Answer::Error(format!("{method} failed in vornd: {e}")),
        Err(_) => late,
    }
}
