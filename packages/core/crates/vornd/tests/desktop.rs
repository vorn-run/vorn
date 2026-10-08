//! The desktop bridge end to end: main claims its connection to vornd with
//! `bridge:identify`, and an agent's `browser:*` and `device:*` calls reach
//! main through vornd and come back, none of them reaching the server.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vornd::{Daemon, Groups};

const TOKEN: &str = "local-secret";
const NATIVE: &str = "browser=native,device=native,bridge=native";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A server that keeps every frame it is sent and answers each request
/// with an error.
async fn stand_in() -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let kept = Arc::clone(&kept);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let frame: Value = serde_json::from_str(text.as_str()).unwrap();
                    kept.lock().unwrap().push(frame.clone());
                    if let (Some(id), Some(_)) = (frame.get("id"), frame.get("method")) {
                        let answer = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "from the server" } });
                        let _ = ws.send(Message::text(answer.to_string())).await;
                    }
                }
            });
        }
    });
    (addr, seen)
}

async fn vornd(groups: &str) -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let (upstream, seen) = stand_in().await;
    let daemon = Daemon::new(upstream, Groups::parse(groups).unwrap());
    daemon.set_desktop_token(TOKEN.as_bytes().to_vec());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    daemon.set_listen_addr(addr);
    tokio::spawn(vornd::serve(listener, daemon, std::future::pending()));
    (addr, seen)
}

async fn connect(addr: SocketAddr) -> Socket {
    let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

async fn send(ws: &mut Socket, frame: Value) {
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

async fn next(ws: &mut Socket) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("a frame in time")
            .expect("open")
            .unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn identify(ws: &mut Socket, id: u64) -> Value {
    send(
        ws,
        json!({ "jsonrpc": "2.0", "id": id, "method": "bridge:identify" }),
    )
    .await;
    next(ws).await
}

fn methods(seen: &Mutex<Vec<Value>>) -> Vec<(Option<Value>, String)> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f.get("id").cloned(),
                f["method"].as_str().unwrap_or("(answer)").to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn an_agents_browser_call_reaches_main_and_comes_back() {
    let (addr, seen) = vornd(NATIVE).await;
    let mut main = connect(addr).await;
    assert_eq!(
        identify(&mut main, 1).await,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } })
    );

    let mut agent = connect(addr).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "browser:tabs", "params": { "sessionId": "s" } }),
    )
    .await;
    let asked = next(&mut main).await;
    assert_eq!(asked["method"], "browser:tabs");
    assert_eq!(asked["params"], json!({ "sessionId": "s" }));
    let id = asked["id"].clone();
    assert!(id.as_str().is_some_and(|i| i.starts_with("vornd-")), "{id}");
    send(
        &mut main,
        json!({ "jsonrpc": "2.0", "id": id, "result": [{ "id": "t1" }] }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 5, "result": [{ "id": "t1" }] })
    );

    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "device:claim", "params": { "udid": "u" } }),
    )
    .await;
    let asked = next(&mut main).await;
    send(
        &mut main,
        json!({ "jsonrpc": "2.0", "id": asked["id"], "error": { "code": -32000, "message": "no such device" } }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 6, "error": { "code": -32000, "message": "no such device" } })
    );

    // The server hears none of it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(methods(&seen).is_empty(), "{:?}", methods(&seen));
}

#[tokio::test]
async fn one_main_at_a_time_and_its_close_fails_what_waits() {
    let (addr, _) = vornd(NATIVE).await;
    let mut agent = connect(addr).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "browser:tabs" }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await["error"]["message"],
        "Vorn app is not running (no main process connected)"
    );

    let mut main = connect(addr).await;
    assert_eq!(identify(&mut main, 1).await["result"]["ok"], true);
    let mut thief = connect(addr).await;
    assert_eq!(identify(&mut thief, 2).await["result"]["ok"], false);

    // A guessed answer from another socket settles nothing.
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "device:list" }),
    )
    .await;
    let asked = next(&mut main).await;
    send(
        &mut thief,
        json!({ "jsonrpc": "2.0", "id": asked["id"], "result": "forged" }),
    )
    .await;
    main.close(None).await.unwrap();
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32000, "message": "Vorn main process disconnected" } })
    );
    assert_eq!(identify(&mut thief, 3).await["result"]["ok"], true);
}

#[tokio::test]
async fn forwarded_the_server_answers_as_before() {
    let (addr, seen) = vornd("").await;
    let mut main = connect(addr).await;
    assert_eq!(
        identify(&mut main, 1).await["error"]["message"],
        "from the server"
    );
    assert_eq!(
        methods(&seen),
        [(Some(json!(1)), "bridge:identify".to_owned())]
    );
}
