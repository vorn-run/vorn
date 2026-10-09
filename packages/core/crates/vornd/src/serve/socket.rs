//! One client's WebSocket, with vornd as the server.
//!
//! Every socket is greeted first (`server:hello`, and `server:identity` for
//! one on this machine, which a desktop deciding whether to adopt this
//! server reads before it authenticates). A credential on the upgrade admits
//! it at once, or closes it if refused; without one, the only call taken is
//! `auth:authenticate`, within [`AUTH_TIMEOUT`], and at most
//! [`MAX_PENDING`] sockets wait at a time. Once admitted, `subscribe:set`
//! names the notifications it wants and the `server:` calls are answered
//! here; the rest go to [`crate::terminal`] or [`crate::native`].

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::clients::{close, Topics, CLOSE_CREDENTIAL_REJECTED, CLOSE_UNAUTHENTICATED};
use super::Serving;
use crate::endpoint::Daemon;
use crate::native::{Conn, Offer};
use crate::streams::Forwarder;

/// How long a socket has to authenticate.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// How many sockets may wait to authenticate at once.
pub const MAX_PENDING: usize = 64;
/// The error code for a call on a socket not authenticated.
const NOT_AUTHENTICATED: i64 = -32001;

/// Serves `client` until it closes.
pub async fn run<C>(
    daemon: Arc<Daemon>,
    serving: Arc<Serving>,
    client: WebSocketStream<C>,
    desktop: bool,
    credential: Option<String>,
    peer: SocketAddr,
    topics: Option<Topics>,
) where
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut to_client, mut from_client) = client.split();
    let mut conn = daemon.streams().connect();
    let id = conn.id();
    if desktop {
        daemon.mark_desktop(id);
    }
    let out = conn.forwarder();
    let writer = tokio::spawn(async move {
        while let Some(o) = conn.next().await {
            let size = o.size();
            let closing = matches!(o.msg, Message::Close(_));
            if to_client.send(o.msg).await.is_err() {
                break;
            }
            conn.written(size);
            if closing {
                break;
            }
        }
        let _ = to_client.close().await;
    });
    let native = Arc::clone(daemon.native());
    out.send_now(&serving.hello());
    if peer.ip().is_loopback() {
        let sessions = native.registry_live().and_then(|l| l["sessions"].as_u64());
        out.send_now(&serving.identity(sessions));
    }
    let calls = Conn::new(id, Arc::clone(&native), out.clone(), desktop);
    let mut session = Session {
        id,
        out: out.clone(),
        serving: Arc::clone(&serving),
        calls: Arc::clone(&calls),
        topics,
        admitted: false,
        waiting: false,
        ended: Arc::default(),
    };
    let mut deadline = None;
    match credential {
        Some(raw) => {
            let n = Arc::clone(&native);
            let presented = raw.clone();
            let who = tokio::task::spawn_blocking(move || n.authenticate(&presented))
                .await
                .ok()
                .flatten();
            match who {
                Some(who) => session.admit(&raw, who.token_id),
                None => session.refuse("credential rejected", CLOSE_CREDENTIAL_REJECTED),
            }
        }
        None if serving.pending.fetch_add(1, Ordering::AcqRel) >= MAX_PENDING => {
            serving.pending.fetch_sub(1, Ordering::AcqRel);
            session.refuse("too many pending connections", CLOSE_UNAUTHENTICATED);
        }
        None => {
            session.waiting = true;
            deadline = Some(tokio::time::Instant::now() + AUTH_TIMEOUT);
        }
    }
    let ended = Arc::clone(&session.ended);
    loop {
        // Refused or revoked: nothing more is read, whatever the client sends.
        let frame = match deadline.filter(|_| !session.admitted) {
            Some(at) => tokio::select! {
                frame = from_client.next() => frame,
                () = ended.notified() => break,
                () = tokio::time::sleep_until(at) => {
                    session.refuse("authentication timeout", CLOSE_UNAUTHENTICATED);
                    break;
                }
            },
            None => tokio::select! {
                frame = from_client.next() => frame,
                () = ended.notified() => break,
            },
        };
        let Some(Ok(frame)) = frame else {
            break;
        };
        match frame {
            Message::Text(text) => {
                if !session.take(&daemon, &native, text.as_str()).await {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    session.leave();
    calls.closed();
    // Ends the writer, which flushes the close handshake to the client.
    out.send_message_now(Message::Close(None));
    if tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .is_err()
    {
        tracing::debug!(id, "a client's writer did not finish");
    }
}

/// One socket's state.
struct Session {
    id: u64,
    out: Forwarder,
    serving: Arc<Serving>,
    calls: Arc<Conn>,
    /// What it asked for on the upgrade, until it is admitted.
    topics: Option<Topics>,
    admitted: bool,
    /// Counted among the sockets waiting to authenticate.
    waiting: bool,
    /// Ends the read loop once the socket is refused or its token revoked.
    ended: Arc<tokio::sync::Notify>,
}

impl Session {
    fn admit(&mut self, raw: &str, token: Option<String>) {
        self.calls.admit(raw);
        self.serving.clients.add(
            self.id,
            self.out.clone(),
            Arc::clone(&self.ended),
            self.topics.take(),
            token,
        );
        self.admitted = true;
        self.stop_waiting();
    }

    fn refuse(&mut self, reason: &str, code: u16) {
        tracing::warn!(reason, "refusing an unauthenticated socket");
        self.out.send_message_now(close(code, reason));
        self.ended.notify_one();
        self.stop_waiting();
    }

    fn stop_waiting(&mut self) {
        if std::mem::take(&mut self.waiting) {
            self.serving.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn leave(&mut self) {
        self.stop_waiting();
        if self.admitted {
            self.serving.clients.remove(self.id);
        }
    }

    fn reply(&self, id: &Value, answer: Result<Option<Value>, (i64, String)>) {
        if id.is_null() {
            return;
        }
        let frame = match answer {
            Ok(Some(result)) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Ok(None) => json!({ "jsonrpc": "2.0", "id": id }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.out.send_now(&frame);
    }

    /// Takes one text frame; false once the socket is to close.
    async fn take(
        &mut self,
        daemon: &Arc<Daemon>,
        native: &Arc<crate::native::Native>,
        text: &str,
    ) -> bool {
        let Ok(frame) = serde_json::from_str::<Value>(text) else {
            tracing::warn!("a client sent something that is not JSON");
            return true;
        };
        let method = frame.get("method").and_then(Value::as_str);
        let id = frame.get("id").cloned().unwrap_or(Value::Null);
        if !self.admitted {
            if method != Some(crate::native::AUTH_METHOD) {
                self.reply(&id, Err((NOT_AUTHENTICATED, "Not authenticated".into())));
                self.refuse("method before authentication", CLOSE_UNAUTHENTICATED);
                return false;
            }
            let token = frame["params"]["token"].as_str().unwrap_or("").to_owned();
            let n = Arc::clone(native);
            let presented = token.clone();
            let who = tokio::task::spawn_blocking(move || {
                (!presented.is_empty())
                    .then(|| n.authenticate(&presented))
                    .flatten()
            })
            .await
            .ok()
            .flatten();
            let Some(who) = who else {
                self.reply(
                    &id,
                    Err((NOT_AUTHENTICATED, "Authentication failed".into())),
                );
                self.refuse("invalid credential", CLOSE_CREDENTIAL_REJECTED);
                return false;
            };
            self.admit(&token, who.token_id);
            self.out.send_now(&json!({
                "jsonrpc": "2.0", "method": "auth:ok", "params": { "userId": who.user_id },
            }));
            self.reply(&id, Ok(Some(json!({ "ok": true }))));
            return true;
        }
        let Some(method) = method else {
            // An answer to a call vornd made of the desktop.
            self.calls.settle_desktop(text);
            return true;
        };
        if method != "bridge:identify" && method != "subscribe:set" {
            self.serving.clients.touch();
        }
        match method {
            "subscribe:set" => {
                if let Some(topics) = Topics::from_params(&frame["params"]) {
                    self.serving.clients.set_topics(self.id, topics);
                }
                self.reply(&id, Ok(Some(json!({ "ok": true }))));
                return true;
            }
            "server:vornd" => {
                let port = self.serving.addr().port();
                self.reply(&id, Ok(Some(json!({ "state": "on", "port": port }))));
                return true;
            }
            "server:shutdown" => {
                tracing::info!("asked to stop by a client");
                self.reply(&id, Ok(None));
                let serving = Arc::clone(&self.serving);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    serving.request_stop();
                });
                return true;
            }
            _ => {}
        }
        if daemon.answered_here(self.id, &self.out, text) {
            return true;
        }
        if method == crate::native::AUTH_METHOD || self.calls.offer(method, text) == Offer::Pass {
            self.reply(&id, Err((-32601, format!("Method not found: {method}"))));
        }
        true
    }
}
