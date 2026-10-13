//! A shell started through vornd the way the desktop starts one: created,
//! attached as soon as the answer is in, and typed into. The real vornd and
//! vorn-sessiond binaries, on a home and data directory of the test's own.

#![cfg(unix)]

mod common;

use std::time::Instant;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use vorn_term_proto::bytes::BytesFrame;

use common::{Reap, Socket, Vornd, CREDENTIAL, PATIENCE};

/// A desktop's connection: its calls, and what the attached terminal printed.
struct Desktop {
    ws: Socket,
    next_id: u64,
    output: Vec<u8>,
    exits: Vec<Value>,
}

impl Desktop {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        common::send(&mut self.ws, frame).await;
        loop {
            let frame = self.read().await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }

    /// The next text frame, keeping the terminal output and exits seen on the way.
    async fn read(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(PATIENCE, self.ws.next())
                .await
                .expect("a frame in time")
                .expect("open")
                .unwrap();
            match msg {
                Message::Binary(b) => {
                    let f = BytesFrame::decode(&b).expect("a bytes frame");
                    self.output.extend_from_slice(f.data);
                }
                Message::Text(t) => {
                    let frame: Value = serde_json::from_str(t.as_str()).unwrap();
                    if frame["method"] == "terminal:exit" {
                        self.exits.push(frame["params"].clone());
                    }
                    return frame;
                }
                _ => {}
            }
        }
    }
}

/// The desktop's `+`: the card attaches the moment create answers, before the
/// shell is up, and must be told the shell is live -- not that it ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_shell_attached_at_once_is_live_and_echoes_what_is_typed() {
    let v = Vornd::start();
    let _reap = Reap(v.holder());
    let ws = common::connect(v.port, CREDENTIAL).await;
    let mut d = Desktop {
        ws,
        next_id: 0,
        output: Vec::new(),
        exits: Vec::new(),
    };
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap().to_owned();

    for round in 0..3 {
        let t = Instant::now();
        let created = loop {
            let answer = d.call("shell:create", json!(cwd)).await;
            if answer.get("error").is_none() {
                break answer["result"].clone();
            }
            // vornd reads its sessions once it is up.
            assert!(t.elapsed() < PATIENCE, "{answer}\n{}", v.log());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        let id = created["id"].as_str().expect("an id").to_owned();

        let attached = d.call("terminal:attach", json!({ "id": id })).await;
        assert_eq!(
            attached["result"]["live"],
            true,
            "round {round}: a shell being started was answered as ended: {attached}\n{}",
            v.log()
        );

        d.output.clear();
        let typed = d
            .call(
                "terminal:write",
                json!({ "id": id, "data": format!("echo vorn-$((6*{}))\r", 7 + round) }),
            )
            .await;
        assert!(typed.get("error").is_none(), "{typed}");
        let want = format!("vorn-{}", 6 * (7 + round));
        let t = Instant::now();
        while !String::from_utf8_lossy(&d.output).contains(&want) {
            assert!(
                t.elapsed() < PATIENCE,
                "round {round}: no echo; printed {:?}\n{}",
                String::from_utf8_lossy(&d.output),
                v.log()
            );
            let _ = tokio::time::timeout(std::time::Duration::from_millis(200), d.read()).await;
        }
        assert!(
            !d.exits.iter().any(|e| e["id"] == json!(id)),
            "the shell ended: {:?}",
            d.exits
        );
        d.call("terminal:kill", json!(id)).await;
    }
}
