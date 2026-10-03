//! Vorn's native core.
//!
//! The server loads this as `vorn_core.node` when `VORN_CORE=native`. For now it
//! only proves the boundary: Node can call into Rust, and Rust can drive
//! libghostty-vt. The screen model, output analysis and flush pipeline land
//! here in later work packages.

use napi_derive::napi;

/// What the loaded binary was built from, so the server can log it.
#[napi(object)]
pub struct CoreInfo {
    /// The crate version, kept in step with the app version.
    pub version: String,
    /// libghostty-vt's own version, or `None` when built without it.
    pub ghostty: Option<String>,
}

#[napi]
pub fn info() -> CoreInfo {
    CoreInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        ghostty: ghostty_version(),
    }
}

#[napi]
pub fn hello(name: String) -> String {
    format!("hello {name} from vorn-core {}", env!("CARGO_PKG_VERSION"))
}

/// Feeds bytes through a throwaway terminal and returns the title they set.
///
/// A smoke test for the libghostty-vt link: an OSC 2 sequence in, the title out.
#[cfg(feature = "ghostty")]
#[napi]
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
