//! Terminal wire types shared by everything that moves a terminal between
//! processes: vorn-sessiond, vornd, the native app and the server's history.
//!
//! - [`position`]: record headers and cursors, the one position both design
//!   docs use (Session Recovery Contract §4, Terminal State Protocol §5).
//! - [`screen`]: the mirror a grid client keeps instead of a VT parser (TP §6).
//! - [`row`]: the row cell encoding, `row_fmt` 1.
//! - [`bytes`]: the raw-byte stream for bytes clients: frame version 2 and
//!   the records it carries (TP §7, §14).
//! - [`msg`]: grid mode's messages and framing (TP §7), over [`cbor`].
//!
//! No dependencies and no Ghostty: a client links this without a terminal.
//! The `serde` feature adds the derives a wire encoding needs.

pub mod bytes;
pub mod cbor;
pub mod msg;
pub mod position;
pub mod row;
pub mod screen;

pub use position::{Cursor, Entry, GapReason, Record, RecordHeader, Stream};
