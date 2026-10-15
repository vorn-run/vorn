//! What an agent's hooks tell Vorn, and how Vorn asks them to.
//!
//! An agent's adapter says how its hooks are installed: HTTP hooks merged
//! into its JSON settings ([`settings`]), or a hooks file of Vorn's own whose
//! scripts post the same shape ([`file`]). Both carry the terminal they come
//! from when Vorn started the agent, and a conversation id either way. [`event`]
//! reads a posted body, [`mapper`] decides which terminal it is about and
//! the status it means, and [`permission`] words the answer to a permission
//! request. [`owner`] decides which of several running Vorns registers the
//! endpoint, since there is one registration per user. [`capture`] reads
//! the conversation an agent without hooks took from the agent's own
//! database.

pub mod capture;
pub mod event;
pub mod file;
pub mod mapper;
pub mod owner;
pub mod permission;
pub mod settings;

pub use event::{Event, Malformed};
pub use mapper::{Mapper, Resolved, Terminal};
pub use owner::Owner;
