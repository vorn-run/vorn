//! The bridge's rules against the real vornd: a method or host the manifest
//! does not declare fails naming it, an extension's tables live in `vorn.db`
//! behind the authorizer and go when it is removed, and its MCP tool shows
//! in an agent's tool list and answers there. Skipped without `node`.

mod common;

use std::net::SocketAddr;

use bytes::Bytes;
use common::{connect, next, send, serve, Socket, CREDENTIAL};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{json, Value};

const PROGRAM: &str = include_str!("../../extensions/tests/fixtures/program.mjs");

const MANIFEST: &str = r#"id = "test.rules"
name = "Rules"
version = "1.0.0"
api = "vorn:extension@0.9"

[service]
command = ["node", "program.mjs"]

[declares]
network = ["api.example.com"]
uses = ["sessions.read", "storage"]

[storage]
migrations = "migrations"

[mcp.echo]
description = "Echoes its arguments"
"#;

fn node() -> bool {
    let found = std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipped: no node on PATH");
    }
    found
}

struct Ws {
    ws: Socket,
    next_id: u64,
}

impl Ws {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        send(
            &mut self.ws,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let frame = next(&mut self.ws).await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }

    /// What the program got back when it asked Vorn `method` with `params`.
    async fn relay(&mut self, method: &str, params: Value) -> Value {
        let args = json!({ "method": method, "params": params, "notification": false });
        let answer = self.call("test.rules:relay", args).await;
        assert!(answer.get("error").is_none(), "{answer}");
        answer["result"].clone()
    }
}

async fn mcp(addr: SocketAddr, session: Option<&str>, body: Value) -> (Option<String>, Value) {
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let mut req = Request::post(format!("http://{addr}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {CREDENTIAL}"));
    if let Some(s) = session {
        req = req
            .header("mcp-session-id", s)
            .header("mcp-protocol-version", "2025-06-18");
    }
    let res = client
        .request(req.body(Full::new(Bytes::from(body.to_string()))).unwrap())
        .await
        .unwrap();
    let session = res
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        session,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tables(db: &std::path::Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'x\\_%' ESCAPE '\\'",
        )
        .unwrap();
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_holds_a_program_to_what_it_declares() {
    if !node() {
        return;
    }
    let served = serve();
    let addr = SocketAddr::from(([127, 0, 0, 1], served.port));
    let mut c = Ws {
        ws: connect(served.port, CREDENTIAL).await,
        next_id: 0,
    };
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    std::fs::write(folder.path().join("program.mjs"), PROGRAM).unwrap();
    std::fs::create_dir(folder.path().join("migrations")).unwrap();
    std::fs::write(
        folder.path().join("migrations/001_notes.sql"),
        "CREATE TABLE x_test_0rules_notes (body TEXT);",
    )
    .unwrap();
    let linked = c
        .call("extension:link", json!({ "path": folder.path() }))
        .await;
    assert!(linked.get("error").is_none(), "{linked}");

    // An undeclared method and an undeclared host each fail, naming them.
    let refused = c
        .relay("host.call", json!({ "method": "git.status", "args": {} }))
        .await;
    assert_eq!(
        refused["error"]["message"],
        "test.rules does not declare git.read, which git.status needs"
    );
    let req =
        json!({ "method": "GET", "url": "https://evil.example/x", "headers": [], "body": null });
    let refused = c
        .relay("net.fetch", json!({ "req": req, "connection": null }))
        .await;
    assert_eq!(
        refused["error"]["message"],
        "test.rules does not declare the host evil.example"
    );

    // Its own tables, made by its migration, and nobody else's.
    let insert =
        json!({ "sql": "INSERT INTO x_test_0rules_notes (body) VALUES (?)", "params": ["hi"] });
    assert_eq!(c.relay("storage.exec", insert).await["result"], 1);
    let read = json!({ "sql": "SELECT body FROM x_test_0rules_notes", "params": [] });
    assert_eq!(
        c.relay("storage.query", read).await["result"],
        json!([{ "body": "hi" }])
    );
    let theirs = json!({ "sql": "SELECT * FROM tasks", "params": [] });
    let refused = c.relay("storage.query", theirs).await;
    assert_eq!(refused["error"]["data"]["code"], "denied", "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("test.rules may not read the table tasks"),
        "{refused}"
    );
    let db = served.data.path().join("vorn.db");
    assert!(tables(&db).contains(&"x_test_0rules_notes".to_owned()));

    // Its MCP tool is in an agent's tool list, and answers there.
    let init = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    });
    let (session, _) = mcp(addr, None, init).await;
    let session = session.expect("an MCP session");
    let (_, listed) = mcp(
        addr,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "test_rules__echo")
        .cloned()
        .unwrap_or_else(|| panic!("no extension tool in {listed}"));
    assert_eq!(
        tool["description"],
        "Echoes its arguments (from the extension Rules)"
    );
    let (_, called) = mcp(
        addr,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "test_rules__echo", "arguments": { "n": 7 } } }),
    )
    .await;
    let text = called["result"]["content"][0]["text"].as_str().unwrap();
    let echoed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(echoed, json!({ "echoed": { "n": 7 } }), "{called}");

    // Removed, it leaves no table behind.
    let removed = c
        .call("extension:remove", json!({ "id": "test.rules" }))
        .await;
    assert!(removed.get("error").is_none(), "{removed}");
    assert!(tables(&db).is_empty(), "{:?}", tables(&db));

    // Linked again, its migration runs again and its table is back.
    let linked = c
        .call("extension:link", json!({ "path": folder.path() }))
        .await;
    assert!(linked.get("error").is_none(), "{linked}");
    let insert =
        json!({ "sql": "INSERT INTO x_test_0rules_notes (body) VALUES (?)", "params": ["again"] });
    assert_eq!(c.relay("storage.exec", insert).await["result"], 1);
}
