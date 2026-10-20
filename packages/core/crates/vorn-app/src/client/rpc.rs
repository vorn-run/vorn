//! JSON-RPC 2.0 over vornd's `/ws`: the [`link`] owns the socket, calls
//! answer through a [`Pending`] the caller waits on or polls, and
//! notifications arrive in order on a channel. Nothing here blocks the UI
//! thread unless it chooses to wait.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex, MutexGuard, PoisonError};
use web_time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;

use super::endpoint::Endpoint;
use super::link::{self, Ended, Frame, Inbound, Transport};

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
    /// vornd refused the credential.
    Rejected,
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
            RpcError::Rejected => f.write_str("vornd did not accept this device's key"),
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
    /// vornd asks the app something only it can answer (see
    /// [`answers_for_vornd`]); [`Rpc::respond`] settles it by `id`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The connection ended; no more events follow.
    Closed,
}

type Reply = std_mpsc::Sender<Result<Value, RpcError>>;

/// The calls vornd makes of the app once it holds the bridge: the browser
/// pane and the device pane.
pub fn answers_for_vornd(method: &str) -> bool {
    matches!(
        method.split_once(':').map(|(g, _)| g),
        Some("browser" | "device" | "session")
    )
}

/// A call on its way: wait for its answer or poll it from a frame loop.
#[derive(Debug)]
pub struct Pending(std_mpsc::Receiver<Result<Value, RpcError>>);

impl Pending {
    /// Blocks for the answer, at most `timeout`. Never in a browser, whose
    /// one thread is the one the answer arrives on: use [`Pending::answer`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait(self, timeout: Duration) -> Result<Value, RpcError> {
        match self.0.recv_timeout(timeout) {
            Ok(r) => r,
            Err(std_mpsc::RecvTimeoutError::Timeout) => Err(RpcError::Timeout),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(RpcError::Closed),
        }
    }

    /// The answer, waited for without holding the thread, at most `timeout`.
    #[cfg(target_arch = "wasm32")]
    pub async fn answer(self, timeout: Duration) -> Result<Value, RpcError> {
        let started = web_time::Instant::now();
        loop {
            if let Some(r) = self.poll() {
                return r;
            }
            if started.elapsed() >= timeout {
                return Err(RpcError::Timeout);
            }
            crate::page::sleep(Duration::from_millis(5)).await;
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

/// What both ends of the link share: the calls waiting for answers.
struct Core {
    waiting: Mutex<HashMap<u64, Reply>>,
    events: std_mpsc::Sender<Event>,
    /// Taken when the [`Rpc`] drops, which closes the link once no
    /// [`Responder`] holds it either.
    out: Mutex<Option<mpsc::UnboundedSender<Frame>>>,
    rejected: AtomicBool,
    wake: Wake,
}

impl Core {
    fn waiting(&self) -> MutexGuard<'_, HashMap<u64, Reply>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fail_waiting(&self, error: &RpcError) {
        for (_, reply) in self.waiting().drain() {
            let _ = reply.send(Err(error.clone()));
        }
    }

    fn out(&self) -> Option<mpsc::UnboundedSender<Frame>> {
        self.out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn send(&self, frame: Value) -> bool {
        self.out()
            .is_some_and(|out| out.send(Frame::Json(frame)).is_ok())
    }

    /// Settles what one frame from vornd says; false once the link is gone.
    fn incoming(&self, text: &str) -> bool {
        match classify(text) {
            Incoming::Answer { id, result } => {
                if let Some(reply) = self.waiting().remove(&id) {
                    let _ = reply.send(result);
                    (self.wake)();
                }
            }
            Incoming::Notification { method, params } => {
                if self
                    .events
                    .send(Event::Notification { method, params })
                    .is_ok()
                {
                    (self.wake)();
                }
            }
            Incoming::Call { id, method, params } if answers_for_vornd(&method) => {
                if self
                    .events
                    .send(Event::Request { id, method, params })
                    .is_ok()
                {
                    (self.wake)();
                }
            }
            Incoming::Call { id, method, .. } => {
                return self.send(json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("Method not found: {method}")}}));
            }
            Incoming::Ignored => {}
        }
        true
    }

    fn ended(&self, why: Ended) {
        let rejected = why == Ended::Rejected;
        self.rejected.store(rejected, Ordering::Relaxed);
        let error = if rejected {
            RpcError::Rejected
        } else {
            RpcError::Closed
        };
        // Taken first, so a call made from now on fails at once.
        self.out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.fail_waiting(&error);
        let _ = self.events.send(Event::Closed);
        (self.wake)();
    }
}

/// A connection to vornd. Dropping it closes the socket.
pub struct Rpc {
    core: Arc<Core>,
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
        #[cfg(not(target_arch = "wasm32"))]
        let link = link::Socket {
            url: ep.ws_url(),
            token: ep.token.clone(),
            pin: None,
        };
        #[cfg(target_arch = "wasm32")]
        let link = link::PageSocket {
            url: ep.ws_url(),
            token: ep.token.clone(),
        };
        Rpc::over(Box::new(link), wake)
    }

    /// Speaks JSON-RPC over `link`.
    pub fn over(
        link: Box<dyn Transport>,
        wake: Wake,
    ) -> Result<(Rpc, std_mpsc::Receiver<Event>), RpcError> {
        let (out, rx) = mpsc::unbounded_channel();
        let (events_tx, events) = std_mpsc::channel();
        let core = Arc::new(Core {
            waiting: Mutex::default(),
            events: events_tx,
            out: Mutex::new(Some(out)),
            rejected: AtomicBool::new(false),
            wake,
        });
        let (rd, end) = (Arc::clone(&core), Arc::clone(&core));
        link.start(
            rx,
            Inbound {
                frame: Box::new(move |f| match f {
                    Frame::Text(t) => rd.incoming(&t),
                    Frame::Binary(_) | Frame::Json(_) => true,
                }),
                ended: Box::new(move |why| end.ended(why)),
            },
        )
        .map_err(RpcError::Connect)?;
        Ok((
            Rpc {
                core,
                next_id: AtomicU64::new(1),
            },
            events,
        ))
    }

    /// Whether vornd refused this connection's credential.
    pub fn rejected(&self) -> bool {
        self.core.rejected.load(Ordering::Relaxed)
    }

    /// Sends a call; its answer comes through the [`Pending`].
    pub fn request(&self, method: &str, params: Value) -> Pending {
        let (reply, rx) = std_mpsc::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.core.waiting().insert(id, reply);
        // Built by hand: `json!` would copy `params`, which can be the whole config.
        let mut frame = Map::new();
        frame.insert("jsonrpc".into(), "2.0".into());
        frame.insert("id".into(), id.into());
        frame.insert("method".into(), method.into());
        frame.insert("params".into(), params);
        let sent = self.core.send(Value::Object(frame));
        if !sent {
            if let Some(reply) = self.core.waiting().remove(&id) {
                let _ = reply.send(Err(if self.rejected() {
                    RpcError::Rejected
                } else {
                    RpcError::Closed
                }));
            }
        }
        Pending(rx)
    }

    /// Calls `method` and waits up to [`CALL_TIMEOUT`] for its result.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.request(method, params).wait(CALL_TIMEOUT)
    }

    /// [`Rpc::call`], its result read as `T`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn call_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        decode(self.call(method, params)?)
    }

    /// Sends a notification, which has no answer.
    pub fn notify(&self, method: &str, params: Value) {
        self.core
            .send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Answers vornd's [`Event::Request`] `id`: its result, or an error
    /// message it hands to whoever asked.
    pub fn respond(&self, id: Value, result: Result<Value, String>) {
        respond(self.core.out(), id, result);
    }

    /// A handle that answers vornd's requests from another thread.
    pub fn responder(&self) -> Responder {
        Responder(self.core.out())
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        self.core
            .out
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // Nothing answers them now; their waiters must not spin forever.
        self.core.fail_waiting(&RpcError::Closed);
    }
}

fn respond(out: Option<mpsc::UnboundedSender<Frame>>, id: Value, result: Result<Value, String>) {
    let Some(out) = out else { return };
    let frame = match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(message) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})
        }
    };
    let _ = out.send(Frame::Text(frame.to_string()));
}

/// [`Rpc::respond`] for work that finishes off the UI thread.
#[derive(Clone)]
pub struct Responder(Option<mpsc::UnboundedSender<Frame>>);

impl Responder {
    pub fn respond(&self, id: Value, result: Result<Value, String>) {
        respond(self.0.clone(), id, result);
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
        params: Value,
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
        (Some(id), Some(method)) if !id.is_null() => Incoming::Call {
            id,
            method,
            params: frame
                .get_mut("params")
                .map(Value::take)
                .unwrap_or(Value::Null),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
                method: "bridge:ask".into(),
                params: Value::Null,
            }
        );
        assert!(answers_for_vornd("browser:readPage"));
        assert!(answers_for_vornd("device:list"));
        assert!(answers_for_vornd("session:fetch"));
        assert!(!answers_for_vornd("terminal:create"));
        assert_eq!(classify("not json"), Incoming::Ignored);
        assert_eq!(classify(r#"{"id":"str"}"#), Incoming::Ignored);
    }

    /// A link that answers each call with its own method, or ends at once.
    struct Fake(Option<link::Ended>);

    impl Transport for Fake {
        fn start(
            self: Box<Self>,
            mut out: mpsc::UnboundedReceiver<Frame>,
            mut inbound: Inbound,
        ) -> Result<(), String> {
            std::thread::spawn(move || {
                if let Some(why) = self.0 {
                    return (inbound.ended)(why);
                }
                while let Some(f) = out.blocking_recv() {
                    let v: Value = match f {
                        Frame::Text(t) => serde_json::from_str(&t).unwrap(),
                        Frame::Json(v) => v,
                        Frame::Binary(_) => break,
                    };
                    let reply = json!({"jsonrpc": "2.0", "id": v["id"], "result": v["method"]});
                    (inbound.frame)(Frame::Text(reply.to_string()));
                }
                (inbound.ended)(link::Ended::Closed);
            });
            Ok(())
        }
    }

    #[test]
    fn calls_are_answered_over_a_transport() {
        let (rpc, _events) = Rpc::over(Box::new(Fake(None)), Arc::new(|| {})).unwrap();
        assert_eq!(
            rpc.request("config:load", Value::Null)
                .wait(Duration::from_secs(5)),
            Ok(json!("config:load"))
        );
    }

    #[test]
    fn a_closed_or_refused_link_fails_calls() {
        for (why, err) in [
            (link::Ended::Closed, RpcError::Closed),
            (link::Ended::Rejected, RpcError::Rejected),
        ] {
            let (rpc, _events) = Rpc::over(Box::new(Fake(Some(why))), Arc::new(|| {})).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while rpc.core.out.lock().unwrap().is_some() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                rpc.request("config:load", Value::Null)
                    .wait(Duration::from_secs(5)),
                Err(err)
            );
        }
    }

    /// A link that takes calls and never answers them.
    struct Silent;

    impl Transport for Silent {
        fn start(
            self: Box<Self>,
            out: mpsc::UnboundedReceiver<Frame>,
            inbound: Inbound,
        ) -> Result<(), String> {
            std::mem::forget((out, inbound));
            Ok(())
        }
    }

    /// A call still waiting when its link is dropped (the app took a new
    /// one) fails, so whoever waits on it stops waiting.
    #[test]
    fn a_dropped_link_fails_the_calls_it_was_waiting_on() {
        let (rpc, _events) = Rpc::over(Box::new(Silent), Arc::new(|| {})).unwrap();
        let pending = rpc.request("config:load", Value::Null);
        assert_eq!(pending.poll(), None);
        drop(rpc);
        assert_eq!(pending.poll(), Some(Err(RpcError::Closed)));
    }
}
