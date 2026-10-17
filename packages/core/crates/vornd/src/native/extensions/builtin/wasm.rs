//! The extensions Vorn ships as Wasm components. Each is a [`Builtin`] like
//! the rest, with a switch, but its logic is a component compiled into vornd
//! that runs through the router as an installed component does: the same
//! gate, lifecycle and crash budget, compiled into the cache by the vorn
//! command the first time it is switched on.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use serde_json::Value;
use tracing::warn;
use vorn_extensions::api::types::{CallCtx, Event};
use vorn_extensions::api::ExtError;
use vorn_extensions::component::Bundled;

use super::{Api, Builtin};

/// One extension Vorn ships as a component.
pub struct Component {
    /// Its `vorn-extension.toml`.
    pub manifest: &'static str,
    /// Its templates, as `("views/x.kdl", text)`.
    pub views: &'static [(&'static str, &'static str)],
    /// The events its manifest's `[service]` subscribes to.
    pub events: &'static [&'static str],
    /// The component, built for wasm32-wasip2.
    pub wasm: &'static [u8],
    pub on_by_default: bool,
}

/// Worktree cleanup (`packages/core/extensions/worktrees`).
pub fn worktrees() -> Component {
    Component {
        manifest: include_str!("../../../../../../extensions/worktrees/vorn-extension.toml"),
        views: &[
            (
                "views/idle.kdl",
                include_str!("../../../../../../extensions/worktrees/views/idle.kdl"),
            ),
            (
                "views/confirm.kdl",
                include_str!("../../../../../../extensions/worktrees/views/confirm.kdl"),
            ),
        ],
        events: &[vorn_extensions::events::WORKTREE_IDLE],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/worktrees.wasm")),
        on_by_default: true,
    }
}

impl Component {
    fn bundled(&self, api: &Api) -> Bundled {
        Bundled {
            manifest: Arc::clone(&api.manifest),
            wasm: self.wasm,
        }
    }
}

impl Builtin for Component {
    fn manifest(&self) -> &'static str {
        self.manifest
    }

    fn views(&self) -> &[(&'static str, &'static str)] {
        self.views
    }

    fn events(&self) -> &'static [&'static str] {
        self.events
    }

    fn on_by_default(&self) -> bool {
        self.on_by_default
    }

    /// Compiles it into the cache, so its first use waits for nothing.
    fn start<'a>(&'a self, api: &'a Api) -> BoxFuture<'a, ()> {
        async move {
            if let Err(err) = api.router.prepare_bundled(&self.bundled(api)).await {
                warn!("[extensions] {err}");
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
        async move { api.router.stop(&api.manifest.id).await }.boxed()
    }
}
