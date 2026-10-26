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
api = "vorn:extension@1.0"

[service]
command = ["node", "program.mjs"]

[declares]
network = ["api.example.com"]
uses = ["sessions.read", "storage", "workflows.read"]

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
        self.relay_as("test.rules", method, params).await
    }

    /// What `test.flows` got back when it called `method`.
    async fn flows(&mut self, method: &str, args: Value) -> Value {
        let params = json!({ "method": method, "args": args });
        self.relay_as("test.flows", "host.call", params).await
    }

    /// [`Ws::relay`] through the program of extension `id`.
    async fn relay_as(&mut self, id: &str, method: &str, params: Value) -> Value {
        let args = json!({ "method": method, "params": params, "notification": false });
        let answer = self.call(&format!("{id}:relay"), args).await;
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

    // The calls the shipped editors make are public, held to the same declared permissions.
    let listed = c
        .relay(
            "host.call",
            json!({ "method": "workflow.list", "args": {} }),
        )
        .await;
    assert!(listed.get("error").is_none(), "{listed}");
    let refused = c
        .relay(
            "host.call",
            json!({ "method": "workflow.unseed", "args": {} }),
        )
        .await;
    assert_eq!(
        refused["error"]["message"],
        "test.rules does not declare workflows.write, which workflow.unseed needs"
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
    let own: Vec<_> = tables(&db)
        .into_iter()
        .filter(|t| t.starts_with("x_test_0rules_"))
        .collect();
    assert!(own.is_empty(), "{own:?}");

    // Linked again, its migration runs again and its table is back.
    let linked = c
        .call("extension:link", json!({ "path": folder.path() }))
        .await;
    assert!(linked.get("error").is_none(), "{linked}");
    let insert =
        json!({ "sql": "INSERT INTO x_test_0rules_notes (body) VALUES (?)", "params": ["again"] });
    assert_eq!(c.relay("storage.exec", insert.clone()).await["result"], 1);

    // A migration put in as a link is not run, and its tables are not used until it is gone.
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(
            outside.path().join("more.sql"),
            "CREATE TABLE x_test_0rules_more (a);",
        )
        .unwrap();
        let planted = folder.path().join("migrations/002_more.sql");
        std::os::unix::fs::symlink(outside.path().join("more.sql"), &planted).unwrap();
        let refused = c.relay("storage.exec", insert.clone()).await;
        let message = refused["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("migrations/002_more.sql is a link"),
            "{refused}"
        );
        assert!(!tables(&db).contains(&"x_test_0rules_more".to_owned()));
        std::fs::remove_file(&planted).unwrap();
        assert_eq!(c.relay("storage.exec", insert).await["result"], 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extension_seeds_runs_and_removes_only_its_own_workflows() {
    if !node() {
        return;
    }
    let served = serve();
    let mut c = Ws {
        ws: connect(served.port, CREDENTIAL).await,
        next_id: 0,
    };
    let folder = tempfile::tempdir().unwrap();
    let manifest = MANIFEST
        .replace("test.rules", "test.flows")
        .replace(
            "\"storage\", \"workflows.read\"",
            "\"workflows.read\", \"workflows.write\"",
        )
        .replace("[storage]\nmigrations = \"migrations\"\n", "");
    std::fs::write(folder.path().join("vorn-extension.toml"), manifest).unwrap();
    std::fs::write(folder.path().join("program.mjs"), PROGRAM).unwrap();
    let linked = c
        .call("extension:link", json!({ "path": folder.path() }))
        .await;
    assert!(linked.get("error").is_none(), "{linked}");
    let workflow = |id: &str, step: &str| {
        json!({ "id": id, "name": id, "icon": "Plug", "iconColor": "#fff", "enabled": false,
                "edges": [], "nodes": [{ "id": "t", "type": "trigger", "config": {} },
                                      { "id": "s", "type": step, "config": {} }] })
    };
    let refused = |answer: &Value| answer["error"]["message"].as_str().unwrap_or("").to_owned();

    // Someone else's workflows are not its to seed, remove or run.
    let theirs = workflow("import:bundle:a", "condition");
    let answer = c
        .flows("workflow.seed", json!({ "workflows": [theirs] }))
        .await;
    assert_eq!(
        refused(&answer),
        "workflow.seed: import:bundle:a is not test.flows's; its workflows' ids start with test.flows:"
    );
    let answer = c
        .flows("workflow.unseed", json!({ "prefix": "import:" }))
        .await;
    assert_eq!(
        refused(&answer),
        "workflow.unseed: import: is not test.flows's; its workflows' ids start with test.flows:"
    );
    let answer = c
        .flows("workflow.run", json!({ "workflowId": "wf-theirs" }))
        .await;
    assert_eq!(
        refused(&answer),
        "test.flows does not declare process.run, which workflow.run needs for wf-theirs, a workflow not its own"
    );

    // Its own, but a step that runs commands or an agent needs process.run.
    for step in ["script", "launchAgent", "nobodyDeclaresThis"] {
        let answer = c
            .flows(
                "workflow.seed",
                json!({ "workflows": [workflow("test.flows:x", step)] }),
            )
            .await;
        assert_eq!(
            refused(&answer),
            format!("test.flows does not declare process.run, which workflow.seed needs for test.flows:x: its {step} step runs commands")
        );
    }

    // What is its own and runs nothing is seeded, and removed again.
    let answer = c
        .flows(
            "workflow.seed",
            json!({ "workflows": [workflow("test.flows:y", "condition")] }),
        )
        .await;
    assert_eq!(answer["result"], json!({ "created": [true] }), "{answer}");
    let answer = c
        .flows("workflow.unseed", json!({ "prefix": "test.flows:" }))
        .await;
    assert_eq!(answer["result"], json!({ "removed": 1 }), "{answer}");
}
