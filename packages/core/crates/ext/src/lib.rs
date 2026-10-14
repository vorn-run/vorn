//! Extensions in two tiers on one manifest.
//!
//! A quick extension is a folder with a manifest and any program, run as the
//! person over stdio ([`quick`]). A safe extension is the same manifest with a
//! precompiled WebAssembly component, run inside vornd on Wasmtime without its
//! compiler ([`wasm`]). Both call back through one [`bridge::Gate`] and both
//! answer with rows a typed [`template`] draws, so clients never learn the tier.

#[cfg(feature = "host")]
pub mod bridge;
#[cfg(feature = "host")]
pub mod checks;
#[cfg(feature = "host")]
pub mod doc;
#[cfg(feature = "live")]
pub mod live;
#[cfg(feature = "host")]
pub mod manifest;
#[cfg(feature = "host")]
pub mod promote;
#[cfg(feature = "host")]
pub mod quick;
pub mod template;
#[cfg(feature = "host")]
pub mod view_file;
#[cfg(feature = "wasm")]
pub mod wasm;

use std::fmt;

/// A failed extension call, as the WIT's `ext-error` and the stdio mapping carry it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtError {
    /// Stable, for callers to match: `denied`, `timeout`, `trap`, `bad-output`, ...
    pub code: String,
    pub message: String,
}

impl ExtError {
    pub fn new(code: &str, message: impl Into<String>) -> ExtError {
        ExtError {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ExtError {}
