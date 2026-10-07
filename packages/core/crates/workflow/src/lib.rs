//! Vorn's workflow engine: walks a stored workflow's steps through a
//! [`Host`], in waves, with conditions, loops, approval gates that can send
//! work back, sign-in waits, retries, partial runs and stops.
//!
//! The TypeScript engine that ran workflows before is the reference
//! (`tests/fixtures/js-reference/workflow-engine.json`): each case's runs,
//! the calls they made and the answers given are what that engine produced,
//! and `tests/reference.rs` replays every case here. What decides rather
//! than does lives in `vorn-work`; this crate waits, times and runs.

mod engine;
mod host;
mod steps;
mod triggers;
mod waves;

pub use engine::{
    same_token, workflows_of, Answer, Decision, Engine, GateComment, Options, Run, Started,
    LEASE_RENEW_INTERVAL, RESTORED_POLL_INTERVAL,
};
pub use host::{Completion, Host, Note, Source, TaskMove};
