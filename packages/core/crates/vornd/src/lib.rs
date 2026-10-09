//! vornd: one endpoint in front of Vorn's Node server.
//!
//! It answers the WebSocket and HTTP endpoint clients already use and forwards
//! to the Node server everything it has not taken over, so groups of calls move
//! into vornd one at a time behind [`groups`] switches without clients
//! noticing. It also has the parts every
//! long-running service needs: a log, a health check at
//! [`proxy::HEALTH_PATH`] and a protocol version ([`protocol::VORND_PROTOCOL`])
//! that clients see in the `Vornd-Protocol` header when their WebSocket opens.
//! Given a sessiond binary, it also keeps a session holder running
//! ([`holder`]) and, with the `engine` feature, runs every session it holds
//! through the session engine ([`engine`]), answers the terminal calls for
//! those sessions itself ([`terminal`], [`streams`]) and serves grid clients
//! on a local socket ([`grid`]). Both kinds of client report what fits on
//! them, and one rule decides each session's size ([`size`]).
//!
//! The groups of calls it has taken over from the server ([`native`]) are
//! answered here ([`groups`]). So, under the `mcp` group, is Vorn's MCP server,
//! at `/mcp` ([`mcp`]).

pub mod applink;
pub mod boot;
pub mod bridge;
pub mod carry;
pub mod claims;
#[cfg(feature = "engine")]
pub mod control;
#[cfg(feature = "engine")]
pub mod engine;
#[cfg(feature = "engine")]
pub mod grid;
pub mod groups;
pub mod holder;
#[cfg(feature = "engine")]
pub mod journal;
pub mod mcp;
pub mod names;
pub mod native;
mod pair;
pub mod protocol;
pub mod proxy;
pub mod registry;
pub mod serve;
#[cfg(feature = "engine")]
pub mod size;
pub mod streams;
#[cfg(feature = "engine")]
pub mod terminal;

pub use groups::{Groups, Mode};
pub use proxy::{serve, Daemon};
