//! vornd as the server: what it publishes, greeting and admission, topics,
//! unknown calls, the default-dir guard, TLS and revoking a device.

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
type TlsSocket = WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

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
    serve_with(data, home, &[])
}

fn serve_with(data: &Path, home: &Path, args: &[&str]) -> Served {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .args(args)
        .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
        .env("VORN_TAILSCALE", "none")
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
async fn next<S>(ws: &mut WebSocketStream<S>) -> Result<Value, Option<u16>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
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
async fn answer<S>(ws: &mut WebSocketStream<S>, id: u64) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let frame = next(ws).await.expect("still open");
        if frame["id"] == json!(id) {
            return frame;
        }
    }
}

async fn send<S>(ws: &mut WebSocketStream<S>, frame: Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

/// A vornd the app started says so, so that only an app restarts it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn says_when_the_app_started_it() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve_with(data.path(), home.path(), &["--for-app"]);
    let mut desktop = open(served.port, Some(CREDENTIAL), "").await;
    assert_eq!(next(&mut desktop).await.unwrap()["method"], "server:hello");
    let identity = next(&mut desktop).await.unwrap();
    assert_eq!(identity["method"], "server:identity");
    assert_eq!(identity["params"]["startedByApp"], true);
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
    assert_eq!(identity["params"]["startedByApp"], false);

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
    // vornd's reports want the local credential and a Host that names this machine.
    let port = served.port;
    assert_eq!(
        reqwest_get_as(port, "/vornd/health", "127.0.0.1", None).await,
        "a token is required"
    );
    assert_eq!(
        reqwest_get_as(port, "/vornd/sessions", "127.0.0.1", Some("nope")).await,
        "a token is required"
    );
    assert_eq!(
        reqwest_get_as(port, "/vornd/health", "rebound.example", Some(CREDENTIAL)).await,
        "this route answers only requests addressed to this machine"
    );
    // A browser here holds a device token, which reads them too.
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 7, "method": "token:create", "params": { "name": "Browser" } }),
    )
    .await;
    let minted = answer(&mut desktop, 7).await["result"]["plaintext"]
        .as_str()
        .unwrap()
        .to_owned();
    let read: Value = serde_json::from_str(
        &reqwest_get_as(port, "/vornd/health", "127.0.0.1", Some(&minted)).await,
    )
    .unwrap();
    assert_eq!(read["ok"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pages_the_task_list() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());
    let mut ws = open(served.port, Some(CREDENTIAL), "").await;
    next(&mut ws).await.unwrap();
    next(&mut ws).await.unwrap();
    let call = |id: u64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    send(&mut ws, call(1, "config:load", Value::Null)).await;
    let mut config = answer(&mut ws, 1).await["result"].clone();
    let path = home.path().to_string_lossy();
    config["projects"] = json!([{ "name": "p", "path": path, "preferredAgents": [] }]);
    send(&mut ws, call(2, "config:save", config)).await;
    answer(&mut ws, 2).await;
    for (i, title) in ["one", "two", "three", "four"].iter().enumerate() {
        let id = 10 + i as u64;
        send(
            &mut ws,
            call(
                id,
                "task:create",
                json!({ "projectName": "p", "title": title }),
            ),
        )
        .await;
        assert_eq!(answer(&mut ws, id).await["result"]["ok"], true);
    }
    let titles = |v: &Value| -> Vec<String> {
        let list = v["result"].as_array().unwrap().iter();
        list.map(|t| t["title"].as_str().unwrap().to_owned())
            .collect()
    };
    send(
        &mut ws,
        call(20, "task:list", json!({ "offset": 1, "limit": 2 })),
    )
    .await;
    assert_eq!(titles(&answer(&mut ws, 20).await), ["two", "three"]);
    send(&mut ws, call(21, "task:list", json!({ "offset": 3 }))).await;
    assert_eq!(titles(&answer(&mut ws, 21).await), ["four"]);
    send(&mut ws, call(22, "task:list", json!({}))).await;
    assert_eq!(titles(&answer(&mut ws, 22).await).len(), 4);
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

/// A socket over TLS that trusts only the certificate `pin` names.
async fn open_pinned(port: u16, pin: &str) -> Result<TlsSocket, String> {
    let config = vorn_reach::tls::client_config(pin).map_err(|e| e.to_string())?;
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let tls = tokio_rustls::TlsConnector::from(config)
        .connect(name, tcp)
        .await
        .map_err(|e| e.to_string())?;
    let req = format!("wss://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    let (ws, _) = tokio_tungstenite::client_async(req, tls)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ws)
}

/// The fingerprint of the certificate vornd keeps in `data`.
fn kept_fingerprint(data: &Path) -> String {
    vorn_reach::tls::fingerprint(&std::fs::read(data.join("tls-cert.der")).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusts_a_client_only_with_the_pinned_certificate() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());
    let pin = kept_fingerprint(data.path());

    let mut ws = open_pinned(served.port, &pin)
        .await
        .expect("the right pin connects");
    assert_eq!(next(&mut ws).await.unwrap()["method"], "server:hello");

    let wrong = vorn_reach::tls::fingerprint(b"another certificate");
    let refused = open_pinned(served.port, &wrong).await.expect_err("refused");
    assert!(refused.contains("certificate"), "{refused}");

    // The certificate is kept, so a paired phone's pin holds after a restart.
    drop(served);
    let again = serve(data.path(), home.path());
    assert_eq!(kept_fingerprint(data.path()), pin);
    assert!(open_pinned(again.port, &pin).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_device_is_closed_out_at_once() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());
    let pin = kept_fingerprint(data.path());
    let mut desktop = open(served.port, Some(CREDENTIAL), "").await;
    send(&mut desktop, json!({ "jsonrpc": "2.0", "id": 1, "method": "token:create", "params": { "name": "Phone" } })).await;
    let minted = answer(&mut desktop, 1).await["result"].clone();
    let token = minted["plaintext"].as_str().unwrap().to_owned();
    let id = minted["token"]["id"].as_str().unwrap().to_owned();

    let mut phone = open_pinned(served.port, &pin).await.unwrap();
    send(&mut phone, json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": token } })).await;
    assert_eq!(answer(&mut phone, 1).await["result"], json!({ "ok": true }));

    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "token:list" }),
    )
    .await;
    let listed = answer(&mut desktop, 2).await["result"].clone();
    assert_eq!(listed[0]["lastAddress"], "127.0.0.1", "{listed}");

    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "token:revoke", "params": id }),
    )
    .await;
    assert_eq!(answer(&mut desktop, 3).await["result"]["revoked"], true);
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Err(code) = next(&mut phone).await {
                return code;
            }
        }
    })
    .await
    .expect("closed at once");
    assert_eq!(closed, Some(4002));

    let mut again = open_pinned(served.port, &pin).await.unwrap();
    send(&mut again, json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": token } })).await;
    assert_eq!(
        answer(&mut again, 1).await["error"]["message"],
        "Authentication failed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_listens_on_the_lan_while_sharing_is_off() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let served = serve(data.path(), home.path());
    let mut desktop = open(served.port, Some(CREDENTIAL), "").await;
    send(
        &mut desktop,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "server:reachableUrls" }),
    )
    .await;
    let reachable = answer(&mut desktop, 1).await["result"].clone();
    assert_eq!(reachable["remote"], false, "{reachable}");
    assert_eq!(reachable["urls"], json!([]));
    for lan in vorn_reach::sys::lan_addresses() {
        let ip: std::net::IpAddr = lan.parse().unwrap();
        let refused = TcpStream::connect((ip, served.port)).await;
        assert!(refused.is_err(), "{lan}:{} answered", served.port);
    }
}

#[test]
fn a_second_vornd_on_the_directory_stands_down() {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let _first = serve(data.path(), home.path());
    let second = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
        .env("VORN_TAILSCALE", "none")
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
        .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
        .env("VORN_TAILSCALE", "none")
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
    reqwest_get_as(port, path, "127.0.0.1", Some(CREDENTIAL)).await
}

/// A GET naming `host`, with `bearer` when given.
async fn reqwest_get_as(port: u16, path: &str, host: &str, bearer: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let auth = bearer.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned()
}
