//! `/mcp` end to end: a vornd with the `mcp` group native, in front of a
//! stand-in server that answers `config:load` over `/ws`. A tool call goes
//! from HTTP into the MCP server, out through vornd's own `/ws` to the
//! stand-in, and back.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use vornd::{Daemon, Groups};

const TOKEN: &str = "local-secret";

/// A server that accepts every WebSocket and answers `config:load` with one
/// project; it counts the sockets it was asked to open.
async fn stand_in() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let sockets = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&sockets);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let counted = Arc::clone(&counted);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                counted.fetch_add(1, Ordering::SeqCst);
                // A broadcast first, which the tools must not take for an answer.
                let _ = ws
                    .send(Message::text(
                        r#"{"jsonrpc":"2.0","method":"task:changed","params":{}}"#,
                    ))
                    .await;
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let frame: Value = serde_json::from_str(text.as_str()).unwrap();
                    let Some(id) = frame.get("id").cloned() else {
                        continue;
                    };
                    let answer = match frame["method"].as_str() {
                        Some("config:load") => json!({ "jsonrpc": "2.0", "id": id, "result": {
                            "version": 1,
                            "projects": [{ "name": "app", "path": "/work/app", "preferredAgents": ["claude"] }],
                            "tasks": [], "workflows": [], "workspaces": []
                        } }),
                        other => json!({ "jsonrpc": "2.0", "id": id, "error": {
                            "code": -32601, "message": format!("Method not found: {}", other.unwrap_or_default())
                        } }),
                    };
                    let _ = ws.send(Message::text(answer.to_string())).await;
                }
            });
        }
    });
    (addr, sockets)
}

/// A vornd in front of `upstream` with `groups`, listening on loopback.
async fn vornd(upstream: SocketAddr, groups: &str, token: bool) -> (SocketAddr, Arc<Daemon>) {
    let daemon = Daemon::new(upstream, Groups::parse(groups).unwrap());
    if token {
        daemon.set_desktop_token(TOKEN.as_bytes().to_vec());
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    daemon.set_listen_addr(addr);
    tokio::spawn(vornd::serve(
        listener,
        Arc::clone(&daemon),
        std::future::pending(),
    ));
    (addr, daemon)
}

struct Answer {
    status: StatusCode,
    session: Option<String>,
    body: Value,
}

async fn post(addr: SocketAddr, headers: &[(&str, &str)], body: Value) -> Answer {
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let mut req = Request::post(format!("http://{addr}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json");
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let res = client
        .request(req.body(Full::new(Bytes::from(body.to_string()))).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let session = res
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Answer {
        status,
        session,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    })
}

const BEARER: &str = "Bearer local-secret";

#[tokio::test]
async fn a_tool_call_reaches_the_server_through_vornds_own_socket() {
    let (upstream, sockets) = stand_in().await;
    let (addr, daemon) = vornd(upstream, "mcp=native", true).await;

    let opened = post(addr, &[("authorization", BEARER)], initialize()).await;
    assert_eq!(opened.status, StatusCode::OK);
    assert_eq!(opened.body["result"]["serverInfo"]["name"], "vorn");
    let session = opened.session.expect("a session id");

    let auth = [
        ("authorization", BEARER),
        ("mcp-session-id", session.as_str()),
    ];
    for id in 2..4 {
        let listed = post(
            addr,
            &auth,
            json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": "list_projects", "arguments": {} } }),
        )
        .await;
        assert_eq!(listed.status, StatusCode::OK);
        let text = listed.body["result"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("\"/work/app\""), "{text}");
    }
    assert_eq!(
        sockets.load(Ordering::SeqCst),
        1,
        "one socket serves every call"
    );

    let counts = daemon.groups().counts();
    assert_eq!(counts["mcp"].native, 3);
}

#[tokio::test]
async fn only_an_agent_with_the_local_credential_gets_in() {
    let (upstream, _) = stand_in().await;
    let (addr, _) = vornd(upstream, "mcp=native", true).await;
    let with_origin = post(
        addr,
        &[("authorization", BEARER), ("origin", "http://127.0.0.1")],
        initialize(),
    )
    .await;
    assert_eq!(with_origin.status, StatusCode::FORBIDDEN);
    assert_eq!(
        post(addr, &[], initialize()).await.status,
        StatusCode::UNAUTHORIZED
    );
    let wrong = post(
        addr,
        &[("authorization", "Bearer local-secreT")],
        initialize(),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let (untold, _) = vornd(upstream, "mcp=native", false).await;
    let refused = post(untold, &[("authorization", BEARER)], initialize()).await;
    assert_eq!(refused.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn forward_and_shadow_leave_the_path_to_the_server() {
    let (upstream, _) = stand_in().await;
    for (mode, shadowed) in [("forward", 0), ("shadow", 1)] {
        let (addr, daemon) = vornd(upstream, &format!("mcp={mode}"), true).await;
        let answer = post(addr, &[("authorization", BEARER)], initialize()).await;
        assert_ne!(
            answer.status,
            StatusCode::OK,
            "{mode}: the stand-in has no /mcp"
        );
        let counts = daemon.groups().counts();
        assert_eq!(counts["mcp"].forwarded, 1, "{mode}");
        assert_eq!(counts["mcp"].shadow_unported, shadowed, "{mode}");
        assert_eq!(counts["mcp"].native, 0, "{mode}");
    }
}
