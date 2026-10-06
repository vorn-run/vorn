//! MCP's Streamable HTTP transport, server side, answering in JSON.
//!
//! Follows the SDK's `WebStandardStreamableHTTPServerTransport` with
//! `enableJsonResponse`: a POST carries one JSON-RPC message or a batch and is
//! answered with the responses as `application/json`, or `202` when it held no
//! request; `initialize` opens a session whose id every later request must
//! carry in `Mcp-Session-Id`; `DELETE` ends it. There are no server-sent
//! events, so `GET`, which only opens a stream, is not allowed.
//!
//! The host reads the HTTP request and decides who may make it; this module
//! only answers it, so it works over any HTTP stack.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::json as js;
use crate::rpc::{Caller, Rpc};
use crate::server::{is_initialize_request, Server, SUPPORTED_PROTOCOL_VERSIONS};

/// Sessions kept at once. A client that never sends `DELETE` leaves its
/// session behind, so the least recently used one makes room for a new one.
pub const MAX_SESSIONS: usize = 256;

/// What the transport reads of an HTTP request.
#[derive(Clone, Copy, Debug, Default)]
pub struct Request<'a> {
    /// `GET`, `POST`, `DELETE`, ...
    pub method: &'a str,
    pub accept: Option<&'a str>,
    pub content_type: Option<&'a str>,
    /// `Mcp-Session-Id`.
    pub session_id: Option<&'a str>,
    /// `MCP-Protocol-Version`.
    pub protocol_version: Option<&'a str>,
    pub body: &'a [u8],
}

/// The answer, for the host to write out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    /// `None` for an empty body.
    pub body: Option<String>,
}

impl Response {
    fn json(status: u16, body: &Value, session: Option<&str>) -> Response {
        let mut headers = vec![("content-type", "application/json".to_owned())];
        if let Some(id) = session {
            headers.push(("mcp-session-id", id.to_owned()));
        }
        Response {
            status,
            headers,
            body: Some(js::stringify(body)),
        }
    }

    /// `createJsonErrorResponse(status, code, message)`.
    fn error(status: u16, code: i64, message: &str) -> Response {
        Response::json(
            status,
            &json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null }),
            None,
        )
    }

    fn empty(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: None,
        }
    }
}

/// Open sessions, by id, with when each was last used.
#[derive(Default)]
struct Sessions {
    last_used: HashMap<String, u64>,
    clock: u64,
}

impl Sessions {
    fn touch(&mut self, id: &str) -> bool {
        self.clock += 1;
        match self.last_used.get_mut(id) {
            Some(at) => {
                *at = self.clock;
                true
            }
            None => false,
        }
    }

    fn open(&mut self) -> String {
        if self.last_used.len() >= MAX_SESSIONS {
            if let Some(oldest) = self
                .last_used
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(id, _)| id.clone())
            {
                self.last_used.remove(&oldest);
            }
        }
        self.clock += 1;
        let id = uuid::Uuid::new_v4().to_string();
        self.last_used.insert(id.clone(), self.clock);
        id
    }
}

/// The transport: a server and its sessions.
pub struct Transport {
    server: Server,
    sessions: Mutex<Sessions>,
}

impl Transport {
    pub fn new(server: Server) -> Transport {
        Transport {
            server,
            sessions: Mutex::new(Sessions::default()),
        }
    }

    /// How many sessions are open.
    pub fn session_count(&self) -> usize {
        self.lock().last_used.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Sessions> {
        // A panic elsewhere cannot leave the map half-written: every change is
        // one insert or remove.
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Answers one HTTP request.
    pub async fn handle<R: Rpc>(&self, rpc: &R, caller: &Caller, req: Request<'_>) -> Response {
        match req.method {
            "POST" => self.post(rpc, caller, req).await,
            "DELETE" => self.delete(req),
            _ => {
                let mut response = Response::error(405, -32000, "Method not allowed.");
                response
                    .headers
                    .insert(0, ("allow", "POST, DELETE".to_owned()));
                response
            }
        }
    }

    /// `validateSession` then `validateProtocolVersion`, for anything but `initialize`.
    fn check_session(&self, req: &Request<'_>) -> Result<String, Response> {
        let Some(id) = req.session_id.filter(|s| !s.is_empty()) else {
            return Err(Response::error(
                400,
                -32000,
                "Bad Request: Mcp-Session-Id header is required",
            ));
        };
        if !self.lock().touch(id) {
            return Err(Response::error(404, -32001, "Session not found"));
        }
        check_protocol_version(req)?;
        Ok(id.to_owned())
    }

    fn delete(&self, req: Request<'_>) -> Response {
        match self.check_session(&req) {
            Ok(id) => {
                self.lock().last_used.remove(&id);
                Response::empty(200)
            }
            Err(response) => response,
        }
    }

    async fn post<R: Rpc>(&self, rpc: &R, caller: &Caller, req: Request<'_>) -> Response {
        let accept = req.accept.unwrap_or_default();
        if !accept.contains("application/json") || !accept.contains("text/event-stream") {
            return Response::error(
                406,
                -32000,
                "Not Acceptable: Client must accept both application/json and text/event-stream",
            );
        }
        if !req
            .content_type
            .is_some_and(|ct| ct.contains("application/json"))
        {
            return Response::error(
                415,
                -32000,
                "Unsupported Media Type: Content-Type must be application/json",
            );
        }
        let Ok(raw) = serde_json::from_slice::<Value>(req.body) else {
            return Response::error(400, -32700, "Parse error: Invalid JSON");
        };
        // The TypeScript only ever sees the body as `JSON.parse` left it.
        let raw = js::js_order(raw);
        let messages = match raw {
            Value::Array(items) => items,
            one => vec![one],
        };
        if !messages.iter().all(is_message) {
            return Response::error(400, -32700, "Parse error: Invalid JSON-RPC message");
        }

        let session = if messages.iter().any(is_initialize_request) {
            if let Some(id) = req.session_id.filter(|s| !s.is_empty()) {
                if self.lock().touch(id) {
                    return Response::error(
                        400,
                        -32600,
                        "Invalid Request: Server already initialized",
                    );
                }
                return Response::error(404, -32001, "Session not found");
            }
            if messages.len() > 1 {
                return Response::error(
                    400,
                    -32600,
                    "Invalid Request: Only one initialization request is allowed",
                );
            }
            self.lock().open()
        } else {
            match self.check_session(&req) {
                Ok(id) => id,
                Err(response) => return response,
            }
        };

        if !messages.iter().any(is_request) {
            return Response::empty(202);
        }
        let mut responses = Vec::new();
        for message in &messages {
            if let Some(response) = self.server.handle(rpc, caller, message).await {
                responses.push(response);
            }
        }
        let body = match responses.len() {
            1 => responses.pop().unwrap_or(Value::Null),
            _ => Value::Array(responses),
        };
        Response::json(200, &body, Some(&session))
    }
}

fn check_protocol_version(req: &Request<'_>) -> Result<(), Response> {
    match req.protocol_version {
        Some(version) if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) => Err(Response::error(
            400,
            -32000,
            &format!(
                "Bad Request: Unsupported protocol version: {version} (supported versions: {})",
                SUPPORTED_PROTOCOL_VERSIONS.join(", ")
            ),
        )),
        _ => Ok(()),
    }
}

/// `RequestIdSchema`: a string or an integer.
fn is_id(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(n) => n
            .as_f64()
            .is_some_and(|f| f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0),
        _ => false,
    }
}

/// `RequestMetaSchema`, the only part of a message's params the transport checks.
fn is_meta(meta: &Value) -> bool {
    let Value::Object(map) = meta else {
        return false;
    };
    let token_ok = map.get("progressToken").is_none_or(is_id);
    let task_ok = map
        .get("io.modelcontextprotocol/related-task")
        .is_none_or(|t| t.get("taskId").is_some_and(Value::is_string) && t.is_object());
    token_ok && task_ok
}

/// Params of a request or notification: an object whose `_meta` is well formed.
fn is_params(params: &Value) -> bool {
    params.is_object() && params.get("_meta").is_none_or(is_meta)
}

fn only_keys(map: &serde_json::Map<String, Value>, allowed: &[&str]) -> bool {
    map.keys().all(|k| allowed.contains(&k.as_str()))
}

/// `isJSONRPCRequest`.
fn is_request(message: &Value) -> bool {
    let Value::Object(map) = message else {
        return false;
    };
    map.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && only_keys(map, &["jsonrpc", "id", "method", "params"])
        && map.get("id").is_some_and(is_id)
        && map.get("method").is_some_and(Value::is_string)
        && map.get("params").is_none_or(is_params)
}

/// `JSONRPCMessageSchema`: a request, a notification, a result or an error.
fn is_message(message: &Value) -> bool {
    let Value::Object(map) = message else {
        return false;
    };
    if map.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return false;
    }
    let notification = only_keys(map, &["jsonrpc", "method", "params"])
        && map.get("method").is_some_and(Value::is_string)
        && map.get("params").is_none_or(is_params);
    let result = only_keys(map, &["jsonrpc", "id", "result"])
        && map.get("id").is_some_and(is_id)
        && map
            .get("result")
            .is_some_and(|r| r.is_object() && r.get("_meta").is_none_or(is_meta));
    let error = only_keys(map, &["jsonrpc", "id", "error"])
        && map.get("id").is_none_or(is_id)
        && map.get("error").is_some_and(|e| {
            e.is_object()
                && e.get("code").is_some_and(|c| {
                    c.as_f64()
                        .is_some_and(|f| f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0)
                })
                && e.get("message").is_some_and(Value::is_string)
        });
    is_request(message) || notification || result || error
}

#[cfg(test)]
mod tests;
