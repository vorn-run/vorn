//! Connectors as a built-in, against the real vornd: a connector installs
//! from a git repository with its declared hosts on the sheet, runs an
//! action and polls into the workflows' inbox through the bridge, backfills
//! the board through the board's own method, adds its tools to Vorn's MCP
//! server under their old names, and with its switch off answers nothing
//! and lists no tools while every connection stays. Skipped without node or git.

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use bytes::Bytes;
use common::{connect, next, send, serve, Socket, CREDENTIAL};
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("../../../../../tests/fixtures/native-connector.mjs");

const MANIFEST: &str = r#"id = "tickets"
name = "Tickets"
version = "1.0.0"
api = "vorn:extension@0.9"
publisher = "Vorn tests"

[service]
command = ["node", "index.js"]

[declares]
network = ["tickets.example.com"]

[connector.auth]
rung = "none"

[[connector.triggers]]
type = "items"
label = "Items"

[[connector.actions]]
type = "create"
label = "Create"

[[connector.actions.inputs]]
key = "title"

[[contributes.sidebar]]
id = "queue"
title = "Queue"
tree = true

[[contributes.commands]]
id = "refresh"
title = "Refresh Tickets"
call = "refresh"
"#;

fn has(program: &str) -> bool {
    let found = Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipped: no {program} on PATH");
    }
    found
}

/// A repository holding the connector, committed.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    std::fs::write(dir.path().join("index.js"), FIXTURE).unwrap();
    std::fs::write(dir.path().join("package.json"), r#"{"type":"module"}"#).unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "init"]);
    dir
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

    async fn result(&mut self, method: &str, params: Value) -> Value {
        let answer = self.call(method, params).await;
        assert!(answer.get("error").is_none(), "{method}: {answer}");
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

/// The names of the tools an agent sees.
async fn tool_names(addr: SocketAddr) -> (String, Vec<String>) {
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
    let names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    (session, names)
}

fn rows(db: &Path, sql: &str) -> Vec<Vec<String>> {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn.prepare(sql).unwrap();
    let n = stmt.column_count();
    let out = stmt
        .query_map([], |r| {
            (0..n)
                .map(|i| {
                    r.get::<_, rusqlite::types::Value>(i).map(|v| match v {
                        rusqlite::types::Value::Text(t) => t,
                        rusqlite::types::Value::Integer(i) => i.to_string(),
                        rusqlite::types::Value::Null => "null".into(),
                        other => format!("{other:?}"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connectors_run_as_a_built_in_and_keep_their_data_when_off() {
    if !has("node") || !has("git") {
        return;
    }
    let served = serve();
    let addr = SocketAddr::from(([127, 0, 0, 1], served.port));
    let db = served.data.path().join("vorn.db");
    let mut c = Ws {
        ws: connect(served.port, CREDENTIAL).await,
        next_id: 0,
    };
    let repo = repository();
    let url = format!("file://{}", repo.path().display());

    // From git, with what it reaches on the sheet; installed by the click on it.
    let inspected = c
        .result(
            "connector:inspectPack",
            json!({ "kind": "git", "url": url }),
        )
        .await;
    assert_eq!(inspected["ok"], true, "{inspected}");
    let preview = &inspected["preview"];
    assert_eq!(preview["id"], "tickets");
    assert_eq!(preview["hosts"], json!(["tickets.example.com"]));
    assert_eq!(preview["triggers"][0]["type"], "items");
    let installed = c
        .result(
            "connector:installPack",
            json!({ "kind": "staged", "token": preview["token"] }),
        )
        .await;
    assert_eq!(installed["ok"], true, "{installed}");
    assert_eq!(installed["pack"]["source"]["git"], url);

    let conn = c
        .result(
            "connection:create",
            json!({ "connectorId": "sdk", "name": "Tickets", "syncIntervalMinutes": 0, "statusMapping": {},
                    "executionProject": "app",
                    "filters": { "sdkConnectorId": "tickets", "sdkVersion": "1.0.0", "sdkTrigger": "items" } }),
        )
        .await;
    let id = conn["id"].as_str().unwrap().to_owned();
    let acted = c
        .result(
            "connection:executeAction",
            json!({ "connectionId": id, "action": "create", "args": { "title": "T" } }),
        )
        .await;
    assert_eq!(acted["success"], true, "{acted}");

    // The same package draws a sidebar from a template, and its command
    // calls the program, started as an extension beside its connection.
    let points = c.result("extension:contributions", json!({})).await;
    let ours: Vec<&Value> = points
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["extension"] == "tickets")
        .collect();
    assert!(
        ours.iter().any(|p| p["kind"] == "sidebar"
            && p["id"] == "queue"
            && p["errors"].as_array().is_none_or(Vec::is_empty)),
        "{points}"
    );
    assert!(ours.iter().any(|p| p["kind"] == "commands"), "{points}");
    let refreshed = c
        .result(
            "extension:call",
            json!({ "id": "tickets", "action": "refresh", "args": {} }),
        )
        .await;
    assert_eq!(refreshed["toast"], "Refreshed", "{refreshed}");
    let queue = c
        .result(
            "extension:rows",
            json!({ "id": "tickets", "collection": "queue" }),
        )
        .await;
    assert_eq!(queue.as_array().map(Vec::len), Some(2), "{queue}");

    // A poll fills the workflow's inbox through the bridge.
    let workflow = json!({
        "id": "wf-poll", "name": "Poll", "icon": "Zap", "iconColor": "#ffffff", "enabled": false,
        "nodes": [{ "id": "t", "type": "trigger", "label": "Trigger", "position": { "x": 0, "y": 0 },
                    "config": { "triggerType": "connectorPoll", "connectionId": id, "event": "mcpPoll", "cron": "0 0 1 1 *" } }],
        "edges": []
    });
    c.result("workflow:create", json!({ "workflow": workflow }))
        .await;
    assert_eq!(
        c.result("connector:poll", json!({ "workflowId": "wf-poll" }))
            .await,
        json!({ "pages": 1 })
    );
    let inbox = rows(&db, "SELECT workflow_id FROM connector_inbox");
    assert!(!inbox.is_empty(), "the poll left nothing in the inbox");

    // Backfill puts the items on the board, linked to where they came from.
    let backfilled = c
        .result("connection:backfill", json!({ "connectionId": id }))
        .await;
    assert_eq!(backfilled["imported"], 1, "{backfilled}");
    let tasks = rows(
        &db,
        "SELECT t.title, t.source_connector_id, l.connection_id FROM tasks t JOIN task_source_links l ON l.task_id = t.id",
    );
    assert_eq!(
        tasks,
        [["One".to_owned(), "tickets".to_owned(), id.clone()]]
    );

    // Its tools are an agent's, under the names they always had.
    let (session, names) = tool_names(addr).await;
    assert!(names.iter().any(|n| n == "list_connections"), "{names:?}");
    let (_, called) = mcp(
        addr,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_connections", "arguments": {} } }),
    )
    .await;
    let text = called["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&id), "{called}");

    // Off: nothing answers, no tool is listed, nothing polls; the data stays.
    let listed = c.result("extension:builtins", json!({})).await;
    let ours = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "vorn.connectors")
        .cloned()
        .unwrap_or_else(|| panic!("Connectors is not among the built-ins: {listed}"));
    assert_eq!(ours["on"], true);
    c.result(
        "extension:setBuiltin",
        json!({ "id": "vorn.connectors", "on": false }),
    )
    .await;
    let off = c.call("connection:list", json!({})).await;
    assert_eq!(
        off["error"]["message"],
        "Connectors is off. Turn it on in Settings → Extensions."
    );
    let off = c
        .call("connector:poll", json!({ "workflowId": "wf-poll" }))
        .await;
    assert!(off.get("error").is_some(), "{off}");
    let (_, names) = tool_names(addr).await;
    assert!(!names.iter().any(|n| n == "list_connections"), "{names:?}");
    let kept_rows = rows(&db, "SELECT id FROM source_connections");
    assert!(kept_rows.contains(&vec![id.clone()]), "{kept_rows:?}");
    assert_eq!(rows(&db, "SELECT count(*) FROM tasks"), [["1".to_owned()]]);
    let kept = std::fs::read_to_string(served.data.path().join("builtins.json")).unwrap();
    assert!(kept.contains("\"vorn.connectors\": false"), "{kept}");

    // On again, everything is where it was.
    c.result(
        "extension:setBuiltin",
        json!({ "id": "vorn.connectors", "on": true }),
    )
    .await;
    let back = c.result("connection:list", json!({})).await;
    assert!(
        back.as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id.as_str()),
        "{back}"
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
}

/// A feed on this machine, served until the test ends.
async fn feed_server() -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let body = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Local</title><link>http://localhost/</link><description>d</description><item><title>Sandboxed item</title><link>http://localhost/1</link><guid>1</guid><pubDate>Wed, 14 Oct 2026 10:00:00 GMT</pubDate></item></channel></rss>"#;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(body.as_bytes()).await;
            });
        }
    });
    port
}

/// The RSS connector, sandboxed, reads a feed its connection names through
/// the bridge's proxy. macOS, with `VORN_CONNECTORS_CHECKOUT` naming a
/// connectors checkout whose folders are built.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rss_connector_reads_its_feeds_under_the_sandbox() {
    let Some(root) = std::env::var_os("VORN_CONNECTORS_CHECKOUT").map(std::path::PathBuf::from)
    else {
        eprintln!("skipped: VORN_CONNECTORS_CHECKOUT names no connectors checkout");
        return;
    };
    if !cfg!(target_os = "macos") || !has("node") || !has("git") {
        eprintln!("skipped: the sandbox profile needs macOS, node and git");
        return;
    }
    let served = serve();
    let mut c = Ws {
        ws: connect(served.port, CREDENTIAL).await,
        next_id: 0,
    };
    let folder = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(root.join("rss")).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), folder.path().join(entry.file_name())).unwrap();
        }
    }
    for args in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "-A"],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "rss",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(folder.path())
            .status()
            .unwrap()
            .success());
    }
    let url = format!("file://{}", folder.path().display());
    let inspected = c
        .result(
            "connector:inspectPack",
            json!({ "kind": "git", "url": url }),
        )
        .await;
    assert_eq!(
        inspected["preview"]["hostsFrom"],
        json!(["RSS_FEEDS"]),
        "{inspected}"
    );
    let installed = c
        .result(
            "connector:installPack",
            json!({ "kind": "staged", "token": inspected["preview"]["token"] }),
        )
        .await;
    assert_eq!(installed["ok"], true, "{installed}");
    c.result("extension:setSandbox", json!({ "id": "rss", "on": true }))
        .await;

    let port = feed_server().await;
    let feed = format!("http://localhost:{port}/feed.xml");
    let conn = c
        .result(
            "connection:create",
            json!({ "connectorId": "sdk", "name": "Feeds", "syncIntervalMinutes": 0, "statusMapping": {},
                    "filters": { "sdkConnectorId": "rss", "sdkVersion": installed["pack"]["version"], "sdkTrigger": "newItem",
                                 "env": json!({ "RSS_FEEDS": feed }).to_string() } }),
        )
        .await;
    let read = c
        .result(
            "connection:executeAction",
            json!({ "connectionId": conn["id"], "action": "readFeed", "args": { "url": feed } }),
        )
        .await;
    assert_eq!(read["success"], true, "{read}");
    assert!(read.to_string().contains("Sandboxed item"), "{read}");

    // A host its connection does not name stays out of reach.
    let elsewhere = c
        .result(
            "connection:executeAction",
            json!({ "connectionId": conn["id"], "action": "readFeed", "args": { "url": format!("http://127.0.0.1:{port}/feed.xml") } }),
        )
        .await;
    assert_eq!(elsewhere["success"], false, "{elsewhere}");
}
