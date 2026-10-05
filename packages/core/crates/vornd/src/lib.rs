//! vornd: one endpoint in front of Vorn's Node server.
//!
//! This first version owns no behaviour of its own. It answers the WebSocket and
//! HTTP endpoint clients already use and forwards everything to the Node server,
//! so groups of calls can later move into vornd one at a time behind
//! [`groups`] switches without clients noticing. It also has the parts every
//! long-running service needs: a log, a health check at
//! [`proxy::HEALTH_PATH`] and a protocol version ([`protocol::VORND_PROTOCOL`])
//! that clients see in the `Vornd-Protocol` header when their WebSocket opens.
//! Given a sessiond binary, it also keeps a session holder running
//! ([`holder`]) and, with the `engine` feature, runs every session it holds
//! through the session engine ([`engine`]), answers the terminal calls for
//! those sessions itself ([`terminal`], [`streams`]) and serves grid clients
//! on a local socket ([`grid`]). Given the server's credential, it is also
//! the server's process backend ([`node_link`]): the server creates, signals
//! and feeds its terminals and piped agents through vornd instead of spawning
//! them itself.

#[cfg(feature = "engine")]
pub mod engine;
#[cfg(feature = "engine")]
pub mod grid;
pub mod groups;
pub mod holder;
#[cfg(feature = "engine")]
pub mod node_link;
pub mod protocol;
pub mod proxy;
pub mod streams;
#[cfg(feature = "engine")]
pub mod terminal;

pub use groups::{Groups, Mode};

/// The variable the app hands vornd the server's credential in. Here rather
/// than only in [`node_link`] so a build without the engine still takes it
/// out of its environment.
pub const fn node_link_token_env() -> &'static str {
    "VORND_SERVER_TOKEN"
}
pub use proxy::{serve, Daemon};
