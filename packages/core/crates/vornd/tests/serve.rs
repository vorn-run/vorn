//! vornd as the server: what it publishes, greeting and admission, topics, unknown calls, the default-dir guard.

use std::io::BufRead;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const CREDENTIAL: &str = "serve-test-credential";

/// vornd serving `data`, with `home` as its home; killed when dropped.
struct Served {
    child: Child,
    port: u16,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(data: &Path, home: &Path) -> Served {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .args(["--data-dir"])
        .arg(data)
        .args(["--port", "0"])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("SECRET_VORN_BOOTSTRAP_TOKEN", CREDENTIAL)
        .env("VORND_KEYCHAIN", "0")
        .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("vornd starts");
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: Value = serde_json::from_str(&line).expect("vornd says where it listens");
    let port = u16::try_from(ready["port"].as_u64().unwrap()).unwrap();
    Served { child, port }
}

async fn open(port: u16, bearer: Option<&str>, query: &str) -> Socket {
    let mut req = format!("ws://127.0.0.1:{port}/ws{query}")
        .into_client_request()
        .unwrap();
    if let Some(token) = bearer {
        req.headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

/// The next text frame, as JSON; `None` once the socket closed, with its code.
async fn next(ws: &mut Socket) -> Result<Value, Option<u16>> {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("an answer in time");
        match msg {
            Some(Ok(Message::Text(t))) => return Ok(serde_json::from_str(t.as_str()).unwrap()),
            Some(Ok(Message::Close(frame))) => return Err(frame.map(|f| u16::from(f.code))),
            Some(Ok(_)) => continue,
            _ => return Err(None),
        }
    }
}

/// The answer to call `id`, skipping notifications.
async fn answer(ws: &mut Socket, id: u64) -> Value {
    loop {
        let frame = next(ws).await.expect("still open");
        if frame["id"] == json!(id) {
            return frame;
        }
    }
}

async fn send(ws: &mut Socket, frame: Value) {
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serves_clients_with_nothing_behind_it() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());

    // What it publishes, for anything on this machine to find and reach it by.
    let record: Value =
        serde_json::from_slice(&std::fs::read(data.path().join("ws-port")).unwrap()).unwrap();
    assert_eq!(record["port"], json!(served.port));
    assert_eq!(record["pid"], json!(served.child.id()));
    assert_eq!(
        std::fs::read_to_string(data.path().join("local-token")).unwrap(),
        CREDENTIAL
    );
    let health = reqwest_get(served.port, "/health").await;
    assert_eq!(health, r#"{"status":"ok"}"#);

    // Greeted, and admitted by the credential on the upgrade.
    let mut desktop = open(served.port, Some(CREDENTIAL), "?topics=config:changed").await;
    let hello = next(&mut desktop).await.unwrap();
    assert_eq!(hello["method"], "server:hello");
    assert_eq!(hello["params"]["protocolVersion"], 1);
    let identity = next(&mut desktop).await.unwrap();
    assert_eq!(identity["method"], "server:identity");
    assert_eq!(identity["params"]["pid"], json!(served.child.id()));

    // Without one, only `auth:authenticate` is taken.
    let mut phone = open(served.port, None, "").await;
    next(&mut phone).await.unwrap();
    next(&mut phone).await.unwrap();
    send(&mut phone, json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": CREDENTIAL } })).await;
    let ok = next(&mut phone).await.unwrap();
    assert_eq!(ok["method"], "auth:ok");
    assert!(ok["params"]["userId"].is_string());
    assert_eq!(answer(&mut phone, 1).await["result"], json!({ "ok": true }));
    send(&mut phone, json!({ "jsonrpc": "2.0", "id": 2, "method": "subscribe:set", "params": { "topics": ["pairing:*"] } })).await;
    assert_eq!(answer(&mut phone, 2).await["result"], json!({ "ok": true }));

    // A change is told to the clients that asked for it.
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "config:load" }),
    )
    .await;
    let config = answer(&mut desktop, 3).await["result"].clone();
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "config:save", "params": config }),
    )
    .await;
    let mut told = false;
    for _ in 0..4 {
        let frame = next(&mut desktop).await.unwrap();
        told |= frame["method"] == "config:changed";
        if told {
            break;
        }
    }
    assert!(told, "the change was told");
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "nope:nothing" }),
    )
    .await;
    assert_eq!(
        answer(&mut desktop, 5).await["error"],
        json!({ "code": -32601, "message": "Method not found: nope:nothing" })
    );
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "server:vornd" }),
    )
    .await;
    assert_eq!(
        answer(&mut desktop, 6).await["result"],
        json!({ "state": "on", "port": served.port })
    );

    let health: Value =
        serde_json::from_str(&reqwest_get(served.port, "/vornd/health").await).unwrap();
    assert_eq!(health["ok"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_a_socket_that_does_not_authenticate() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());

    let mut wrong = open(served.port, Some("not-it"), "").await;
    next(&mut wrong).await.unwrap();
    next(&mut wrong).await.unwrap();
    assert_eq!(next(&mut wrong).await.unwrap_err(), Some(4002));

    let mut early = open(served.port, None, "").await;
    next(&mut early).await.unwrap();
    next(&mut early).await.unwrap();
    send(
        &mut early,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "config:load" }),
    )
    .await;
    let refused = next(&mut early).await.unwrap();
    assert_eq!(refused["error"]["code"], -32001);
    assert_eq!(next(&mut early).await.unwrap_err(), Some(4001));

    let mut bad = open(served.port, None, "").await;
    next(&mut bad).await.unwrap();
    next(&mut bad).await.unwrap();
    send(&mut bad, json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": "nope" } })).await;
    assert_eq!(
        next(&mut bad).await.unwrap()["error"]["message"],
        "Authentication failed"
    );
    assert_eq!(next(&mut bad).await.unwrap_err(), Some(4002));
}

#[test]
fn a_second_vornd_on_the_directory_stands_down() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let _first = serve(data.path(), home.path());
    let second = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .arg("--data-dir")
        .arg(data.path())
        .args(["--port", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(3));
}

#[test]
fn a_debug_build_will_not_touch_the_default_data_directory() {
    if !cfg!(debug_assertions) {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let default = home.path().join(".vorn");
    let out = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .arg("--data-dir")
        .arg(&default)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("default data directory"));
    assert!(!default.exists());
}

/// GETs `path` and answers the body.
async fn reqwest_get(port: u16, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned()
}
