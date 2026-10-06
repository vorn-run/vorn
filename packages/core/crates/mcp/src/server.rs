//! The MCP protocol as the SDK's `McpServer` speaks it for a tools-only server.
//!
//! `initialize`, `ping`, `tools/list` and `tools/call` are answered; any other
//! request is "Method not found", and notifications need no answer. Each
//! request is checked against the SDK's own schema for it first, since a
//! malformed one is refused with zod's issues, and a tool call's arguments
//! against the tool's schema, refused the same way the SDK refuses them.

use std::sync::LazyLock;

use serde_json::{json, Map, Value};

use crate::rpc::{Caller, Rpc};
use crate::tools::{self, Args, Cx};
use crate::zod::{self, Registry};

/// The newest protocol version, offered to a client asking for one this
/// server does not know.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Every version the SDK accepts, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

const METHOD_NOT_FOUND: i64 = -32601;
const INTERNAL_ERROR: i64 = -32603;

/// The SDK's request schemas, in the generator's format, keyed by method.
/// Only the parts a tools-only server reads are spelled out; a client's
/// capabilities need only be an object, which is all the server checks of them.
const REQUEST_SCHEMAS: &str = r#"{
  "defs": {
    "meta": { "t": "object", "mode": "loose", "shape": [
      ["progressToken", { "t": "optional", "inner": { "t": "union", "options": [
        { "t": "string" }, { "t": "number", "checks": [{ "k": "int" }] }
      ] } }],
      ["io.modelcontextprotocol/related-task", { "t": "optional", "inner": { "t": "object", "mode": "strip", "shape": [
        ["taskId", { "t": "string" }]
      ] } }]
    ] },
    "baseParams": { "t": "object", "mode": "strip", "shape": [
      ["_meta", { "t": "optional", "inner": { "t": "ref", "name": "meta" } }]
    ] }
  },
  "tools": {
    "initialize": { "t": "object", "mode": "strip", "shape": [
      ["method", { "t": "literal", "values": ["initialize"] }],
      ["params", { "t": "object", "mode": "strip", "shape": [
        ["_meta", { "t": "optional", "inner": { "t": "ref", "name": "meta" } }],
        ["protocolVersion", { "t": "string" }],
        ["capabilities", { "t": "object", "mode": "loose", "shape": [] }],
        ["clientInfo", { "t": "object", "mode": "strip", "shape": [
          ["name", { "t": "string" }],
          ["title", { "t": "optional", "inner": { "t": "string" } }],
          ["icons", { "t": "optional", "inner": { "t": "unknown" } }],
          ["version", { "t": "string" }],
          ["websiteUrl", { "t": "optional", "inner": { "t": "string" } }],
          ["description", { "t": "optional", "inner": { "t": "string" } }]
        ] }]
      ] }]
    ] },
    "ping": { "t": "object", "mode": "strip", "shape": [
      ["method", { "t": "literal", "values": ["ping"] }],
      ["params", { "t": "optional", "inner": { "t": "ref", "name": "baseParams" } }]
    ] },
    "tools/list": { "t": "object", "mode": "strip", "shape": [
      ["method", { "t": "literal", "values": ["tools/list"] }],
      ["params", { "t": "optional", "inner": { "t": "object", "mode": "strip", "shape": [
        ["_meta", { "t": "optional", "inner": { "t": "ref", "name": "meta" } }],
        ["cursor", { "t": "optional", "inner": { "t": "string" } }]
      ] } }]
    ] },
    "tools/call": { "t": "object", "mode": "strip", "shape": [
      ["method", { "t": "literal", "values": ["tools/call"] }],
      ["params", { "t": "object", "mode": "strip", "shape": [
        ["_meta", { "t": "optional", "inner": { "t": "ref", "name": "meta" } }],
        ["task", { "t": "optional", "inner": { "t": "object", "mode": "strip", "shape": [
          ["ttl", { "t": "optional", "inner": { "t": "number" } }]
        ] } }],
        ["name", { "t": "string" }],
        ["arguments", { "t": "optional", "inner": { "t": "record", "value": { "t": "unknown" } } }]
      ] }]
    ] }
  }
}"#;

static REQUESTS: LazyLock<Registry> = LazyLock::new(|| {
    let value: Value = serde_json::from_str(REQUEST_SCHEMAS).expect("REQUEST_SCHEMAS is JSON");
    Registry::from_json(&value).expect("REQUEST_SCHEMAS is a schema file")
});

/// A request checked against the SDK's schema for it: the parsed request, or
/// the error the SDK answers with.
fn parse_request(method: &str, request: &Value) -> Result<Value, Value> {
    let Some(Some(schema)) = REQUESTS.tool(method) else {
        return Ok(request.clone());
    };
    match REQUESTS.parse(schema, Some(request)) {
        Ok(parsed) => Ok(parsed.unwrap_or(Value::Null)),
        Err(issues) => {
            Err(json!({ "code": INTERNAL_ERROR, "message": zod::error_message(&issues) }))
        }
    }
}

/// `isInitializeRequest`: whether a message parses as an `initialize` request.
/// The SDK's schema does not ask for an id, so neither does this.
pub fn is_initialize_request(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("initialize")
        && parse_request("initialize", message).is_ok()
}

/// One MCP server: its name and version, and the tools.
#[derive(Clone, Debug)]
pub struct Server {
    version: String,
}

impl Server {
    /// A server reporting itself as `vorn` at `version`.
    pub fn new(version: impl Into<String>) -> Server {
        Server {
            version: version.into(),
        }
    }

    /// Answers one JSON-RPC message: `Some` response for a request, `None`
    /// for a notification or a response, which need none.
    pub async fn handle<R: Rpc>(&self, rpc: &R, caller: &Caller, message: &Value) -> Option<Value> {
        let method = message.get("method").and_then(Value::as_str)?;
        let id = message.get("id")?.clone();
        let outcome = match method {
            "initialize" => parse_request(method, message).map(|r| self.initialize(&r)),
            "ping" => parse_request(method, message).map(|_| json!({})),
            "tools/list" => {
                parse_request(method, message).map(|_| json!({ "tools": crate::tools_list() }))
            }
            "tools/call" => match parse_request(method, message) {
                Ok(request) => Ok(call_tool(rpc, caller, &request["params"]).await),
                Err(err) => Err(err),
            },
            _ => Err(json!({ "code": METHOD_NOT_FOUND, "message": "Method not found" })),
        };
        Some(match outcome {
            Ok(result) => json!({ "result": result, "jsonrpc": "2.0", "id": id }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        })
    }

    fn initialize(&self, request: &Value) -> Value {
        let requested = request["params"]["protocolVersion"]
            .as_str()
            .unwrap_or_default();
        let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
            requested
        } else {
            LATEST_PROTOCOL_VERSION
        };
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": true } },
            "serverInfo": { "name": "vorn", "version": self.version }
        })
    }
}

/// `CallToolResult` for an error: what the SDK makes of anything a call throws.
fn tool_error(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// `tools/call`: the tool by name, its arguments checked, then run.
async fn call_tool<R: Rpc>(rpc: &R, caller: &Caller, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let Some(schema) = zod::registry().tool(name) else {
        return tool_error(format!("MCP error -32602: Tool {name} not found"));
    };
    let args = match schema {
        // No schema: the handler is called without looking at the arguments.
        None => None,
        Some(schema) => match zod::registry().parse(schema, params.get("arguments")) {
            Ok(parsed) => parsed,
            Err(issues) => {
                return tool_error(format!(
                "MCP error -32602: Input validation error: Invalid arguments for tool {name}: {}",
                zod::error_message(&issues)
            ))
            }
        },
    };
    let cx = Cx { rpc, caller };
    match tools::call(&cx, name, Args::new(args)).await {
        Ok(result) => normalize_result(result),
        Err(message) => tool_error(message),
    }
}

/// The result as the SDK hands it on after checking it against
/// `CallToolResultSchema`: `content` first, `isError` after.
fn normalize_result(result: Value) -> Value {
    let Value::Object(mut map) = result else {
        return result;
    };
    let mut out = Map::new();
    if let Some(meta) = map.shift_remove("_meta") {
        out.insert("_meta".into(), meta);
    }
    out.insert(
        "content".into(),
        map.shift_remove("content").unwrap_or_else(|| json!([])),
    );
    for key in ["structuredContent", "isError"] {
        if let Some(v) = map.shift_remove(key) {
            out.insert(key.into(), v);
        }
    }
    out.extend(map);
    Value::Object(out)
}

#[cfg(test)]
mod tests;
