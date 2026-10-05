//! Vorn's native core, as the Node server loads it (`vorn_core.node`).
//!
//! Only adapters live here: `gitRun` over `vorn-git` and `NativeStore` over
//! `vorn-store`, plus the small exports the server uses to check the binary
//! loaded. Terminals do not pass through this library; they run in vornd,
//! which owns each session's screen and output analysis. The logic is in
//! plain crates under `crates/` with no napi in them, so its tests and
//! benchmarks link as ordinary Rust.

use napi_derive::napi;

pub mod git;
pub mod store;

// Every exported function uses `#[napi(catch_unwind)]`: napi-rs only turns a
// panic into a JS exception when asked to, and an uncaught one unwinding into
// Node aborts the server.

/// What the loaded binary was built from, so the server can log it.
#[napi(object)]
pub struct CoreInfo {
    /// The crate version, kept in step with the app version.
    pub version: String,
}

#[napi(catch_unwind)]
pub fn info() -> CoreInfo {
    CoreInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

#[napi(catch_unwind)]
pub fn hello(name: String) -> String {
    format!("hello {name} from vorn-core {}", env!("CARGO_PKG_VERSION"))
}

/// Takes a chunk and does nothing with it: what one napi crossing with a string
/// argument costs, for the bench to set beside the real calls.
#[napi(catch_unwind)]
pub fn noop(data: String) -> u32 {
    data.len() as u32
}
