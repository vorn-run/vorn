//! Vorn's native core, as the Node server loads it (`vorn_core.node`).
//!
//! Only adapters live here. The logic is in plain crates under `crates/`
//! (`vorn-screen`, `vorn-analysis`, `vorn-pipeline`) with no napi in them, so the same code can
//! later serve a daemon or the native UI, and its tests and benchmarks link as
//! ordinary Rust.

use napi_derive::napi;

pub mod analysis;
#[cfg(feature = "ghostty")]
pub mod pipeline;
#[cfg(feature = "ghostty")]
pub mod screen;

// Every exported function uses `#[napi(catch_unwind)]`: napi-rs only turns a
// panic into a JS exception when asked to, and an uncaught one unwinding into
// Node aborts the server and every terminal it hosts.

/// What the loaded binary was built from, so the server can log it.
#[napi(object)]
pub struct CoreInfo {
    /// The crate version, kept in step with the app version.
    pub version: String,
    /// libghostty-vt's own version, or `None` when built without it.
    pub ghostty: Option<String>,
}

#[napi(catch_unwind)]
pub fn info() -> CoreInfo {
    CoreInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        ghostty: ghostty_version(),
    }
}

#[napi(catch_unwind)]
pub fn hello(name: String) -> String {
    format!("hello {name} from vorn-core {}", env!("CARGO_PKG_VERSION"))
}

/// Feeds bytes through a throwaway terminal and returns the title they set.
///
/// A smoke test for the libghostty-vt link: an OSC 2 sequence in, the title out.
#[cfg(feature = "ghostty")]
#[napi(catch_unwind)]
pub fn parse_title(bytes: napi::bindgen_prelude::Buffer) -> napi::Result<String> {
    use libghostty_vt::terminal::{Options, Terminal};

    let mut terminal = Terminal::new(Options {
        cols: 80,
        rows: 24,
        max_scrollback: 0,
    })
    .map_err(to_napi)?;
    terminal.vt_write(&bytes);
    terminal.title().map(str::to_owned).map_err(to_napi)
}

#[cfg(feature = "ghostty")]
fn to_napi(err: libghostty_vt::Error) -> napi::Error {
    napi::Error::from_reason(format!("libghostty-vt: {err:?}"))
}

#[cfg(feature = "ghostty")]
fn ghostty_version() -> Option<String> {
    libghostty_vt::build_info::version_string()
        .ok()
        .map(str::to_owned)
}

#[cfg(not(feature = "ghostty"))]
fn ghostty_version() -> Option<String> {
    None
}

/// Takes a chunk and does nothing with it: what one napi crossing with a string
/// argument costs, for the bench to set beside the real calls.
#[napi(catch_unwind)]
pub fn noop(data: String) -> u32 {
    data.len() as u32
}
