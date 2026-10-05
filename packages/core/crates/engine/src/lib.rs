//! vorn-engine: vornd's session engine, the one place a session's output is
//! parsed.
//!
//! sessiond holds the sessions and their record logs; vornd attaches to
//! each, feeds every record through a terminal ([`vorn_screen::Emulator`])
//! and the output analyzer, writes the disk history, and now and then cuts a
//! checkpoint for sessiond to keep. When vornd dies, the next one rebuilds
//! every session from a checkpoint and the records after it.
//!
//! - [`session`]: one session's actor, a state machine with no I/O: restore
//!   base selection, replay mode, effects and their ids, checkpoint cadence.
//! - [`pool`]: the fixed worker threads the actors run on, which also keep
//!   each session's grid render clock.
//! - the checkpoint blob, [`FORMAT`]: the terminal, the analyzer and the
//!   state between them.
//!
//! Plain Rust with no sockets in it: vornd's driver carries [`Input`] from
//! sessiond and [`Out`] back.

pub mod pool;
pub mod session;
mod term;

pub use pool::{Pool, Sink};
pub use session::{
    Base, Brief, Cadence, Config, Effect, EffectId, Input, Open, Out, Session, State, Summary,
    PIPED_SIZE,
};
pub use term::{Fidelity, Rejected, FORMAT};
pub use vorn_grid::{GridIn, HubConfig, HubOut, Peer};
