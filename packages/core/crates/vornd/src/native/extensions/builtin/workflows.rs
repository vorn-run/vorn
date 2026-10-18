//! Workflow steps and triggers, the built-in ([`vorn_workflows`]), as an
//! entry of the built-ins' registry: its switch decides whether the engine's
//! registry holds its node types and trigger types. Switching it, like
//! installing any extension that declares some, fills the registry again
//! ([`crate::native::work::types`]); its steps run in vornd by the engine.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use vorn_extensions::api::types::Event;
use vorn_extensions::api::ExtError;

use super::{unknown, Api, Builtin};

/// The built-in.
#[derive(Debug, Default)]
pub struct Workflows;

impl Builtin for Workflows {
    fn manifest(&self) -> &'static str {
        vorn_workflows::MANIFEST
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
        async move { Err(unknown("Workflow steps and triggers", action)) }.boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }
}
