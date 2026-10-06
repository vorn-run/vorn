//! Who may reach a Vorn server and how.
//!
//! - [`origin`]: which browsers may open a socket.
//! - [`token`]: device tokens, the credential another machine presents.
//! - [`pairing`]: trading a code shown on screen for a token.
//! - [`tailscale`]: reading Tailscale's status.
//! - [`reachable`]: the addresses a browser on the network can use.
//! - [`sys`]: this machine's name and addresses.
//!
//! No I/O beyond asking the system for its name and addresses: the caller
//! runs programs and reads and writes the database.

pub mod origin;
pub mod pairing;
pub mod reachable;
pub mod sys;
pub mod tailscale;
pub mod token;
