//! The desktop app's main process, which answers what only it can.
//!
//! The browser pane's `<webview>` guests and the device registry live in the
//! desktop's main process, so `browser:*` and `device:*` are asked of it over
//! the socket it opened to vornd, as are the connectors' `session:*` calls
//! ([`crate::bridge::Bridge`]). Main claims that socket with
//! `bridge:identify`; vornd sends each call down it as a request of its own
//! and hands the answer to whoever asked.
//!
//! - One holder: the first admitted connection to claim. A second claim
//!   while the holder's connection is open is refused (`{ok: false}`); a
//!   claim over a closed one replaces it, and a claim again on the same
//!   connection is a no-op that succeeds.
//! - vornd's request ids are strings (`vornd-1`, ...), so they never meet
//!   main's own ids (numbers), which share the socket.
//! - Only the holder's connection can answer a pending request: any other
//!   socket could otherwise settle one by guessing its id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::streams::Forwarder;

/// How long a call waits for main, as the server waited.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// What a client hears with no main process connected, word for word.
pub const NOT_RUNNING: &str = "Vorn app is not running (no main process connected)";
/// What a pending call hears when main's connection closes.
pub const DISCONNECTED: &str = "Vorn main process disconnected";

const ID_PREFIX: &str = "vornd-";

/// The groups main answers.
pub fn answers(method: &str) -> bool {
    matches!(
        method.split_once(':').map(|(g, _)| g),
        Some("browser" | "device")
    )
}

type Reply = oneshot::Sender<Result<Option<Value>, String>>;

/// Main's connection, once it has claimed it.
#[derive(Debug)]
struct Holder {
    conn: u64,
    reply: Forwarder,
}

/// The desktop's main process, as vornd reaches it.
#[derive(Debug, Default)]
pub struct Desktop {
    holder: Mutex<Option<Holder>>,
    pending: Mutex<HashMap<String, Reply>>,
    next: AtomicU64,
}

impl Desktop {
    /// Connection `conn` says it is main's. False when another open
    /// connection already is.
    pub fn claim(&self, conn: u64, reply: &Forwarder) -> bool {
        let mut holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(held) = holder.as_ref() {
            if held.conn != conn && !held.reply.is_closed() {
                warn!("refused a second bridge:identify while one is live");
                return false;
            }
        }
        let replaced = holder.replace(Holder {
            conn,
            reply: reply.clone(),
        });
        drop(holder);
        if replaced.is_some_and(|h| h.conn != conn) {
            self.fail_pending(DISCONNECTED);
        }
        info!("the desktop's main process registered");
        true
    }

    /// Connection `conn` closed. When it was main's, every call waiting on
    /// main fails now rather than at its timeout.
    pub fn release(&self, conn: u64) {
        let mut holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
        if holder.as_ref().is_none_or(|h| h.conn != conn) {
            return;
        }
        *holder = None;
        drop(holder);
        self.fail_pending(DISCONNECTED);
    }

    /// Whether main's connection is open.
    pub fn connected(&self) -> bool {
        self.holder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|h| !h.reply.is_closed())
    }

    /// Whether `conn` is main's connection.
    pub fn holds(&self, conn: u64) -> bool {
        self.holder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|h| h.conn == conn)
    }

    /// Asks main `method` with `params` and waits up to `timeout` for its
    /// answer: `None` for one with no `result`, or main's error message.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Option<Value>, String> {
        let reply = {
            let holder = self.holder.lock().unwrap_or_else(|e| e.into_inner());
            match holder.as_ref().filter(|h| !h.reply.is_closed()) {
                Some(h) => h.reply.clone(),
                None => return Err(NOT_RUNNING.to_owned()),
            }
        };
        let id = format!(
            "{ID_PREFIX}{}",
            self.next.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (tx, rx) = oneshot::channel();
        self.lock_pending().insert(id.clone(), tx);
        let mut frame = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            frame["params"] = params;
        }
        reply.send_now(&frame);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(DISCONNECTED.to_owned()),
            Err(_) => {
                self.lock_pending().remove(&id);
                Err(format!("Browser request timed out: {method}"))
            }
        }
    }

    /// Settles a pending call with `frame`, a frame connection `conn` sent
    /// without a method. False when it was not an answer to vornd from main,
    /// so it goes on to the server.
    pub fn settle(&self, conn: u64, frame: &str) -> bool {
        if !frame.contains(ID_PREFIX) || !self.holds(conn) {
            return false;
        }
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(frame) else {
            return false;
        };
        if frame.contains_key("method") {
            return false;
        }
        let Some(id) = frame.get("id").and_then(Value::as_str) else {
            return false;
        };
        let Some(tx) = self.lock_pending().remove(id) else {
            return false;
        };
        let _ = tx.send(outcome(frame));
        true
    }

    fn fail_pending(&self, message: &str) {
        let pending = std::mem::take(&mut *self.lock_pending());
        for (_, tx) in pending {
            let _ = tx.send(Err(message.to_owned()));
        }
    }

    fn lock_pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, Reply>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Main's answer: its result, or its error's message.
fn outcome(mut frame: Map<String, Value>) -> Result<Option<Value>, String> {
    match frame.remove("error") {
        Some(error) => Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()),
        None => Ok(frame.remove("result")),
    }
}

impl crate::bridge::Bridge for Desktop {
    fn connected(&self) -> bool {
        Desktop::connected(self)
    }

    fn request<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        timeout: Duration,
    ) -> crate::bridge::Answer<'a> {
        Box::pin(async move {
            Desktop::request(self, method, params, timeout)
                .await
                .map(Option::unwrap_or_default)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streams::Streams;

    async fn sent(conn: &mut crate::streams::ClientConn) -> Value {
        let out = conn.next().await.expect("a frame");
        match out.msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => {
                serde_json::from_str(t.as_str()).expect("json")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn answers_only_the_browser_and_device_groups() {
        assert!(answers("browser:readPage"));
        assert!(answers("device:list"));
        assert!(!answers("bridge:identify"));
        assert!(!answers("browserless:x"));
    }

    #[tokio::test]
    async fn asks_main_and_hands_back_its_answer() {
        let streams = Streams::new();
        let mut main = streams.connect();
        let desktop = std::sync::Arc::new(Desktop::default());
        assert!(desktop.claim(main.id(), &main.forwarder()));
        assert!(desktop.connected());

        let asking = {
            let desktop = std::sync::Arc::clone(&desktop);
            tokio::spawn(async move {
                desktop
                    .request("browser:tabs", json!({ "sessionId": "s" }), TIMEOUT)
                    .await
            })
        };
        let frame = sent(&mut main).await;
        assert_eq!(frame["method"], "browser:tabs");
        assert_eq!(frame["params"], json!({ "sessionId": "s" }));
        let id = frame["id"].as_str().unwrap().to_owned();
        assert!(id.starts_with(ID_PREFIX));

        let answer = json!({ "jsonrpc": "2.0", "id": id, "result": [1] }).to_string();
        assert!(!desktop.settle(main.id() + 1, &answer), "only main answers");
        assert!(desktop.settle(main.id(), &answer));
        assert_eq!(asking.await.unwrap(), Ok(Some(json!([1]))));
        assert!(!desktop.settle(main.id(), &answer), "settled once");
    }

    #[tokio::test]
    async fn passes_on_main_errors_and_an_answer_without_a_result() {
        let streams = Streams::new();
        let mut main = streams.connect();
        let desktop = std::sync::Arc::new(Desktop::default());
        desktop.claim(main.id(), &main.forwarder());
        for (reply, expected) in [
            (
                json!({ "error": { "code": -32601, "message": "no such tab" } }),
                Err("no such tab".to_owned()),
            ),
            (json!({}), Ok(None)),
        ] {
            let asking = {
                let desktop = std::sync::Arc::clone(&desktop);
                tokio::spawn(
                    async move { desktop.request("device:list", Value::Null, TIMEOUT).await },
                )
            };
            let frame = sent(&mut main).await;
            assert!(frame.get("params").is_none());
            let mut answer = reply.clone();
            answer["jsonrpc"] = json!("2.0");
            answer["id"] = frame["id"].clone();
            assert!(desktop.settle(main.id(), &answer.to_string()));
            assert_eq!(asking.await.unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn refuses_without_main_and_after_its_deadline() {
        let desktop = Desktop::default();
        assert_eq!(
            desktop.request("browser:tabs", Value::Null, TIMEOUT).await,
            Err(NOT_RUNNING.to_owned())
        );
        let streams = Streams::new();
        let main = streams.connect();
        desktop.claim(main.id(), &main.forwarder());
        assert_eq!(
            desktop
                .request("browser:tabs", Value::Null, Duration::from_millis(5))
                .await,
            Err("Browser request timed out: browser:tabs".to_owned())
        );
        assert!(desktop.lock_pending().is_empty());
    }

    #[tokio::test]
    async fn one_live_holder_and_its_close_fails_what_waits() {
        let streams = Streams::new();
        let mut main = streams.connect();
        let other = streams.connect();
        let desktop = std::sync::Arc::new(Desktop::default());
        assert!(desktop.claim(main.id(), &main.forwarder()));
        assert!(desktop.claim(main.id(), &main.forwarder()), "again is fine");
        assert!(!desktop.claim(other.id(), &other.forwarder()));
        assert!(desktop.holds(main.id()) && !desktop.holds(other.id()));

        let asking = {
            let desktop = std::sync::Arc::clone(&desktop);
            tokio::spawn(async move { desktop.request("device:list", Value::Null, TIMEOUT).await })
        };
        sent(&mut main).await;
        desktop.release(other.id());
        assert!(desktop.connected(), "another connection closing is nothing");
        desktop.release(main.id());
        assert_eq!(asking.await.unwrap(), Err(DISCONNECTED.to_owned()));
        assert!(!desktop.connected());
        assert!(desktop.claim(other.id(), &other.forwarder()));
    }

    #[tokio::test]
    async fn a_closed_holder_is_replaced() {
        let streams = Streams::new();
        let main = streams.connect();
        let other = streams.connect();
        let desktop = Desktop::default();
        desktop.claim(main.id(), &main.forwarder());
        let main_id = main.id();
        drop(main);
        assert!(!desktop.connected());
        assert!(desktop.claim(other.id(), &other.forwarder()));
        assert!(!desktop.holds(main_id));
    }

    #[test]
    fn ignores_what_is_not_an_answer_from_main() {
        let desktop = Desktop::default();
        assert!(!desktop.settle(1, r#"{"id":"vornd-1","result":1}"#));
        let streams = Streams::new();
        let main = streams.connect();
        desktop.claim(main.id(), &main.forwarder());
        for frame in [
            r#"{"id":-1,"result":1}"#,
            r#"{"id":"vornd-9","result":1}"#,
            r#"{"id":"vornd-1","method":"x"}"#,
            "vornd- not json",
        ] {
            assert!(!desktop.settle(main.id(), frame), "{frame}");
        }
    }
}
