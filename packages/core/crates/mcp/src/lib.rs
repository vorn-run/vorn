//! Vorn's MCP server, answering as the TypeScript one in `packages/mcp` does.
//!
//! An agent cannot tell which of the two it is talking to: the same tools are
//! listed with the same schemas, a call is checked by the same rules and
//! refused in the same words, and a result reads the same down to the key
//! order of its JSON. What the TypeScript writes down as data (the tool list,
//! the argument schemas, the node reference) is generated from it by
//! `scripts/gen-mcp-tools.mjs` into `generated/`; what it writes as code is
//! ported here.
//!
//! The crate does no I/O of its own. Tools reach the Vorn server through
//! [`Rpc`], which the host implements over whatever socket it has, and
//! [`http`] answers MCP's Streamable HTTP transport from a request the host
//! has already read. So the same code serves vornd and its tests.

pub mod http;
pub mod json;
pub mod rpc;
pub mod server;
mod time;
mod tools;
mod workflow;
pub mod zod;

use std::sync::LazyLock;

use serde_json::Value;

pub use rpc::{Caller, Rpc, RpcError};
pub use server::Server;

static TOOLS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/tools.json"))
        .expect("generated/tools.json is JSON")
});

static WORKFLOW_NODES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../generated/workflow-nodes.json"))
        .expect("generated/workflow-nodes.json is JSON")
});

/// The `tools` of a `tools/list` answer, exactly as the TypeScript sends them.
pub fn tools_list() -> &'static Value {
    &TOOLS
}

/// How many tools the server offers.
pub fn tool_count() -> usize {
    TOOLS.as_array().map_or(0, Vec::len)
}

/// The reference `describe_workflow_nodes` answers with.
pub(crate) fn workflow_nodes() -> &'static Value {
    &WORKFLOW_NODES
}
