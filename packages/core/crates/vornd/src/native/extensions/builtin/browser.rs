//! The Browser in vornd: the component Vorn ships
//! (`packages/core/extensions/browser`) as an entry of the built-ins'
//! registry, and the core's route to it. An agent's `browser:*` calls, its
//! MCP browser tools and a published artifact opening reach the component
//! through the router, under the same gate and lifecycle as an installed
//! component; it drives the app's web views with `web.call`. While its
//! switch is off each call is refused with [`OFF`].

use serde_json::Value;

use super::wasm::Component;
use crate::native::{Answer, Native};

/// The id the Browser goes by.
pub const ID: &str = "vorn.browser";

/// What a call to a browser method is told while the Browser is off.
pub const OFF: &str = "Browser is off. Turn it on in Settings → Extensions.";

/// Every `browser:*` call it answers, each by the action of the same name.
pub const METHODS: &[&str] = &[
    "browser:openPane",
    "browser:navigate",
    "browser:tabs",
    "browser:listTabs",
    "browser:history",
    "browser:readPage",
    "browser:getText",
    "browser:find",
    "browser:interact",
    "browser:consoleMessages",
    "browser:networkRequests",
    "browser:screenshot",
];

/// The component, built for wasm32-wasip2 by vornd's build script; nothing
/// runs until the first pane opens or an agent calls.
pub fn component() -> Component {
    Component {
        manifest: include_str!("../../../../../../extensions/browser/vorn-extension.toml"),
        events: &[],
        wasm: include_bytes!(concat!(env!("OUT_DIR"), "/browser.wasm")),
        on_by_default: true,
        start: None,
    }
}

/// Answers one of its `browser:*` calls; while it is off, [`OFF`].
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    let Some(ext) = native.extensions() else {
        return Answer::Error("vornd is not hosting extensions".to_owned());
    };
    let builtins = ext.builtins();
    if !builtins.is_on(ID) {
        return Answer::Error(OFF.to_owned());
    }
    match builtins.call(ID, method, params).await {
        Some(Ok(v)) => Answer::Result(v),
        Some(Err(e)) => Answer::Error(e.message),
        None => Answer::Unanswered,
    }
}
