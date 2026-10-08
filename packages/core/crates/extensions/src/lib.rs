//! Extension hosting: the packs that extend Vorn, the child each runs as per
//! project, the bridge it calls back on, and the pages its panes draw.
//!
//! Nothing here knows how sessions are kept or how a client is reached;
//! the daemon that hosts extensions hands those in. That keeps one host for
//! every daemon that may grow to load plugins.

pub mod activation;
pub mod bridge;
pub mod child;
pub mod footer;
pub mod grants;
pub mod host;
pub mod js;
pub mod links;
pub mod manifest;
pub mod pack;
pub mod page;
pub mod token;
pub mod usage;
