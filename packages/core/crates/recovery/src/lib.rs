//! vorn-recovery: the test harness session recovery work shares.
//!
//! - [`gen`]: seeded VT: shell output, full-screen redraws, resizes, OSC,
//!   UTF-8 and sequences split across records, unterminated sequences.
//! - [`transcript`]: recorded programs (vim, htop, an agent CLI) as logs.
//! - [`log`]: record logs with sessiond's headers, and their digest.
//! - [`scan`]: whether a record boundary is a safe checkpoint point.
//! - [`compare`]: the recovery contract's state equivalence.
//! - [`engine`]: the engine trait vornd's session engine implements, and a
//!   reference engine that stands in for it until then.
//! - [`driver`]: kills a target at chosen, random or timed records, recovers
//!   it, and runs the differential test in one call.
//! - [`child`]: the same with a real child process killed by the OS.
//! - [`emit`]: seeded output for a real terminal, printed by the
//!   `recovery-emit` binary, for tests that run it in sessiond.
//!
//! The harness models sessiond itself (the log, the checkpoint store), so
//! what it tests is the engine's side of recovery.
//!
//! The differential test, in one call: the same log through an engine that
//! never dies and one killed after five random records and recovered from
//! its checkpoints, compared by the equivalence.
//!
//! ```
//! use vorn_recovery::gen::{Generator, Profile};
//! use vorn_recovery::{differential, InProcess, KillPlan, ReferenceConfig, ReferenceEngine, Restore};
//!
//! let log = Generator::log(7, Profile::round_trip().bytes(64 << 10));
//! let report = differential(&log, &KillPlan::random(7, 5), || {
//!     Ok(InProcess::<ReferenceEngine>::new(ReferenceConfig::default(), Restore::Checkpoint))
//! })?;
//! assert_eq!(report.digest, log.digest());
//! # Ok::<(), vorn_recovery::Error>(())
//! ```

pub mod child;
pub mod compare;
pub mod driver;
pub mod emit;
pub mod engine;
mod error;
pub mod gen;
pub mod log;
mod rng;
pub mod scan;
pub mod transcript;

pub use child::ChildProcess;
pub use compare::{compare, Check, Mismatch, Subject, TermState};
pub use driver::{differential, reference, run, Chaos, InProcess, KillPlan, Report, Target};
pub use engine::{Checkpoint, Engine, ReferenceConfig, ReferenceEngine, Restore, Resume, Store};
pub use error::Error;
pub use gen::{Generator, Mix, Profile, Until};
pub use log::{Digest, Log, LogBuilder, Size};
pub use rng::Rng;
pub use scan::{Scanner, VtState};
