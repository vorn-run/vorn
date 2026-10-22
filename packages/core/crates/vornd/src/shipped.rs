//! The shipped set this build carries ([`vorn_extensions::shipped`]): the
//! packs `packages/core/shipped.toml` pins, embedded by `build.rs`, and the
//! pins of `packages/core/pinned.toml`, Vorn's own that the catalog offers.

use vorn_extensions::install::Pinned;
use vorn_extensions::shipped::Shipped;

/// Every pack this build carries, in the lock's order.
pub static SET: &[Shipped<'static>] = include!(concat!(env!("OUT_DIR"), "/shipped.rs"));

/// `(id, version, sha256)` of each of Vorn's own packs this build does not
/// carry but vouches for, so the catalog's may have an id in Vorn's namespace.
pub static CATALOG: &[(&str, &str, &str)] = include!(concat!(env!("OUT_DIR"), "/pinned.rs"));

/// Every pack this build vouches for: the carried set's and the catalog's.
pub fn pins() -> Vec<Pinned> {
    let catalog = CATALOG.iter().map(|(id, version, sha256)| Pinned {
        id: (*id).to_owned(),
        version: (*version).to_owned(),
        sha256: (*sha256).to_owned(),
    });
    SET.iter().map(Shipped::pinned).chain(catalog).collect()
}
