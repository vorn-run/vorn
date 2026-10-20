//! The Device in vornd: the component Vorn ships
//! (`packages/core/extensions/device`) as an entry of the built-ins'
//! registry, and the core's route to it. An agent's `device:*` calls and its
//! MCP device tools reach the component through the router, under the same
//! gate and lifecycle as an installed component; it drives the machine's
//! simulators through the app with `video.call`. While its switch is off
//! each call is refused with [`OFF`].

use serde_json::Value;

use super::wasm::Component;
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

/// The component, built for wasm32-wasip2 by vornd's build script; nothing
/// runs until the first pane opens or an agent calls.
pub fn component() -> Component {
    Component {
        manifest: include_str!("../../../../../../extensions/device/vorn-extension.toml"),
        events: &[],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/device.wasm")),
        on_by_default: true,
        start: None,
    }
}

/// Answers one of its `device:*` calls; while it is off, [`OFF`].
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    super::wasm::answer(native, ID, OFF, method, params).await
}
