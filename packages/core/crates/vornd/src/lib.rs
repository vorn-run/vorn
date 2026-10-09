//! vornd, the Vorn server: the endpoint every client uses ([`endpoint`], [`serve`]), its session holder ([`holder`]) and every call ([`native`]).

pub mod applink;
pub mod boot;
pub mod bridge;
pub mod carry;
pub mod claims;
pub mod endpoint;
#[cfg(feature = "engine")]
pub mod engine;
#[cfg(feature = "engine")]
pub mod grid;
pub mod holder;
#[cfg(feature = "engine")]
pub mod journal;
pub mod mcp;
pub mod names;
pub mod native;
mod pair;
pub mod protocol;
pub mod registry;
pub mod serve;
#[cfg(feature = "engine")]
pub mod size;
pub mod streams;
#[cfg(feature = "engine")]
pub mod terminal;

pub use endpoint::Daemon;
