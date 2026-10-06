//! `vorn mcp` against a fake Streamable HTTP `/mcp`: the relay on its own,
//! then the binary end to end with a fake server telling it where vornd is.

mod common;

use std::process::Stdio;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    announce, mcp_server, vornd_mcp, ws_server, Answer, FakeMcp, Reply, Seen, CREDENTIAL,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::task::JoinHandle;
use vorn_cli::mcp::{relay, Backoff, Credential, Endpoint, Locate, RelayError, Upstream};
use vorn_cli::rpc::CallError;

fn credential() -> Credential {
    Arc::new(|| Ok(CREDENTIAL.to_owned()))
}

/// Short waits, so a test of a restart takes milliseconds.
const FAST: Backoff = Backoff {
    first: Duration::from_millis(10),
    max: Duration::from_millis(50),
    give_up_after: Duration::from_secs(10),
};

/// vornd on `port`, found there again whenever the relay asks.
fn upstream(port: u16) -> Upstream {
    Upstream {
        endpoint: Endpoint { port },
        locate: Arc::new(move || Box::pin(async move { Ok(Endpoint { port }) })),
        credential: credential(),
        backoff: FAST,
    }
}

/// Runs the relay over `input` and returns what it wrote, line by line.
async fn relayed(port: u16, input: &str) -> (Result<(), RelayError>, Vec<Value>) {
    let (writer, mut reader) = tokio::io::duplex(1 << 20);
    let outcome = relay(upstream(port), input.as_bytes(), writer).await;
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
    let (writer, reader) = tokio::io::duplex(1 << 20);
    let (mut input, input_end) = tokio::io::duplex(1 << 16);
    let run = tokio::spawn(relay(
        upstream(port),
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

/// An agent on the other end of a running relay: writes lines to its stdin
/// and reads its stdout as they come.
struct Agent {
    input: DuplexStream,
    output: Lines<BufReader<DuplexStream>>,
    run: JoinHandle<Result<(), RelayError>>,
}

impl Agent {
    fn start(upstream: Upstream) -> Agent {
        let (writer, reader) = tokio::io::duplex(1 << 20);
        let (input, input_end) = tokio::io::duplex(1 << 16);
        let run = tokio::spawn(relay(upstream, BufReader::new(input_end), writer));
        Agent {
            input,
            output: BufReader::new(reader).lines(),
            run,
        }
    }

    async fn send(&mut self, line: Value) {
        let line = format!("{line}\n");
        self.input.write_all(line.as_bytes()).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(20), self.output.next_line())
            .await
            .expect("an answer in time")
            .unwrap()
            .expect("stdout is still open");
        serde_json::from_str(&line).unwrap()
    }

    async fn ask(&mut self, id: u64, method: &str) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method}))
            .await;
        self.next().await
    }

    async fn handshake(&mut self) {
        let answer = self.ask(1, "initialize").await;
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
    }

    /// Closes stdin; what the relay returned and anything else it wrote.
    async fn close(self) -> (Result<(), RelayError>, Vec<Value>) {
        let Agent {
            input,
            mut output,
            run,
        } = self;
        drop(input);
        let outcome = run.await.unwrap();
        let mut rest = Vec::new();
        while let Some(line) = output.next_line().await.unwrap() {
            rest.push(serde_json::from_str(&line).unwrap());
        }
        (outcome, rest)
    }
}

fn seen(server: &FakeMcp) -> Vec<Seen> {
    server.log.lock().unwrap().clone()
}

fn posted(seen: &[Seen], method: &str) -> Vec<Seen> {
    seen.iter()
        .filter(|s| {
            serde_json::from_str::<Value>(&s.body)
                .ok()
                .and_then(|b| b.get("method").and_then(Value::as_str).map(str::to_owned))
                .as_deref()
                == Some(method)
        })
        .cloned()
        .collect()
}

#[tokio::test]
async fn reopens_the_session_when_vornd_restarts_on_the_same_port() {
    let before = vornd_mcp(0, CREDENTIAL, "s1").await;
    let port = before.port;
    let mut agent = Agent::start(upstream(port));
    agent.handshake().await;
    assert_eq!(agent.ask(2, "tools/list").await["result"]["from"], "s1");

    before.stop().await;
    // Asked while vornd is down: both wait for it rather than failing.
    agent
        .send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}))
        .await;
    agent
        .send(json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"}))
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let after = vornd_mcp(port, CREDENTIAL, "s2").await;

    let mut answers = vec![agent.next().await, agent.next().await];
    answers.sort_by_key(|a| a["id"].as_u64());
    assert_eq!(answers[0]["id"], 3);
    assert_eq!(answers[1]["id"], 4);
    assert!(
        answers.iter().all(|a| a["result"]["from"] == "s2"),
        "{answers:?}"
    );

    let (outcome, rest) = agent.close().await;
    outcome.unwrap();
    // The replayed initialize was answered to the relay, not the agent.
    assert_eq!(rest, Vec::<Value>::new());

    let log = seen(&after);
    let inits = posted(&log, "initialize");
    assert_eq!(inits.len(), 1, "one reconnect for both requests: {log:?}");
    assert_eq!(inits[0].session, None);
    let note = posted(&log, "notifications/initialized");
    assert_eq!(note.len(), 1);
    assert_eq!(note[0].session.as_deref(), Some("s2"));
    // Where connecting to a closed port is slow to fail, a request may first reach the new vornd with the old session.
    let listed = posted(&log, "tools/list");
    let last = listed.last().unwrap();
    assert_eq!(last.session.as_deref(), Some("s2"));
    assert_eq!(last.protocol.as_deref(), Some("2025-06-18"));
    assert!(listed
        .iter()
        .all(|asked| matches!(asked.session.as_deref(), Some("s1" | "s2"))));
    let last = log.last().unwrap();
    assert_eq!(
        (last.method.as_str(), last.session.as_deref()),
        ("DELETE", Some("s2"))
    );
}

#[tokio::test]
async fn follows_vornd_to_a_new_port_and_credential() {
    let before = vornd_mcp(0, "old-credential", "s1").await;
    let port = Arc::new(AtomicU16::new(before.port));
    let token = Arc::new(Mutex::new(Some("old-credential".to_owned())));
    let locate: Locate = {
        let port = port.clone();
        Arc::new(move || {
            let port = port.load(Ordering::SeqCst);
            Box::pin(async move { Ok(Endpoint { port }) })
        })
    };
    let credential: Credential = {
        let token = token.clone();
        Arc::new(move || {
            token
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| CallError::NoCredential("local-token".into()))
        })
    };
    let mut agent = Agent::start(Upstream {
        endpoint: Endpoint { port: before.port },
        locate,
        credential,
        backoff: FAST,
    });
    agent.handshake().await;

    // Vorn quits: vornd stops and the server removes the credential.
    before.stop().await;
    *token.lock().unwrap() = None;
    agent
        .send(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}))
        .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Vorn is back, elsewhere, with a new credential.
    let after = vornd_mcp(0, "new-credential", "s2").await;
    port.store(after.port, Ordering::SeqCst);
    *token.lock().unwrap() = Some("new-credential".to_owned());

    let answer = agent.next().await;
    assert_eq!(answer["id"], 5);
    assert_eq!(answer["result"]["from"], "s2");
    assert_eq!(agent.ask(6, "tools/list").await["result"]["from"], "s2");
    let (outcome, _) = agent.close().await;
    outcome.unwrap();
    assert_eq!(posted(&seen(&after), "initialize").len(), 1);
}

#[tokio::test]
async fn fails_what_waited_but_stays_open_when_vornd_does_not_come_back_in_time() {
    let before = vornd_mcp(0, CREDENTIAL, "s1").await;
    let port = before.port;
    let mut agent = Agent::start(Upstream {
        backoff: Backoff {
            give_up_after: Duration::from_millis(200),
            ..FAST
        },
        ..upstream(port)
    });
    agent.handshake().await;
    before.stop().await;

    let failed = agent.ask(7, "tools/list").await;
    assert_eq!(failed["id"], 7);
    assert_eq!(failed["error"]["code"], -32000);
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(message.contains("did not come back"), "{message}");

    // The relay is still there for the agent when vornd returns.
    let after = vornd_mcp(port, CREDENTIAL, "s2").await;
    assert_eq!(agent.ask(8, "tools/list").await["result"]["from"], "s2");
    let (outcome, rest) = agent.close().await;
    outcome.unwrap();
    assert!(rest.is_empty(), "{rest:?}");
    assert_eq!(posted(&seen(&after), "initialize").len(), 1);
}

/// Reads each request and hangs up without answering, as a vornd that dies
/// mid-call does. Counts the requests that reached it.
async fn hangs_up() -> (u16, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            // The whole request is in once its JSON body has closed.
            while !request.ends_with(b"}") {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            counted.fetch_add(1, Ordering::SeqCst);
        }
    });
    (port, count)
}

#[tokio::test]
async fn never_sends_a_tool_call_twice() {
    let (port, count) = hangs_up().await;
    let mut agent = Agent::start(upstream(port));
    let failed = agent.ask(9, "tools/call").await;
    assert_eq!(failed["id"], 9);
    assert_eq!(failed["error"]["code"], -32000);
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // What vornd may act on twice is sent again, a bounded number of times.
    let started = std::time::Instant::now();
    let failed = agent.ask(10, "tools/list").await;
    assert_eq!(failed["id"], 10);
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(message.contains("kept turning"), "{message}");
    assert_eq!(count.load(Ordering::SeqCst), 1 + 4);
    assert!(started.elapsed() < FAST.give_up_after / 2);
    let (outcome, _) = agent.close().await;
    outcome.unwrap();
}

#[tokio::test]
async fn says_at_once_when_the_credential_was_never_taken() {
    let wrong = vornd_mcp(0, "another-servers-credential", "s1").await;
    let started = std::time::Instant::now();
    let (outcome, lines) = relayed(
        wrong.port,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n",
    )
    .await;
    outcome.unwrap();
    assert!(started.elapsed() < FAST.give_up_after / 2);
    assert_eq!(lines.len(), 1);
    let message = lines[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("401"), "{message}");
    assert_eq!(seen(&wrong).len(), 1);
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

/// A Vorn server whose vornd is on whatever port `vornd_port` holds.
async fn server_with_vornd_at(vornd_port: Arc<AtomicU16>) -> u16 {
    ws_server(move |method, _| {
        assert_eq!(method, "server:vornd");
        let port = vornd_port.load(Ordering::SeqCst);
        Answer::Result(json!({"state": "on", "port": port, "nativeServer": true}))
    })
    .await
}

#[tokio::test]
async fn the_binary_follows_vornd_when_vorn_restarts_elsewhere() {
    let before = vornd_mcp(0, CREDENTIAL, "s1").await;
    let vornd_port = Arc::new(AtomicU16::new(before.port));
    let dir = tempfile::tempdir().unwrap();
    announce(dir.path(), server_with_vornd_at(vornd_port.clone()).await);

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vorn"))
        .args(["mcp", "--data-dir"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut ask = async |id: u64, method: &str| -> Value {
        let line = format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "id": id, "method": method})
        );
        stdin.write_all(line.as_bytes()).await.unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(20), stdout.next_line())
            .await
            .expect("an answer in time")
            .unwrap()
            .expect("stdout is still open");
        serde_json::from_str(&answer).unwrap()
    };
    assert_eq!(ask(1, "initialize").await["id"], 1);

    // Vorn quits: vornd stops and the announcement goes with the server.
    before.stop().await;
    std::fs::remove_file(dir.path().join("ws-port")).unwrap();
    std::fs::remove_file(dir.path().join("local-token")).unwrap();
    let back = {
        let vornd_port = vornd_port.clone();
        let dir = dir.path().to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            // It comes back with the server and vornd both on new ports.
            let after = vornd_mcp(0, CREDENTIAL, "s2").await;
            vornd_port.store(after.port, Ordering::SeqCst);
            announce(&dir, server_with_vornd_at(vornd_port).await);
            after
        })
    };
    let answer = ask(2, "tools/list").await;
    assert_eq!(answer["id"], 2);
    assert_eq!(answer["result"]["from"], "s2");
    let after = back.await.unwrap();
    assert_eq!(posted(&seen(&after), "initialize").len(), 1);

    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
