//! The transport's answers, status by status, over a fake Vorn server.

use serde_json::{json, Value};

use super::*;
use crate::tools::fake::{block_on, caller, sample_config, FakeRpc};

const ACCEPT: &str = "application/json, text/event-stream";

struct Harness {
    rpc: FakeRpc,
    transport: Transport,
}

impl Harness {
    fn new() -> Harness {
        Harness {
            rpc: FakeRpc::new(sample_config()),
            transport: Transport::new(Server::new("0.0.0")),
        }
    }

    fn send(&self, req: Request<'_>) -> Response {
        block_on(self.transport.handle(&self.rpc, &caller(), req))
    }

    fn post(&self, session: Option<&str>, body: &Value) -> Response {
        let body = body.to_string();
        self.send(Request {
            method: "POST",
            accept: Some(ACCEPT),
            content_type: Some("application/json"),
            session_id: session,
            protocol_version: None,
            body: body.as_bytes(),
        })
    }

    /// Opens a session and returns its id.
    fn open(&self) -> String {
        let response = self.post(None, &initialize(1));
        assert_eq!(response.status, 200, "{response:?}");
        header(&response, "mcp-session-id")
            .expect("a session id")
            .to_owned()
    }
}

fn initialize(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    })
}

fn header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.as_str())
}

fn body(response: &Response) -> Value {
    serde_json::from_str(response.body.as_deref().expect("a body")).expect("a JSON body")
}

/// The status and JSON-RPC error code of a refusal.
fn refusal(response: &Response) -> (u16, i64) {
    (
        response.status,
        body(response)["error"]["code"].as_i64().unwrap(),
    )
}

#[test]
fn a_session_opens_answers_and_closes() {
    let h = Harness::new();
    let id = h.open();
    assert_eq!(h.transport.session_count(), 1);

    let listed = h.post(
        Some(&id),
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    assert_eq!(listed.status, 200);
    assert_eq!(header(&listed, "content-type"), Some("application/json"));
    assert_eq!(header(&listed, "mcp-session-id"), Some(id.as_str()));
    assert_eq!(body(&listed)["id"], 2);

    let closed = h.send(Request {
        method: "DELETE",
        session_id: Some(&id),
        ..Request::default()
    });
    assert_eq!(
        closed,
        Response {
            status: 200,
            headers: Vec::new(),
            body: None
        }
    );
    assert_eq!(h.transport.session_count(), 0);

    let after = h.post(
        Some(&id),
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }),
    );
    assert_eq!(refusal(&after), (404, -32001));
}

#[test]
fn notifications_alone_are_accepted_without_a_body() {
    let h = Harness::new();
    let id = h.open();
    let response = h.post(
        Some(&id),
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    assert_eq!(
        response,
        Response {
            status: 202,
            headers: Vec::new(),
            body: None
        }
    );
}

#[test]
fn a_batch_is_answered_with_one_response_per_request() {
    let h = Harness::new();
    let id = h.open();
    let response = h.post(
        Some(&id),
        &json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/initialized" },
            { "jsonrpc": "2.0", "id": 2, "method": "ping" }
        ]),
    );
    assert_eq!(response.status, 200);
    let answers = body(&response);
    assert_eq!(answers.as_array().map(Vec::len), Some(2));
    assert_eq!(answers[1]["id"], 2);
}

#[test]
fn headers_are_checked_before_the_body() {
    let h = Harness::new();
    let wrong_accept = h.send(Request {
        method: "POST",
        accept: Some("application/json"),
        content_type: Some("application/json"),
        body: b"{}",
        ..Request::default()
    });
    assert_eq!(refusal(&wrong_accept), (406, -32000));

    let wrong_type = h.send(Request {
        method: "POST",
        accept: Some(ACCEPT),
        content_type: Some("text/plain"),
        body: b"{}",
        ..Request::default()
    });
    assert_eq!(refusal(&wrong_type), (415, -32000));
}

#[test]
fn bodies_must_be_json_rpc() {
    let h = Harness::new();
    let not_json = h.send(Request {
        method: "POST",
        accept: Some(ACCEPT),
        content_type: Some("application/json"),
        body: b"{nope",
        ..Request::default()
    });
    assert_eq!(refusal(&not_json), (400, -32700));
    assert_eq!(
        body(&not_json)["error"]["message"],
        "Parse error: Invalid JSON"
    );

    for bad in [
        json!({ "jsonrpc": "1.0", "id": 1, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1.5, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "extra": true }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "ping", "params": [] }),
        json!(5),
    ] {
        let response = h.post(None, &bad);
        assert_eq!(refusal(&response), (400, -32700), "{bad}");
        assert_eq!(
            body(&response)["error"]["message"],
            "Parse error: Invalid JSON-RPC message"
        );
    }
}

#[test]
fn initialize_opens_exactly_one_session() {
    let h = Harness::new();
    let id = h.open();
    assert_eq!(refusal(&h.post(Some(&id), &initialize(2))), (400, -32600));
    assert_eq!(
        refusal(&h.post(Some("gone"), &initialize(2))),
        (404, -32001)
    );
    assert_eq!(
        refusal(&h.post(None, &json!([initialize(1), initialize(2)]))),
        (400, -32600)
    );
    assert_eq!(h.transport.session_count(), 1);
}

#[test]
fn later_requests_need_a_known_session_and_version() {
    let h = Harness::new();
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    let missing = h.post(None, &ping);
    assert_eq!(refusal(&missing), (400, -32000));
    assert_eq!(
        body(&missing)["error"]["message"],
        "Bad Request: Mcp-Session-Id header is required"
    );
    assert_eq!(refusal(&h.post(Some("unknown"), &ping)), (404, -32001));

    let id = h.open();
    let body_text = ping.to_string();
    let request = |version| Request {
        method: "POST",
        accept: Some(ACCEPT),
        content_type: Some("application/json"),
        session_id: Some(&id),
        protocol_version: Some(version),
        body: body_text.as_bytes(),
    };
    assert_eq!(h.send(request("2025-03-26")).status, 200);
    assert_eq!(refusal(&h.send(request("1999-01-01"))), (400, -32000));
}

#[test]
fn other_methods_are_not_allowed() {
    let h = Harness::new();
    let response = h.send(Request {
        method: "GET",
        ..Request::default()
    });
    assert_eq!(refusal(&response), (405, -32000));
    assert_eq!(header(&response, "allow"), Some("POST, DELETE"));
}

#[test]
fn the_least_recently_used_session_makes_room() {
    let h = Harness::new();
    let first = h.open();
    let second = h.open();
    for _ in 2..MAX_SESSIONS {
        h.open();
    }
    // Using the first leaves the second as the oldest.
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    assert_eq!(h.post(Some(&first), &ping).status, 200);
    h.open();
    assert_eq!(h.transport.session_count(), MAX_SESSIONS);
    assert_eq!(h.post(Some(&first), &ping).status, 200);
    assert_eq!(refusal(&h.post(Some(&second), &ping)), (404, -32001));
}
