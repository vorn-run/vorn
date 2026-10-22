//! SSH's route in vornd. The installed SSH extension draws the SSH & Hosts
//! settings page and answers `ssh:testConnection`, reading and writing the
//! hosts and keys with the core's `remote.*` methods.
//!
//! The hosts, the keys and the transport stay the core's (`vorn-remote`,
//! the configuration and the vault): a session already on a remote host
//! keeps running, and starts again, whether SSH is installed or not.

use serde_json::Value;

use crate::native::{Answer, Native};

/// The id SSH goes by.
pub const ID: &str = "vorn.ssh";

/// Every `ssh:*` call it answers, each by the action of the same name.
pub const METHODS: &[&str] = &["ssh:testConnection"];

/// An `ssh:*` call, answered by the extension's action of that name.
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    super::answer(native, ID, "SSH", method, params).await
}
