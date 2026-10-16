//! The `/grid` WebSocket: grid mode ([`crate::grid`]) for a browser.
//!
//! The socket authenticates as `/ws` does: a credential on the upgrade, or
//! an `auth:authenticate` text frame first, within [`AUTH_TIMEOUT`], with
//! at most [`MAX_PENDING`] sockets waiting at a time. A refused credential
//! closes it with [`CLOSE_CREDENTIAL_REJECTED`], anything else first with
//! [`CLOSE_UNAUTHENTICATED`]. Once admitted (`auth:ok` for a credential
//! sent in a frame) its binary messages carry the grid protocol's frames
//! both ways, a frame possibly split across messages; text is ignored. The
//! socket ends with [`CLOSE_CREDENTIAL_REJECTED`] when its token is revoked.

use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

use super::clients::{close, CLOSE_CREDENTIAL_REJECTED, CLOSE_UNAUTHENTICATED};
use super::socket::{AUTH_TIMEOUT, MAX_PENDING};
use super::Serving;
use crate::engine::Engine;
use crate::native::reach::Authenticated;
use crate::native::Native;

/// How much of the grid's output waits for the browser before the grid's
/// own write timeout applies.
const BUFFER: usize = 256 << 10;

/// The error code for a refused `auth:authenticate`.
const NOT_AUTHENTICATED: i64 = -32001;

type Sink<C> = SplitSink<WebSocketStream<C>, Message>;
type Source<C> = SplitStream<WebSocketStream<C>>;

/// Serves `client` until it closes. `desktop` when it opened with the
/// desktop's launch token, which makes it a desktop to the size rule.
pub async fn run<C>(
    native: Arc<Native>,
    serving: Arc<Serving>,
    engine: Arc<Engine>,
    client: WebSocketStream<C>,
    desktop: bool,
    credential: Option<String>,
    peer: IpAddr,
) where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut source) = client.split();
    let Some(who) = authenticate(&native, &serving, &mut sink, &mut source, credential, peer).await
    else {
        let _ = sink.close().await;
        return;
    };
    let (id, revoked) = serving.clients.add_grid(who.token_id);
    let why = relay(&serving, engine, sink, source, desktop, &revoked).await;
    serving.clients.remove_grid(id);
    debug!(%why, "grid socket ended");
}

/// Who the socket authenticates as, or `None` once it is closed for not.
async fn authenticate<C>(
    native: &Arc<Native>,
    serving: &Serving,
    sink: &mut Sink<C>,
    source: &mut Source<C>,
    credential: Option<String>,
    peer: IpAddr,
) -> Option<Authenticated>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if let Some(raw) = credential {
        let who = check(native, raw, peer).await;
        if who.is_none() {
            refuse(sink, CLOSE_CREDENTIAL_REJECTED, "credential rejected").await;
        }
        return who;
    }
    if serving.pending.fetch_add(1, Ordering::AcqRel) >= MAX_PENDING {
        serving.pending.fetch_sub(1, Ordering::AcqRel);
        refuse(sink, CLOSE_UNAUTHENTICATED, "too many pending connections").await;
        return None;
    }
    let first = tokio::time::timeout(AUTH_TIMEOUT, async {
        loop {
            match source.next().await {
                Some(Ok(Message::Text(text))) => return Some(text),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    })
    .await;
    serving.pending.fetch_sub(1, Ordering::AcqRel);
    let text = match first {
        Ok(Some(text)) => text,
        Ok(None) => {
            refuse(sink, CLOSE_UNAUTHENTICATED, "grid before authentication").await;
            return None;
        }
        Err(_) => {
            refuse(sink, CLOSE_UNAUTHENTICATED, "authentication timeout").await;
            return None;
        }
    };
    let frame = serde_json::from_str::<Value>(text.as_str()).unwrap_or(Value::Null);
    let id = frame.get("id").cloned().unwrap_or(Value::Null);
    if frame.get("method").and_then(Value::as_str) != Some(crate::native::AUTH_METHOD) {
        reply(sink, &id, Err("Not authenticated")).await;
        refuse(sink, CLOSE_UNAUTHENTICATED, "method before authentication").await;
        return None;
    }
    let token = frame["params"]["token"].as_str().unwrap_or("").to_owned();
    let who = if token.is_empty() {
        None
    } else {
        check(native, token, peer).await
    };
    let Some(who) = who else {
        reply(sink, &id, Err("Authentication failed")).await;
        refuse(sink, CLOSE_CREDENTIAL_REJECTED, "invalid credential").await;
        return None;
    };
    let ok = json!({ "jsonrpc": "2.0", "method": "auth:ok", "params": { "userId": who.user_id } });
    if sink.send(Message::text(ok.to_string())).await.is_err() {
        return None;
    }
    reply(sink, &id, Ok(())).await;
    Some(who)
}

async fn check(native: &Arc<Native>, raw: String, peer: IpAddr) -> Option<Authenticated> {
    let n = Arc::clone(native);
    tokio::task::spawn_blocking(move || n.authenticate(&raw, peer))
        .await
        .ok()
        .flatten()
}

async fn reply<C>(sink: &mut Sink<C>, id: &Value, answer: Result<(), &str>)
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if id.is_null() {
        return;
    }
    let frame = match answer {
        Ok(()) => json!({ "jsonrpc": "2.0", "id": id, "result": { "ok": true } }),
        Err(message) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": NOT_AUTHENTICATED, "message": message },
        }),
    };
    let _ = sink.send(Message::text(frame.to_string())).await;
}

async fn refuse<C>(sink: &mut Sink<C>, code: u16, reason: &str)
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    warn!(reason, "refusing a grid socket");
    let _ = sink.send(close(code, reason)).await;
}

/// Carries the grid's frames between the socket and a grid connection
/// until either ends or the token is revoked; answers why it ended.
async fn relay<C>(
    serving: &Arc<Serving>,
    engine: Arc<Engine>,
    mut sink: Sink<C>,
    mut source: Source<C>,
    desktop: bool,
    revoked: &tokio::sync::Notify,
) -> String
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (ours, theirs) = tokio::io::duplex(BUFFER);
    let grid = tokio::spawn(async move {
        if desktop {
            crate::grid::serve_conn(theirs, engine, crate::grid::BUILD, crate::grid::instance())
                .await
        } else {
            crate::grid::serve_remote_conn(
                theirs,
                engine,
                crate::grid::BUILD,
                crate::grid::instance(),
            )
            .await
        }
    });
    let (mut from_grid, mut to_grid) = tokio::io::split(ours);
    let clients = Arc::clone(&serving.clients);
    // A separate task, so a grid parked writing to a slow browser never
    // stops the browser's input reaching it.
    let mut inbound = tokio::spawn(async move {
        while let Some(Ok(m)) = source.next().await {
            match m {
                Message::Binary(bytes) => {
                    clients.touch();
                    if to_grid.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = to_grid.shutdown().await;
    });
    let mut buf = vec![0u8; 64 << 10];
    let (why, farewell) = loop {
        tokio::select! {
            r = from_grid.read(&mut buf) => match r {
                Ok(0) | Err(_) => break ("the grid connection ended".to_owned(), Some(close(1000, ""))),
                Ok(n) => {
                    if sink.send(Message::binary(buf[..n].to_vec())).await.is_err() {
                        break ("the socket failed".to_owned(), None);
                    }
                }
            },
            () = revoked.notified() => {
                break ("token revoked".to_owned(), Some(close(CLOSE_CREDENTIAL_REJECTED, "token revoked")));
            }
            _ = &mut inbound => break ("the browser closed".to_owned(), None),
        }
    };
    if let Some(m) = farewell {
        let _ = sink.send(m).await;
    }
    let _ = sink.close().await;
    inbound.abort();
    drop(from_grid);
    let grid_why = grid.await.unwrap_or_default();
    format!("{why} ({grid_why})")
}
