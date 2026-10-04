//! vorn-sessiond: the process that holds every terminal and piped agent, so
//! they outlive vornd (Session Recovery Contract §1 and §3).
//!
//! It never parses terminal output. It reads what each session prints, numbers
//! it into a record log ([`log`]), keeps that log in a memory ring with a disk
//! spool behind it ([`spool`]), stores the checkpoints vornd hands it, and
//! serves all of it over a local socket ([`wire`]). Everything here is kept
//! small on purpose: sessiond's crash rate is the ceiling on every guarantee
//! the contract makes.

pub mod launch;
pub mod log;
pub mod os;
pub mod server;
pub mod session;
pub mod spool;
/// The socket protocol, from the crate vornd shares.
pub use vorn_sessiond_wire as wire;

pub use log::{AppendError, Budget, Overflow, SessionLog, SpoolPool};
pub use wire::{AttachFrom, AttachRefusal, Checkpoint};
