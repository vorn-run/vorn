//! JSON-RPC 2.0 over vornd's `/ws`: one I/O thread owns the socket, calls
//! answer through a [`Pending`] the caller waits on or polls, and
//! notifications arrive in order on a channel. Nothing here blocks the UI
//! thread unless it chooses to wait.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::endpoint::Endpoint;

/// How long a call may take before [`Rpc::call`] gives up on it.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Why a call or the connection failed.
#[derive(Debug, Clone, PartialEq)]
pub enum RpcError {
    /// The socket could not be opened or admitted.
    Connect(String),
    /// The connection ended before the answer came.
    Closed,
    /// No answer within the time asked for.
    Timeout,
    /// vornd answered with an error.
    Server { code: i64, message: String },
    /// The answer was not the shape asked for.
    Decode(String),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::Connect(e) => write!(f, "could not connect to vornd: {e}"),
            RpcError::Closed => f.write_str("the connection to vornd closed"),
            RpcError::Timeout => f.write_str("vornd did not answer in time"),
            RpcError::Server { message, .. } => f.write_str(message),
            RpcError::Decode(e) => write!(f, "unexpected answer: {e}"),
        }
    }
}

impl std::error::Error for RpcError {}

/// What arrives without being asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Notification {
        method: String,
        params: Value,
    },
    /// The connection ended; no more events follow.
    Closed,
}

type Reply = std_mpsc::Sender<Result<Value, RpcError>>;

enum Out {
    Call {
        id: u64,
        method: String,
        params: Value,
        reply: Reply,
    },
    Notify {
        method: String,
        params: Value,
    },
}

/// A call on its way: wait for its answer or poll it from a frame loop.
#[derive(Debug)]
pub struct Pending(std_mpsc::Receiver<Result<Value, RpcError>>);

impl Pending {
    /// Blocks for the answer, at most `timeout`.
    pub fn wait(self, timeout: Duration) -> Result<Value, RpcError> {
        match self.0.recv_timeout(timeout) {
            Ok(r) => r,
            Err(std_mpsc::RecvTimeoutError::Timeout) => Err(RpcError::Timeout),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(RpcError::Closed),
        }
    }

    /// The answer, if it has come.
    pub fn poll(&self) -> Option<Result<Value, RpcError>> {
        match self.0.try_recv() {
            Ok(r) => Some(r),
            Err(std_mpsc::TryRecvError::Empty) => None,
            Err(std_mpsc::TryRecvError::Disconnected) => Some(Err(RpcError::Closed)),
        }
    }
}

/// A connection to vornd. Dropping it closes the socket.
pub struct Rpc {
    out: mpsc::UnboundedSender<Out>,
    next_id: AtomicU64,
}

/// Called from the I/O thread whenever an answer or an event arrives, so a
/// window can ask for a frame.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

impl Rpc {
    /// Opens `/ws` with the endpoint's token. Events go to the returned
    /// receiver; `wake` runs after each one and after each answer.
    pub fn connect(
        ep: &Endpoint,
        wake: Wake,
    ) -> Result<(Rpc, std_mpsc::Receiver<Event>), RpcError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        let mut req = ep
            .ws_url()
            .into_client_request()
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        let auth = format!("Bearer {}", ep.token)
            .parse()
            .map_err(|_| RpcError::Connect("the token is not a header value".into()))?;
        req.headers_mut().insert("authorization", auth);
        let ws = rt
            .block_on(async {
                tokio::time::timeout(CALL_TIMEOUT, tokio_tungstenite::connect_async(req)).await
            })
            .map_err(|_| RpcError::Timeout)?
            .map_err(|e| RpcError::Connect(e.to_string()))?
            .0;
        let (out, rx) = mpsc::unbounded_channel();
        let (events_tx, events) = std_mpsc::channel();
        std::thread::Builder::new()
            .name("vornd-rpc".into())
            .spawn(move || rt.block_on(io(ws, rx, events_tx, wake)))
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        Ok((
            Rpc {
                out,
                next_id: AtomicU64::new(1),
            },
            events,
        ))
    }

    /// Sends a call; its answer comes through the [`Pending`].
    pub fn request(&self, method: &str, params: Value) -> Pending {
        let (reply, rx) = std_mpsc::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let call = Out::Call {
            id,
            method: method.to_owned(),
            params,
            reply,
        };
        if let Err(mpsc::error::SendError(Out::Call { reply, .. })) = self.out.send(call) {
            let _ = reply.send(Err(RpcError::Closed));
        }
        Pending(rx)
    }

    /// Calls `method` and waits up to [`CALL_TIMEOUT`] for its result.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.request(method, params).wait(CALL_TIMEOUT)
    }

    /// [`Rpc::call`], its result read as `T`.
    pub fn call_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        decode(self.call(method, params)?)
    }

    /// Sends a notification, which has no answer.
    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.out.send(Out::Notify {
            method: method.to_owned(),
            params,
        });
    }
}

/// A result read as `T`.
pub fn decode<T: DeserializeOwned>(v: Value) -> Result<T, RpcError> {
    serde_json::from_value(v).map_err(|e| RpcError::Decode(e.to_string()))
}

/// One frame from vornd, sorted.
#[derive(Debug, PartialEq)]
enum Incoming {
    Answer {
        id: u64,
        result: Result<Value, RpcError>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A call vornd makes of the client, by its id.
    Call {
        id: Value,
        method: String,
    },
    Ignored,
}

fn classify(text: &str) -> Incoming {
    let Ok(mut frame) = serde_json::from_str::<Value>(text) else {
        return Incoming::Ignored;
    };
    let method = frame
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (frame.get("id").cloned(), method) {
        (Some(id), Some(method)) if !id.is_null() => Incoming::Call { id, method },
        (None | Some(Value::Null), Some(method)) => Incoming::Notification {
            method,
            params: frame
                .get_mut("params")
                .map(Value::take)
                .unwrap_or(Value::Null),
        },
        (Some(id), None) => {
            let Some(id) = id.as_u64() else {
                return Incoming::Ignored;
            };
            let result = match frame.get("error") {
                Some(e) => Err(RpcError::Server {
                    code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: e
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("vornd refused the call")
                        .to_owned(),
                }),
                None => Ok(frame
                    .get_mut("result")
                    .map(Value::take)
                    .unwrap_or(Value::Null)),
            };
            Incoming::Answer { id, result }
        }
        _ => Incoming::Ignored,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn io(
    mut ws: Socket,
    mut rx: mpsc::UnboundedReceiver<Out>,
    events: std_mpsc::Sender<Event>,
    wake: Wake,
) {
    let mut waiting: HashMap<u64, Reply> = HashMap::new();
    loop {
        tokio::select! {
            msg = ws.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                match classify(text.as_str()) {
                    Incoming::Answer { id, result } => {
                        if let Some(reply) = waiting.remove(&id) {
                            let _ = reply.send(result);
                            wake();
                        }
                    }
                    Incoming::Notification { method, params } => {
                        if events.send(Event::Notification { method, params }).is_ok() {
                            wake();
                        }
                    }
                    Incoming::Call { id, method } => {
                        let refusal = json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": -32601, "message": format!("Method not found: {method}")}});
                        if ws.send(Message::text(refusal.to_string())).await.is_err() {
                            break;
                        }
                    }
                    Incoming::Ignored => {}
                }
            }
            out = rx.recv() => {
                let Some(out) = out else {
                    let _ = ws.close(None).await;
                    break;
                };
                let frame = match out {
                    Out::Call { id, method, params, reply } => {
                        waiting.insert(id, reply);
                        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
                    }
                    Out::Notify { method, params } => {
                        json!({"jsonrpc": "2.0", "method": method, "params": params})
                    }
                };
                if ws.send(Message::text(frame.to_string())).await.is_err() {
                    break;
                }
            }
        }
    }
    for (_, reply) in waiting.drain() {
        let _ = reply.send(Err(RpcError::Closed));
    }
    let _ = events.send(Event::Closed);
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_carry_their_id() {
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":3,"result":{"ok":true}}"#),
            Incoming::Answer {
                id: 3,
                result: Ok(json!({"ok": true}))
            }
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":4}"#),
            Incoming::Answer {
                id: 4,
                result: Ok(Value::Null)
            }
        );
    }

    #[test]
    fn errors_keep_vornd_s_message() {
        let got = classify(r#"{"id":5,"error":{"code":-32000,"message":"no project"}}"#);
        assert_eq!(
            got,
            Incoming::Answer {
                id: 5,
                result: Err(RpcError::Server {
                    code: -32000,
                    message: "no project".into()
                })
            }
        );
    }

    #[test]
    fn notifications_and_calls_are_told_apart() {
        assert_eq!(
            classify(
                r#"{"jsonrpc":"2.0","method":"terminal:exit","params":{"id":"a","exitCode":0}}"#
            ),
            Incoming::Notification {
                method: "terminal:exit".into(),
                params: json!({"id": "a", "exitCode": 0})
            }
        );
        assert_eq!(
            classify(r#"{"jsonrpc":"2.0","id":"x","method":"bridge:ask"}"#),
            Incoming::Call {
                id: json!("x"),
                method: "bridge:ask".into()
            }
        );
        assert_eq!(classify("not json"), Incoming::Ignored);
        assert_eq!(classify(r#"{"id":"str"}"#), Incoming::Ignored);
    }

    #[test]
    fn a_closed_connection_fails_calls() {
        let (out, rx) = mpsc::unbounded_channel();
        drop(rx);
        let rpc = Rpc {
            out,
            next_id: AtomicU64::new(1),
        };
        assert_eq!(
            rpc.request("config:load", Value::Null)
                .wait(Duration::from_secs(1)),
            Err(RpcError::Closed)
        );
    }
}
