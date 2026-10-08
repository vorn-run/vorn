//! Connectors: the packs installed on disk and the manifests they carry, the
//! child each runs as in Vorn's connector protocol, the catalog of what can be
//! installed, and the built-in HTTP and MCP connectors.
//!
//! Extensions are packs too, so the extension host builds on the same packs,
//! manifests and children.

pub mod catalog;
pub mod child;
pub mod connections;
pub mod fetch;
pub mod install;
pub mod js;
pub mod manifest;
pub mod pack;
pub mod poll;
pub mod sdk;
