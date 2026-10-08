//! What an agent's hooks tell Vorn, and how Vorn asks them to.
//!
//! Claude Code posts each hook event to an HTTP endpoint named in its
//! settings ([`claude`]); Copilot runs a script per event from a hooks file
//! that posts the same shape ([`copilot`]). Both carry the terminal they come
//! from when Vorn started them, and a conversation id either way. [`event`]
//! reads a posted body, [`mapper`] decides which terminal it is about and
//! the status it means, and [`permission`] words the answer to a permission
//! request. [`owner`] decides which of several running Vorns registers the
//! endpoint, since there is one registration per user. [`capture`] reads
//! the conversation an agent without hooks took from the agent's own
//! database.

pub mod capture;
pub mod claude;
pub mod copilot;
pub mod event;
pub mod mapper;
pub mod owner;
pub mod permission;

pub use event::{Event, Malformed};
pub use mapper::{Mapper, Resolved, Terminal};
pub use owner::Owner;
