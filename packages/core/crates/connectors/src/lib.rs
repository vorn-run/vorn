//! Connectors, the built-in: connections and their secrets' shape, the
//! catalog of what can be installed, polls into the Workflows inbox, sign-in,
//! and the built-in HTTP and MCP connectors, answered on the public API
//! ([`builtin`], through [`host::Host`]).
//!
//! The packages themselves, their manifest, the programs they run and the
//! bridge's services are [`vorn_extensions`]'s: a connector is an extension
//! that adds a connector.

pub mod auth;
pub mod builtin;
pub mod catalog;
pub mod connections;
pub mod fetch;
pub mod host;
pub mod http;
pub mod mcp;
pub mod nodes;
pub mod page;
pub mod poll;
pub mod tools;
