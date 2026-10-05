//! vorn-grid: grid mode's server half (Terminal State Protocol §5 to §9).
//!
//! vornd keeps the only terminal per session and sends a grid client a
//! mirror of its screen instead of bytes. This crate turns a session's
//! libghostty-vt terminal into that mirror's frames, with no sockets and no
//! threads of its own, so vornd's session engine drives it from the worker
//! that owns the terminal and its tests and benchmarks are plain Rust.
//!
//! - [`grid`]: one session's render state, row cache and revisions;
//!   snapshots and deltas are cut from it.
//! - [`tables`]: the style and link tables rows refer to.
//! - [`lines`]: absolute line numbers and the scrollback epoch.
//! - [`hub`]: a session's attachments, the render clock, credits and
//!   synchronized output; everything a client asks of the session.
//! - [`input`]: input events encoded against the terminal's modes.
//! - [`query`]: history by line, selection, copy and search.
//!
//! The wire types and their encoding are `vorn-term-proto`'s; the client
//! side is `vorn-term-mirror`, which links no terminal.

pub mod grid;
pub mod hub;
pub mod input;
pub mod lines;
mod query;
pub mod tables;

use std::fmt;

pub use grid::{Grid, Held, LOG_LEN};
pub use hub::{Ctx, GridIn, Hub, HubConfig, HubOut, Peer};
pub use input::{Encoded, InputEncoder};
pub use lines::Lines;
pub use query::{MAX_FETCH, MAX_HITS};

/// libghostty-vt refused a read or an allocation. The grid recovers by
/// re-encoding everything at its next update.
#[derive(Debug, Clone, Copy)]
pub struct Error(pub libghostty_vt::Error);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libghostty-vt: {:?}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<libghostty_vt::Error> for Error {
    fn from(e: libghostty_vt::Error) -> Self {
        Error(e)
    }
}
