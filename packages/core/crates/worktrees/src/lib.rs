//! vorn-worktrees: the worktree manager's inventory and cleanup.
//!
//! The inventory lists every worktree of each project on this machine: how
//! much it holds, how much of that is rebuildable build output, when it was
//! last worked in, and a [`Verdict`] on the safest thing to do with it. The
//! actions remove worktrees, sweep their build output, and delete directories
//! git has forgotten. Each answers what the server's `worktree-inventory`
//! answers, in its words, so a client cannot tell which one ran.
//!
//! Nothing here knows which sessions are running: the host says, through the
//! `active` callback of a scan and a [`Guard`] for the actions, which every
//! action asks again right before it deletes anything, since a session can
//! start while the person looks at the list. Remote hosts are not reached:
//! their projects stay the server's.

mod guard;
mod remove;
mod scan;
mod size;
mod verdict;

pub use guard::{assert_inside_worktree, assert_removable_path};
pub use remove::{
    prune_orphan_dirs, reclaim_artifacts, remove_worktrees, ActionResult, Cleanup, Failure, Guard,
    RemoveItem,
};
pub use scan::{
    collect_stale_branches, iso_millis, list_orphan_dirs, scan, BranchInfo, Entry, Inventory,
    Project, ProjectInventory, Retention, Scan, StaleBranch,
};
pub use size::{find_artifact_dirs, Size, Sizes};
pub use verdict::{verdict, Kind, Level, Verdict, VerdictInput};

/// The directory every worktree vorn makes is parked under, beside the project.
pub const WORKTREE_ROOT_SEGMENT: &str = ".vorn-worktrees";

/// Directory names taken for rebuildable build output when the person has
/// named none (`DEFAULT_ARTIFACT_DIRS`).
pub const DEFAULT_ARTIFACT_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "out",
    ".next",
    ".turbo",
    ".nuxt",
    "target",
    "coverage",
    ".venv",
    "__pycache__",
];

/// Days a merged, clean worktree sits idle before it is pre-selected.
pub const DEFAULT_IDLE_DAYS_THRESHOLD: f64 = 14.0;
