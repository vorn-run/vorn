//! Vorn's workflow engine: walks a stored workflow's steps through a
//! [`Host`], in waves, with conditions, loops, approval gates that can send
//! work back, sign-in waits, retries, partial runs and stops.
//!
//! The engine knows four kinds of node: where a run starts, a branch, a
//! gate and a loop. Every other step is run by a node type an extension
//! declares, and every trigger is fired by a trigger type one declares, both
//! kept in the [`Registry`]. What decides rather than does lives in
//! `vorn-work`; this crate waits, times and runs. The TypeScript engine that
//! ran workflows before is the reference, replayed with Vorn's own node
//! types by the Workflows built-in's tests.

mod engine;
mod host;
pub mod parallel;
pub mod registry;
mod step;
mod steps;
mod triggers;
mod waves;

pub use engine::{
    same_token, workflows_of, Answer, Decision, Engine, GateComment, Options, Run, Started,
    LEASE_RENEW_INTERVAL, RESTORED_POLL_INTERVAL,
};
pub use host::{Completion, Host, Note, Source};
pub use registry::{NodeType, Registry, Resolved, Runner, TriggerType};
pub use step::{Step, StepCx};
pub use waves::BoxFuture;
