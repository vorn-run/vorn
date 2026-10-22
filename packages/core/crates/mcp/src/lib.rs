//! Vorn's MCP server.
//!
//! The tool list, the argument schemas and the node reference are generated
//! from the tool definitions in `packages/mcp` by `scripts/gen-mcp-tools.mjs`
//! into `generated/`; calls are checked against them and answered here, with
//! results whose JSON keeps a stable key order.
//!
//! The crate does no I/O of its own. Tools reach the Vorn server through
//! [`Rpc`], which the host implements over whatever socket it has, and
//! [`http`] answers MCP's Streamable HTTP transport from a request the host
//! has already read. So the same code serves vornd and its tests.

pub mod http;
pub mod json;
pub mod rpc;
pub mod server;
pub mod toolkit;
mod tools;
pub mod workflow;
pub mod zod;

use std::sync::LazyLock;

use serde_json::Value;

pub use rpc::{Caller, Rpc, RpcError};
pub use server::Server;

static TOOLS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/tools.json"))
        .expect("generated/tools.json is JSON")
});

static HOSTED: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/hosted.json"))
        .expect("generated/hosted.json is JSON")
});

// The engine's part of the reference, generated beside the engine: the format's limits the tools check by.
static WORKFLOW_NODES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../workflow/generated/reference.json"))
        .expect("the workflow reference is JSON")
});

/// The `tools` of a `tools/list` answer, from `generated/tools.json`.
pub fn tools_list() -> &'static Value {
    &TOOLS
}

/// How many tools the server offers whatever is installed.
pub fn tool_count() -> usize {
    TOOLS.as_array().map_or(0, Vec::len)
}

/// The tools of each extension Vorn ships as an install (Browser, Device,
/// Tasks, Workflows), by its id, from `generated/hosted.json`. They are
/// Vorn's own tools, answered here or by that extension, but listed only
/// while it is installed, so an agent is never offered a tool nothing answers.
pub fn hosted_tools() -> impl Iterator<Item = (&'static str, &'static [Value])> {
    HOSTED.as_object().into_iter().flatten().map(|(id, tools)| {
        let tools = tools.as_array().map_or(&[][..], Vec::as_slice);
        (id.as_str(), tools)
    })
}

/// The engine's part of the workflow reference: its limits among it.
pub(crate) fn workflow_nodes() -> &'static Value {
    &WORKFLOW_NODES
}
