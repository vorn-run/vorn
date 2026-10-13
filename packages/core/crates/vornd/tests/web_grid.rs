//! Grid mode over vornd's `/grid` WebSocket, as a browser reaches it: the
//! real vornd and vorn-sessiond on a home and data directory of the test's
//! own. The socket authenticates as `/ws` does, then carries the grid
//! protocol: a snapshot and deltas a headless mirror follows, input, and a
//! resize. A revoked token closes it.
//!
//! `VORN_RECORD_GRID_FRAMES=<file>` writes what the browser sent and was
//! sent, as hex, for the web client's decoder test.

#![cfg(all(feature = "engine", unix))]

mod common;

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use vorn_grid_client::{Client, Got};
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, ServerMsg, Size,
    PROTO_MAJOR, PROTO_MINOR,
};

use common::{Reap, Socket, Vornd, CREDENTIAL, PATIENCE};

async fn open(port: u16, bearer: Option<&str>) -> Socket {
    let mut req = format!("ws://127.0.0.1:{port}/grid")
        .into_client_request()
        .unwrap();
    if let Some(b) = bearer {
        req.headers_mut()
            .insert("authorization", format!("Bearer {b}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

/// The close code the socket ends with, skipping anything before it.
async fn closed_with(ws: &mut Socket) -> Option<u16> {
    loop {
        match tokio::time::timeout(PATIENCE, ws.next())
            .await
            .expect("a close in time")
        {
            Some(Ok(Message::Close(f))) => return f.map(|f| u16::from(f.code)),
            Some(Ok(_)) => {}
            _ => return None,
        }
    }
}

/// The next text frame, as JSON.
async fn text(ws: &mut Socket) -> Value {
    loop {
        let m = tokio::time::timeout(PATIENCE, ws.next())
            .await
            .expect("a frame in time")
            .expect("open")
            .unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(t.as_str()).unwrap();
        }
    }
}

/// A desktop on `/ws`, for the calls the browser does not make itself.
struct Desktop {
    ws: Socket,
    next_id: u64,
}

impl Desktop {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        common::send(
            &mut self.ws,
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await;
        loop {
            let frame = common::next(&mut self.ws).await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }
}

/// A browser's grid connection: a headless mirror, and what went each way.
struct Browser {
    ws: Socket,
    client: Client,
    got: Vec<Got>,
    sent: Vec<Vec<u8>>,
    received: Vec<Vec<u8>>,
}

impl Browser {
    fn new(ws: Socket) -> Browser {
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: PROTO_MINOR,
            caps: caps::ALL,
            client: ClientKind::Web,
            build: "web-test".into(),
        }));
        Browser {
            ws,
            client,
            got: Vec::new(),
            sent: Vec::new(),
            received: Vec::new(),
        }
    }

    async fn flush(&mut self) {
        let out = self.client.take_out();
        if !out.is_empty() {
            self.sent.push(out.clone());
            self.ws.send(Message::binary(out)).await.unwrap();
        }
    }

    /// Reads until `done` holds, acknowledging every frame as it comes.
    async fn until(&mut self, what: &str, done: impl Fn(&Browser) -> bool) {
        let t = Instant::now();
        loop {
            self.flush().await;
            if done(self) {
                return;
            }
            let left = PATIENCE.saturating_sub(t.elapsed());
            let m = match tokio::time::timeout(left, self.ws.next()).await {
                Ok(Some(Ok(m))) => m,
                Ok(other) => panic!("{what}: the socket ended: {other:?}; got {:?}", self.got),
                Err(_) => panic!("{what}: timed out; got {:?}", self.got),
            };
            let Message::Binary(bytes) = m else {
                continue;
            };
            self.received.push(bytes.to_vec());
            for g in self.client.receive(&bytes).unwrap() {
                if let Got::Frame { sid, rev, .. } = g {
                    self.client.ack(sid, rev);
                }
                if let Got::Refused { why, .. } = g {
                    panic!("{what}: a frame was refused: {why:?}");
                }
                self.got.push(g);
            }
        }
    }

    fn attached(&self) -> Option<u32> {
        self.got.iter().find_map(|g| match g {
            Got::Attached(a) => Some(a.sid),
            _ => None,
        })
    }

    fn text(&self, sid: u32) -> String {
        self.client
            .pane(sid)
            .and_then(|p| p.mirror())
            .map(|m| m.text().join("\n"))
            .unwrap_or_default()
    }

    fn record(&self) {
        let Some(path) = std::env::var_os("VORN_RECORD_GRID_FRAMES") else {
            return;
        };
        let hex = |frames: &[Vec<u8>]| -> Vec<String> {
            frames
                .iter()
                .map(|f| f.iter().map(|b| format!("{b:02x}")).collect())
                .collect()
        };
        let file = json!({ "sent": hex(&self.sent), "received": hex(&self.received) });
        std::fs::write(path, serde_json::to_string_pretty(&file).unwrap()).unwrap();
    }
}

fn view(cols: u16, rows: u16) -> Size {
    Size {
        cols,
        rows,
        px_w: 0,
        px_h: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_follows_types_into_and_resizes_a_session_over_grid() {
    let v = Vornd::start();
    let _reap = Reap(v.holder());
    let mut d = Desktop {
        ws: common::connect(v.port, CREDENTIAL).await,
        next_id: 0,
    };
    let cwd = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let id = loop {
        let answer = d
            .call("shell:create", json!(cwd.path().to_str().unwrap()))
            .await;
        if let Some(id) = answer["result"]["id"].as_str() {
            break id.to_owned();
        }
        assert!(t.elapsed() < PATIENCE, "{answer}\n{}", v.log());
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let minted = d.call("token:create", json!({ "name": "browser" })).await;
    let token = minted["result"]["plaintext"].as_str().unwrap().to_owned();
    let token_id = minted["result"]["token"]["id"].as_str().unwrap().to_owned();

    // Grid before authenticating, and a token vornd never issued, are refused.
    let mut ws = open(v.port, None).await;
    ws.send(Message::binary(vec![1, 0, 0, 0, 1])).await.unwrap();
    assert_eq!(closed_with(&mut ws).await, Some(4001));
    let mut ws = open(v.port, None).await;
    common::send(
        &mut ws,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": "nope" } }),
    )
    .await;
    assert_eq!(text(&mut ws).await["error"]["code"], -32001);
    assert_eq!(closed_with(&mut ws).await, Some(4002));
    let mut ws = open(v.port, Some("nope")).await;
    assert_eq!(closed_with(&mut ws).await, Some(4002));

    // The device token in the first frame, as a browser sends it.
    let mut ws = open(v.port, None).await;
    common::send(
        &mut ws,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "auth:authenticate", "params": { "token": token } }),
    )
    .await;
    assert_eq!(text(&mut ws).await["method"], "auth:ok");
    assert_eq!(text(&mut ws).await["result"]["ok"], true);
    let mut b = Browser::new(ws);
    b.until("welcome", |b| b.client.welcome().is_some()).await;
    b.client.attach(Attach {
        session: id.clone(),
        mode: AttachMode::Grid,
        view: view(80, 24),
        visible: true,
        resume: None,
        history_tail: 0,
    });
    b.until("a snapshot", |b| {
        b.attached().is_some_and(|s| {
            b.got
                .iter()
                .any(|g| matches!(g, Got::Frame { sid, snapshot: true, .. } if *sid == s))
        })
    })
    .await;
    let sid = b.attached().unwrap();

    let seq = b.client.input_seq();
    b.client.send(&ClientMsg::Input {
        sid,
        input_seq: seq,
        event: InputEvent::Text {
            utf8: "echo grid-$((6*7))\r".into(),
        },
    });
    b.until("the echo", |b| b.text(sid).contains("grid-42"))
        .await;

    b.client.send(&ClientMsg::Viewport {
        sid,
        size: view(100, 30),
    });
    b.client.send(&ClientMsg::TakeSize { sid });
    b.until("the resize", |b| {
        b.got.iter().any(|g| {
            matches!(
                g,
                Got::Message(ServerMsg::Resized {
                    cols: 100,
                    rows: 30,
                    ..
                })
            )
        })
    })
    .await;
    b.until("the mirror at the new size", |b| {
        b.client
            .pane(sid)
            .and_then(|p| p.mirror())
            .is_some_and(|m| (m.term().cols, m.term().rows) == (100, 30))
    })
    .await;
    b.record();

    // Revoking the token ends the socket.
    let revoked = d.call("token:revoke", json!(token_id)).await;
    assert_eq!(revoked["result"]["revoked"], true, "{revoked}");
    assert_eq!(closed_with(&mut b.ws).await, Some(4002));

    // The desktop's credential on the upgrade admits at once.
    let mut b = Browser::new(open(v.port, Some(CREDENTIAL)).await);
    b.until("welcome", |b| b.client.welcome().is_some()).await;
}
