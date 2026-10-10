//! The vornd client: finding it ([`endpoint`]), calling it ([`rpc`]) and
//! the models the screen reads, kept current from its notifications
//! ([`store`]).

pub mod endpoint;
pub mod rpc;
pub mod store;

pub use endpoint::{data_dir, Endpoint, FindError};
pub use rpc::{Event, Pending, Rpc, RpcError};
pub use store::{Change, Session, Store};
