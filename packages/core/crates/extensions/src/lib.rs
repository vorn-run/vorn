//! Extension hosting: the packs that extend Vorn, the child each runs as per
//! project, the bridge it calls back on, and the pages its panes draw.
//!
//! Nothing here knows how sessions are kept or how a client is reached;
//! the daemon that hosts extensions hands those in. That keeps one host for
//! every daemon that may grow to load plugins.

pub mod activation;
pub mod bridge;
pub mod footer;
pub mod grants;
pub mod host;
pub mod links;
pub mod page;
pub mod token;
pub mod usage;

pub use vorn_connectors::{child, js, manifest, pack};
