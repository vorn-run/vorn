//! The MCP tools' way to the server: one WebSocket to vornd's own `/ws`.
//!
//! Going through vornd's own listener rather than straight to the server means
//! a tool's call is routed as any client's is: a native group answers it here,
//! a terminal vornd holds is answered from the engine, and everything else is
//! forwarded. The TypeScript client opens a socket per call; one socket with
//! its requests multiplexed by id costs a handshake once instead of per call.
//! It is opened on first use and again after it drops, and a call in flight
//! when it drops fails the way the TypeScript client's does.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hyper::header::{HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;
use vorn_mcp::{Rpc, RpcError};

/// The server's close codes for a credential it does not accept.
const CLOSE_UNAUTHENTICATED: u16 = 4001;
const CLOSE_CREDENTIAL_REJECTED: u16 = 4002;
/// What a close without a frame reads as, as the `ws` package reports it.
const CLOSE_ABNORMAL: u16 = 1006;

type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// One open socket: where to write, and who waits for an answer on it.
struct Link {
    out: mpsc::UnboundedSender<Message>,
    waiters: Waiters,
    open: Arc<AtomicBool>,
}

/// A client of vornd's own `/ws`, presenting the local credential.
pub struct Loopback {
    addr: SocketAddr,
    /// `?topics=` on the connection, so only what its listener wants is sent.
    query: String,
    bearer: String,
    link: tokio::sync::Mutex<Option<Link>>,
    next_id: AtomicU64,
    /// The broadcasts the connection is sent, for whoever listens.
    notes: tokio::sync::broadcast::Sender<Value>,
}

/// Broadcasts kept for a slow listener before the oldest are dropped.
const NOTES_KEPT: usize = 4096;

impl Loopback {
    pub fn new(mut addr: SocketAddr, token: &[u8]) -> Loopback {
        // Listening on every address is reached on loopback.
        if addr.ip().is_unspecified() {
            addr.set_ip(match addr {
                SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
                SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
            });
        }
        Loopback {
            addr,
            query: String::new(),
            bearer: format!("Bearer {}", String::from_utf8_lossy(token)),
            link: tokio::sync::Mutex::new(None),
            next_id: AtomicU64::new(0),
            notes: tokio::sync::broadcast::channel(NOTES_KEPT).0,
        }
    }

    /// A client sent only the broadcasts `topics` name.
    pub fn with_topics(addr: SocketAddr, token: &[u8], topics: &[&str]) -> Loopback {
        let mut loopback = Loopback::new(addr, token);
        loopback.query = format!(
            "?topics={}",
            topics.join(",").replace(':', "%3A").replace(',', "%2C")
        );
        loopback
    }

    /// The broadcasts this connection is sent from now on, as
    /// `{method, params}`, while it is open.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Value> {
        self.notes.subscribe()
    }

    /// Opens the connection when it is not open.
    pub async fn connect(&self) -> Result<(), RpcError> {
        self.link().await.map(|_| ())
    }

    /// The open socket's writer and waiters, opening one when there is none.
    async fn link(&self) -> Result<(mpsc::UnboundedSender<Message>, Waiters), RpcError> {
        let mut link = self.link.lock().await;
        if let Some(l) = link.as_ref().filter(|l| l.open.load(Ordering::Acquire)) {
            return Ok((l.out.clone(), Arc::clone(&l.waiters)));
        }
        let opened = self.open().await?;
        let handles = (opened.out.clone(), Arc::clone(&opened.waiters));
        *link = Some(opened);
        Ok(handles)
    }

    async fn open(&self) -> Result<Link, RpcError> {
        let cannot = |err: &dyn std::fmt::Display| {
            RpcError(format!(
                "Cannot connect to Vorn server: {err}. Is the app running?"
            ))
        };
        let mut request = format!("ws://{}/ws{}", self.addr, self.query)
            .into_client_request()
            .map_err(|e| cannot(&e))?;
        let bearer = HeaderValue::from_str(&self.bearer).map_err(|e| cannot(&e))?;
        request.headers_mut().insert(AUTHORIZATION, bearer);
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| cannot(&e))?;
        let (mut sink, mut stream) = socket.split();
        let (out, mut outbox) = mpsc::unbounded_channel::<Message>();
        let waiters: Waiters = Arc::default();
        let open = Arc::new(AtomicBool::new(true));

        tokio::spawn(async move {
            while let Some(message) = outbox.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });
        let reading = Arc::clone(&waiters);
        let flag = Arc::clone(&open);
        let notes = self.notes.clone();
        tokio::spawn(async move {
            let mut code = CLOSE_ABNORMAL;
            while let Some(frame) = stream.next().await {
                match frame {
                    Ok(Message::Text(text)) => settle(&reading, &notes, text.as_str()),
                    Ok(Message::Close(close)) => {
                        code = close.map_or(CLOSE_ABNORMAL, |c| u16::from(c.code));
                        break;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        debug!(%err, "vornd's own connection failed");
                        break;
                    }
                }
            }
            flag.store(false, Ordering::Release);
            let message = closed_before_answering(code);
            let mut waiting = reading.lock().unwrap_or_else(|e| e.into_inner());
            for (_, waiter) in waiting.drain() {
                let _ = waiter.send(Err(RpcError(message.clone())));
            }
        });
        Ok(Link { out, waiters, open })
    }
}

/// Hands an answer to whoever waits for its id, and a broadcast to whoever
/// listens. A request of the server's is for neither.
fn settle(waiters: &Waiters, notes: &tokio::sync::broadcast::Sender<Value>, text: &str) {
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(id) = frame.get("id").and_then(Value::as_u64) else {
        if frame.get("method").is_some_and(Value::is_string) {
            let _ = notes.send(Value::Object(frame));
        }
        return;
    };
    if frame.contains_key("method") {
        return;
    }
    let waiter = waiters
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    let Some(waiter) = waiter else {
        return;
    };
    let answer = match frame.get("error") {
        Some(error) if !error.is_null() => Err(RpcError::answered(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        _ => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = waiter.send(answer);
}

/// `closedBeforeAnswering` from the TypeScript client.
fn closed_before_answering(code: u16) -> String {
    if code == CLOSE_UNAUTHENTICATED || code == CLOSE_CREDENTIAL_REJECTED {
        return format!(
            "vornd's own connection to the server was refused its credential (code {code})."
        );
    }
    format!("The server closed the connection before answering (code {code}).")
}

/// A JSON-RPC frame; `params` is left out when there are none, as
/// `JSON.stringify` leaves out `undefined`.
fn frame(id: Option<u64>, method: &str, params: Option<Value>) -> Message {
    let mut frame = json!({ "jsonrpc": "2.0" });
    if let Some(id) = id {
        frame["id"] = id.into();
    }
    frame["method"] = method.into();
    if let Some(params) = params {
        frame["params"] = params;
    }
    Message::text(frame.to_string())
}

impl Rpc for Loopback {
    async fn call(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let (out, waiters) = self.link().await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        if out.send(frame(Some(id), method, params)).is_err() {
            waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(RpcError(closed_before_answering(CLOSE_ABNORMAL)));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(RpcError(closed_before_answering(CLOSE_ABNORMAL))),
            Err(_) => {
                waiters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err(RpcError::timed_out(method, timeout))
            }
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), RpcError> {
        let (out, _) = self.link().await?;
        out.send(frame(None, method, params))
            .map_err(|_| RpcError(closed_before_answering(CLOSE_ABNORMAL)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_reach_their_caller_and_broadcasts_their_listeners() {
        let waiters: Waiters = Arc::default();
        let (tx, mut rx) = oneshot::channel();
        waiters.lock().unwrap().insert(7, tx);
        let notes = tokio::sync::broadcast::channel(4).0;
        let mut heard = notes.subscribe();
        settle(
            &waiters,
            &notes,
            r#"{"jsonrpc":"2.0","method":"task:changed","params":{}}"#,
        );
        settle(
            &waiters,
            &notes,
            r#"{"jsonrpc":"2.0","id":7,"method":"ui:ask","params":{}}"#,
        );
        settle(&waiters, &notes, "not json");
        assert!(
            rx.try_recv().is_err(),
            "a broadcast or a request is not an answer"
        );
        assert_eq!(heard.try_recv().unwrap()["method"], "task:changed");
        assert!(heard.try_recv().is_err(), "a request is not a broadcast");
        settle(
            &waiters,
            &notes,
            r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#,
        );
        assert_eq!(rx.try_recv().unwrap(), Ok(json!({ "ok": true })));
    }

    #[test]
    fn an_error_answer_reads_as_the_typescript_client_reads_it() {
        let waiters: Waiters = Arc::default();
        let (tx, mut rx) = oneshot::channel();
        waiters.lock().unwrap().insert(1, tx);
        settle(
            &waiters,
            &tokio::sync::broadcast::channel(1).0,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found: x:y"}}"#,
        );
        let err = rx.try_recv().unwrap().unwrap_err();
        assert!(err.0.starts_with("This server does not have x:y"), "{err}");
    }

    #[test]
    fn frames_leave_out_what_javascript_leaves_out() {
        assert_eq!(
            frame(Some(3), "config:load", None),
            Message::text(r#"{"jsonrpc":"2.0","id":3,"method":"config:load"}"#)
        );
        assert_eq!(
            frame(None, "a:b", Some(json!([1]))),
            Message::text(r#"{"jsonrpc":"2.0","method":"a:b","params":[1]}"#)
        );
    }
}
