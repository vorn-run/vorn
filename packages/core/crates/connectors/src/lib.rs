//! Connectors: connections and their secrets' shape, the catalog of what can
//! be installed, polls and their inbox, sign-in, the installer of 0.8's
//! connector packs, and the built-in HTTP and MCP connectors.
//!
//! The packages themselves, their manifest and the programs they run are
//! [`vorn_extensions`]'s: a connector is an extension that adds a connector.

pub mod auth;
pub mod catalog;
pub mod connections;
pub mod fetch;
pub mod http;
pub mod install;
pub mod poll;
