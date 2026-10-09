//! `/mcp` end to end: a tool call goes from HTTP out through vornd's own `/ws` and back.

mod common;

use std::net::SocketAddr;

use bytes::Bytes;
use common::{connect, next, send, serve, CREDENTIAL};
use http_body_util::{BodyExt, Full};
use hyper::{Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{json, Value};

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

const BEARER: &str = "Bearer vornd-test-credential";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_call_comes_back_through_vornds_own_socket() {
    let served = serve();
    let addr = SocketAddr::from(([127, 0, 0, 1], served.port));
    let mut desktop = connect(served.port, CREDENTIAL).await;
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "config:load" }),
    )
    .await;
    let mut config = next(&mut desktop).await["result"].take();
    config["projects"] =
        json!([{ "name": "app", "path": "/work/app", "preferredAgents": ["claude"] }]);
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "config:save", "params": config }),
    )
    .await;
    assert!(next(&mut desktop).await.get("error").is_none());

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
    assert_eq!(health(addr).await["mcp"]["sessions"], 1);
}

/// vornd's health report.
async fn health(addr: SocketAddr) -> Value {
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let res = client
        .get(format!("http://{addr}/vornd/health").parse().unwrap())
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_an_agent_with_the_local_credential_gets_in() {
    let served = serve();
    let addr = SocketAddr::from(([127, 0, 0, 1], served.port));
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
        &[("authorization", "Bearer vornd-test-credentiaL")],
        initialize(),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
}
