//! The shipped set this build carries ([`vorn_extensions::shipped`]): the
//! packs `packages/core/shipped.toml` pins, embedded by `build.rs`.

use vorn_extensions::shipped::Shipped;

/// Every pack this build carries, in the lock's order.
pub static SET: &[Shipped<'static>] = include!(concat!(env!("OUT_DIR"), "/shipped.rs"));
