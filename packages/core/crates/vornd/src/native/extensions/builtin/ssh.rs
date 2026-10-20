//! SSH in vornd: the component Vorn ships (`packages/core/extensions/ssh`)
//! as an entry of the built-ins' registry, and the core's route to it. It
//! draws the SSH & Hosts settings page and answers `ssh:testConnection`,
//! under the same gate and lifecycle as an installed component, reading
//! and writing the hosts and keys with the core's `remote.*` methods. While
//! its switch is off each call is refused with [`OFF`].
//!
//! The hosts, the keys and the transport stay the core's (`vorn-remote`,
//! the configuration and the vault): a session already on a remote host
//! keeps running, and starts again, whether this is on or off.

use serde_json::Value;

use super::wasm::Component;
use crate::native::{Answer, Native};

/// The id SSH goes by.
pub const ID: &str = "vorn.ssh";

/// What a call to an `ssh:*` method is told while SSH is off.
pub const OFF: &str = "SSH is off. Turn it on in Settings → Extensions.";

/// Every `ssh:*` call it answers, each by the action of the same name.
pub const METHODS: &[&str] = &["ssh:testConnection"];

/// The component, built for wasm32-wasip2 by vornd's build script; nothing
/// runs until the settings page opens or a client tests a host.
pub fn component() -> Component {
    Component {
        manifest: include_str!("../../../../../../extensions/ssh/vorn-extension.toml"),
        events: &[],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/ssh.wasm")),
        on_by_default: true,
        start: None,
        stop: None,
    }
}

/// An `ssh:*` call, answered by the component's action of that name.
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    super::wasm::answer(native, ID, OFF, method, params).await
}
