//! Artifacts, the built-in: pages and documents an agent publishes, their
//! versions and the comments a person leaves on them, and the review pages
//! approval gates keep. Its calls (`artifact:*`) and pages are answered by
//! the work model ([`crate::native::work`]), which asks [`on`] first: off,
//! every call is refused and every page is gone, and nothing is deleted.
//! Gates reach it through [`on`] too: off, a gate still asks, without a page
//! kept for comments.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use vorn_extensions::api::types::Event;
use vorn_extensions::api::ExtError;

use super::{unknown, Api, Builtin};
use crate::native::Native;

/// The id the built-in goes by.
pub const ID: &str = "vorn.artifacts";

/// What a call to Artifacts is told while it is off.
pub const OFF: &str = "Artifacts is off. Turn it on in Settings → Extensions.";

/// Whether Artifacts is on: as the built-ins' registry says, or on before it starts.
pub fn on(native: &Native) -> bool {
    native.extensions().is_none_or(|e| e.builtins().is_on(ID))
}

/// The built-in, as an entry of the built-ins' registry.
#[derive(Debug, Default)]
pub struct Artifacts;

impl Builtin for Artifacts {
    fn manifest(&self) -> &'static str {
        include_str!("artifacts.toml")
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        &[]
    }

    fn events(&self) -> &'static [&'static str] {
        &[]
    }

    fn on_by_default(&self) -> bool {
        true
    }

    fn start<'a>(&'a self, _api: &'a Api) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn handle<'a>(
        &'a self,
        _api: &'a Api,
        action: &'a str,
        _args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move { Err(unknown("Artifacts", action)) }.boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn its_manifest_reads() {
        vorn_extensions::manifest::file::parse_builtin(include_str!("artifacts.toml")).unwrap();
    }
}
