//! `vorn mcp` against a fake Streamable HTTP `/mcp`: the relay on its own,
//! then the binary end to end with a fake server telling it where vornd is.

mod common;

use std::process::Stdio;
use std::sync::Arc;

use common::{announce, mcp_server, ws_server, Answer, Reply, CREDENTIAL};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vorn_cli::mcp::{relay, Credential, Endpoint, RelayError};

fn credential() -> Credential {
    Arc::new(|| Ok(CREDENTIAL.to_owned()))
}

/// Runs the relay over `input` and returns what it wrote, line by line.
async fn relayed(port: u16, input: &str) -> (Result<(), RelayError>, Vec<Value>) {
    let (writer, mut reader) = tokio::io::duplex(1 << 20);
    let outcome = relay(Endpoint { port }, credential(), input.as_bytes(), writer).await;
    let mut out = String::new();
    reader.read_to_string(&mut out).await.unwrap();
    let lines = out
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line is JSON"))
        .collect();
    (outcome, lines)
}

fn id_of(body: &Value) -> Value {
    body.get("id").cloned().unwrap_or(Value::Null)
}

/// A server that issues a session on initialize, answers tools/list as an
/// event stream, accepts notifications, and fails `boom`.
async fn scripted() -> (u16, common::Log) {
    mcp_server(|method, body| {
        if method == "DELETE" {
            return Reply::status(200, "");
        }
        let id = id_of(body);
        match body.get("method").and_then(Value::as_str) {
            Some("initialize") => Reply {
                session: Some("session-1"),
                // Pretty-printed, so the relay has to make it one line.
                ..Reply::json(
                    serde_json::to_string_pretty(&json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {"protocolVersion": "2025-06-18", "serverInfo": {"name": "vornd"}}
                    }))
                    .unwrap(),
                )
            },
            Some("tools/list") => Reply::events(format!(
                ": keep-alive\r\nevent: message\r\ndata: {{\"jsonrpc\":\"2.0\",\r\ndata: \"method\":\"notifications/progress\"}}\r\n\r\nid: 2\ndata: {}\n\nevent: other\ndata: {{\"ignored\":true}}\n\n",
                json!({"jsonrpc": "2.0", "id": id, "result": {"tools": []}})
            )),
            Some("boom") => Reply::status(500, "kaput"),
            Some(_) if body.get("id").is_none() => Reply::status(202, ""),
            _ => Reply::json(json!({"jsonrpc": "2.0", "id": id, "result": {}}).to_string()),
        }
    })
    .await
}

/// Talks as a client does: after each request, waits for its answer before
/// sending the next line.
async fn converse(port: u16, lines: &[&str]) -> (Result<(), RelayError>, Vec<Value>) {
    use tokio::io::AsyncBufReadExt;
    let (writer, reader) = tokio::io::duplex(1 << 20);
    let (mut input, input_end) = tokio::io::duplex(1 << 16);
    let run = tokio::spawn(relay(
        Endpoint { port },
        credential(),
        tokio::io::BufReader::new(input_end),
        writer,
    ));
    let mut output = tokio::io::BufReader::new(reader).lines();
    let mut answers = Vec::new();
    for line in lines {
        input
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        if request.get("id").is_some() {
            let answer = output.next_line().await.unwrap().expect("an answer");
            answers.push(serde_json::from_str(&answer).unwrap());
        }
    }
    drop(input);
    let outcome = run.await.unwrap();
    while let Some(line) = output.next_line().await.unwrap() {
        answers.push(serde_json::from_str(&line).unwrap());
    }
    (outcome, answers)
}

#[tokio::test]
async fn relays_json_and_keeps_the_session() {
    let (port, log) = scripted().await;
    let (outcome, lines) = converse(
        port,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ],
    )
    .await;
    outcome.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");

    let seen = log.lock().unwrap().clone();
    let methods: Vec<&str> = seen.iter().map(|s| s.method.as_str()).collect();
    assert_eq!(methods, ["POST", "POST", "DELETE"]);
    assert_eq!(
        seen[0].accept.as_deref(),
        Some("application/json, text/event-stream")
    );
    assert_eq!(seen[0].session, None);
    // Everything after initialize carries the session and the version it settled on.
    assert_eq!(seen[1].session.as_deref(), Some("session-1"));
    assert_eq!(seen[1].protocol.as_deref(), Some("2025-06-18"));
    assert_eq!(
        seen[1].body,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
    );
    assert_eq!(seen[2].session.as_deref(), Some("session-1"));
}

#[tokio::test]
async fn writes_each_event_of_a_stream_as_a_line() {
    let (port, _) = scripted().await;
    let (outcome, lines) = relayed(
        port,
        "{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"method\":\"tools/list\"}\n",
    )
    .await;
    outcome.unwrap();
    assert_eq!(
        lines,
        vec![
            json!({"jsonrpc": "2.0", "method": "notifications/progress"}),
            json!({"jsonrpc": "2.0", "id": "a", "result": {"tools": []}}),
        ]
    );
}

#[tokio::test]
async fn answers_a_failed_request_with_a_json_rpc_error() {
    let (port, log) = scripted().await;
    let (outcome, lines) = relayed(
        port,
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"boom\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"boom\"}\n",
    )
    .await;
    outcome.unwrap();
    // The notification fails silently on stdout: there is nobody to answer.
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["id"], 9);
    assert_eq!(lines[0]["error"]["code"], -32000);
    let message = lines[0]["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("500") && message.contains("kaput"),
        "{message}"
    );
    // No session was issued, so there is none to end.
    assert!(log.lock().unwrap().iter().all(|s| s.method == "POST"));
}

#[tokio::test]
async fn stops_when_vornd_does_not_serve_mcp() {
    let (port, _) = mcp_server(|_, _| Reply::status(404, "")).await;
    let (outcome, lines) = relayed(
        port,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
    )
    .await;
    assert!(matches!(outcome, Err(RelayError::NotServing { port: p }) if p == port));
    // The request is still answered, so the agent is not left waiting on it.
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["id"], 1);
    assert!(lines[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does not serve MCP"));
}

#[tokio::test]
async fn stops_when_the_session_is_gone() {
    let (port, _) = mcp_server(|_, body| match body.get("method").and_then(Value::as_str) {
        Some("initialize") => Reply {
            session: Some("old"),
            ..Reply::json(json!({"jsonrpc": "2.0", "id": 1, "result": {}}).to_string())
        },
        _ => Reply::status(404, ""),
    })
    .await;
    let (outcome, lines) = converse(
        port,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ],
    )
    .await;
    assert_eq!(lines.len(), 1);
    assert!(
        matches!(outcome, Err(RelayError::SessionGone)),
        "{outcome:?}"
    );
}

/// The binary, told by a fake server where vornd is.
async fn vorn_mcp(status: Value, input: &str) -> (i32, String, String) {
    let (mcp_port, _) = scripted().await;
    let status = match status {
        Value::Null => json!({"state": "on", "port": mcp_port, "nativeServer": true}),
        other => other,
    };
    let ws_port = ws_server(move |method, _| {
        assert_eq!(method, "server:vornd");
        Answer::Result(status.clone())
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    announce(dir.path(), ws_port);

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["mcp", "--data-dir"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).await.unwrap();
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[tokio::test]
async fn the_binary_relays_stdio_to_where_the_server_says() {
    let (code, out, err) = vorn_mcp(
        Value::Null,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
    )
    .await;
    assert_eq!(code, 0, "{err}");
    let lines: Vec<Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert!(lines.iter().any(|l| l["id"] == 1));
    assert!(lines.iter().any(|l| l["id"] == 2));
}

#[tokio::test]
async fn the_binary_refuses_a_vornd_that_does_not_serve_mcp() {
    let (code, out, err) = vorn_mcp(
        json!({"state": "on", "port": 1, "nativeServer": false}),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
    )
    .await;
    assert_eq!(code, 4);
    assert_eq!(out, "");
    assert!(err.contains("Native server"), "{err}");

    let (code, out, err) = vorn_mcp(json!({"state": "off"}), "").await;
    assert_eq!((code, out.as_str()), (4, ""));
    assert!(err.contains("not running vornd"), "{err}");
}

#[tokio::test]
async fn the_binary_says_when_no_server_is_announced() {
    let dir = tempfile::tempdir().unwrap();
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["mcp", "--data-dir"])
        .arg(dir.path())
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("Vorn port file not found"), "{err}");
}
