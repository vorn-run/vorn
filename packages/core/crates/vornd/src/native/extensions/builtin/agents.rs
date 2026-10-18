//! The agent adapters Vorn ships, each a built-in with its own switch: its
//! manifest's `[agent]` section is read by [`crate::adapters`], and its
//! switch decides whether that agent is on. A shipped adapter is data only:
//! it subscribes to nothing and answers no call.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use vorn_adapters::Shipped;
use vorn_extensions::api::types::Event;
use vorn_extensions::api::ExtError;

use super::{unknown, Api, Builtin};
use crate::adapters::Adapters;

/// One shipped adapter as a built-in.
pub struct AgentAdapter {
    shipped: &'static Shipped,
    adapters: Arc<Adapters>,
    /// Its manifest's `id`.
    extension: String,
}

impl AgentAdapter {
    fn new(shipped: &'static Shipped, adapters: Arc<Adapters>) -> AgentAdapter {
        let table: toml::Table = toml::from_str(shipped.manifest).unwrap_or_default();
        let extension = table
            .get("id")
            .and_then(toml::Value::as_str)
            .map_or_else(|| shipped.name.to_owned(), str::to_owned);
        AgentAdapter {
            shipped,
            adapters,
            extension,
        }
    }
}

/// Every shipped adapter, as built-ins sharing `adapters`.
pub fn entries(adapters: &Arc<Adapters>) -> Vec<Arc<dyn Builtin>> {
    vorn_adapters::shipped()
        .iter()
        .map(|s| Arc::new(AgentAdapter::new(s, Arc::clone(adapters))) as Arc<dyn Builtin>)
        .collect()
}

impl Builtin for AgentAdapter {
    fn manifest(&self) -> &'static str {
        self.shipped.manifest
    }

    fn events(&self) -> &'static [&'static str] {
        &[]
    }

    fn on_by_default(&self) -> bool {
        self.shipped.on_by_default
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
        let name = self.shipped.name;
        async move { Err(unknown(name, action)) }.boxed()
    }

    fn on_events<'a>(&'a self, _api: &'a Api, _batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async {}.boxed()
    }

    fn switched(&self, on: bool) {
        self.adapters.set_shipped(&self.extension, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_a_shipped_adapter_switches_its_agent() {
        let adapters = Adapters::shipped();
        let shipped = vorn_adapters::shipped().first().expect("an adapter ships");
        let a = AgentAdapter::new(shipped, Arc::clone(&adapters));
        let before = adapters.agents().len();
        a.switched(false);
        assert_eq!(adapters.agents().len(), before - 1);
        a.switched(true);
        assert_eq!(adapters.agents().len(), before);
    }
}
