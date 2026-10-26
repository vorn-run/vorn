//! The Browser's route in vornd. An agent's `browser:*` calls, its MCP
//! browser tools and a published artifact opening reach the installed
//! Browser extension through the router; it drives the app's web views
//! with `web.call`.

use serde_json::Value;

use crate::native::{Answer, Native};

/// The id the Browser goes by.
pub const ID: &str = "vorn.browser";

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

/// Answers one of its `browser:*` calls.
pub async fn answer(native: &Native, method: &str, params: Value) -> Answer {
    super::answer(native, ID, "Browser", method, params).await
}
