//! Tasks, the built-in: the board's writes (`task:*` but the list), each
//! task's images, the workflow triggers a write fires, the tasks connectors
//! import, the MCP task tools, the card badge of the task a session works
//! on, and the board's template, answered on the public API through
//! [`host::Host`].
//!
//! The rows stay in `vorn.db` under the names they always had (`tasks`,
//! `task_source_links`), and listing them is the core's: a paged query every
//! client and extension reads, so no write here ever reads the whole table.

pub mod builtin;
pub mod calls;
pub mod host;
pub mod images;
pub mod nodes;
pub mod sources;
pub mod tools;
pub mod triggers;

pub use builtin::{Answer, Tasks, ID, MANIFEST, METHODS, OFF};
pub use host::{Change, Host, Pending, TableWork};

/// What `describe_workflow_nodes` tells of the node types it declares, by id.
pub const NODE_DOCS: &str = include_str!("../generated/nodes.json");

/// The built-in's templates, as `("views/x.kdl", text)`.
pub const VIEWS: &[(&str, &str)] = &[
    ("views/board.kdl", include_str!("../views/board.kdl")),
    ("views/badge.kdl", include_str!("../views/badge.kdl")),
];
