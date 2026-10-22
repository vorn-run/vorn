//! The extensions Vorn ships as Wasm components. Each is a [`Builtin`] like
//! the rest, with a switch, but its logic is a component compiled into vornd
//! that runs through the router as an installed component does: the same
//! gate, lifecycle and crash budget, compiled into the cache by the vorn
//! command the first time it is used.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use vorn_extensions::api::types::{CallCtx, Event};
use vorn_extensions::api::ExtError;
use vorn_extensions::component::Bundled;

use super::{Api, Builtin};

/// One extension Vorn ships as a component.
pub struct Component {
    /// Its `vorn-extension.toml`.
    pub manifest: &'static str,
    /// The events its manifest's `[service]` subscribes to.
    pub events: &'static [&'static str],
    /// The component, built for wasm32-wasip2.
    pub wasm: &'static [u8],
    pub on_by_default: bool,
    /// What it is called as it is switched on, to publish what it shows; none starts it at its first use.
    pub start: Option<&'static str>,
    /// What it is called as it is switched off, to let go of what it holds outside Vorn.
    pub stop: Option<&'static str>,
}

/// Workflows (`packages/core/extensions/workflows`): its view, menus and
/// gates, published as it is switched on, and the steps and triggers the
/// engine runs ([`vorn_workflows`]).
pub fn workflows() -> Component {
    use vorn_extensions::events::{WORKFLOW_CHANGED, WORKFLOW_GATES};
    Component {
        manifest: vorn_workflows::MANIFEST,
        events: &[WORKFLOW_CHANGED, WORKFLOW_GATES],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/workflows.wasm")),
        on_by_default: true,
        start: Some("refresh"),
        stop: None,
    }
}

impl Component {
    fn bundled(&self, api: &Api) -> Bundled {
        Bundled {
            manifest: Arc::clone(&api.manifest),
            wasm: self.wasm,
        }
    }

    /// Its own `action`, called by vornd as it starts or stops it.
    async fn call_own(&self, api: &Api, action: &str) {
        let ctx = CallCtx {
            caller: "client".to_owned(),
            project: None,
        };
        if let Err(e) = api
            .router
            .call_bundled(&self.bundled(api), action, Value::Null, ctx)
            .await
        {
            tracing::warn!(extension = %api.manifest.id, %action, error = %e.message, "failed");
        }
    }
}

impl Builtin for Component {
    fn manifest(&self) -> &'static str {
        self.manifest
    }

    fn events(&self) -> &'static [&'static str] {
        self.events
    }

    fn on_by_default(&self) -> bool {
        self.on_by_default
    }

    /// Nothing until it is used, unless it publishes what it shows from the
    /// start: the first call or event compiles it into the cache.
    fn start<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            if let Some(action) = self.start {
                self.call_own(api, action).await;
            }
        }
        .boxed()
    }

    fn handle<'a>(
        &'a self,
        api: &'a Api,
        action: &'a str,
        args: Value,
    ) -> BoxFuture<'a, Result<Value, ExtError>> {
        async move {
            let ctx = CallCtx {
                caller: "client".to_owned(),
                project: None,
            };
            api.router
                .call_bundled(&self.bundled(api), action, args, ctx)
                .await
        }
        .boxed()
    }

    fn on_events<'a>(&'a self, api: &'a Api, batch: Vec<Arc<Event>>) -> BoxFuture<'a, ()> {
        async move { api.router.hand_bundled(&self.bundled(api), &batch).await }.boxed()
    }

    fn stop<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            if let Some(action) = self.stop {
                self.call_own(api, action).await;
            }
            api.router.stop(&api.manifest.id).await;
        }
        .boxed()
    }
}
