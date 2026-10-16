//! Workflows as files. A workflow is a KDL document, as templates are: a
//! `workflow` node named after it, its fields, its trigger and its steps in
//! order. A step runs after the one before it unless it says what it
//! `needs=`; a branch of a condition is `needs="check:true"`.
//!
//! ```kdl
//! // Reviews each pull request it is given.
//! workflow "Review a pull request" {
//!     id "review"
//!     icon "GitPullRequest"
//!     iconColor "#6366f1"
//!     enabled #true
//!     trigger "manual" id="start" {
//!         inputs {
//!             - {
//!                 key "pullRequest"
//!                 label "Pull request"
//!                 type "text"
//!             }
//!         }
//!     }
//!     step "script" id="fetch" slug="fetch" {
//!         scriptType "bash"
//!         scriptContent "gh pr diff {{inputs.pullRequest}}"
//!     }
//!     step "condition" id="big" needs="fetch" {
//!         variable "{{steps.fetch.output}}"
//!         operator "isNotEmpty"
//!     }
//!     step "launchAgent" id="review" needs="big:true" {
//!         agentType "claude"
//!         prompt """
//!             Review this diff.
//!             {{steps.fetch.output}}
//!             """
//!     }
//! }
//! ```
//!
//! A step's config is its children: a key and its value, an object as a
//! block of keys, a list as a block of `-` items. A connection is named
//! rather than numbered, `(connection)"workspace-eng"`, so a file shared
//! with someone runs against their connection of that name; a path in the
//! file's own project is `(project)"src/app"`. What the format cannot say
//! plainly (an edge with an id of its own, a field the editor does not
//! know) it still says, so every stored workflow reads back the same.
//!
//! [`write`] makes a file of a stored workflow, [`read`] reads one back
//! with every error at its line and column, and [`update`] writes a
//! workflow's changes into an existing file, keeping the comments and
//! layout of whatever did not change.

mod merge;
mod read;
mod write;

use std::fmt;

pub use merge::update;
pub use read::{outline, read, Outlined, Read};
pub use write::write;

/// One thing wrong with a workflow file, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileError {
    /// Line and column, from 1.
    pub at: (usize, usize),
    pub message: String,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.at.0, self.at.1, self.message)
    }
}

impl std::error::Error for FileError {}

/// What a file's names stand for on this machine: connections by name, and
/// the project the file belongs to.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bindings<'a> {
    /// Each connection's id and name.
    pub connections: &'a [(String, String)],
    /// The project the file is in: its name and path.
    pub project: Option<(&'a str, &'a str)>,
}

/// The config keys whose value is a connection's id.
pub const CONNECTION_KEYS: [&str; 3] = ["connectionId", "profileConnectionId", "secretsFrom"];

/// The fields of a stored workflow the file leaves to the database: when it
/// last ran and how.
pub const RUNTIME_FIELDS: [&str; 2] = ["lastRunAt", "lastRunStatus"];

/// The `workflows/<name>.kdl` file name a workflow is saved under.
pub fn file_name(name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    let stem: String = out.chars().take(60).collect();
    format!("{}.kdl", if stem.is_empty() { "workflow" } else { &stem })
}

#[cfg(test)]
mod tests;
