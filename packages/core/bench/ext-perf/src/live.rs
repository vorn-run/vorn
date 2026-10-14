//! A client of a test vornd that stamps every answer and broadcast the moment it arrives, reading only what the bench needs of it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use vorn_app::client::Endpoint;

/// How long a call may take before the bench gives up on it.
const PATIENCE: Duration = Duration::from_secs(120);

/// One step's state, as a run broadcast carries it.
#[derive(Debug, Clone, Deserialize)]
pub struct StepFrame {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub status: String,
    #[serde(rename = "startedAt", default)]
    pub started_at: Option<String>,
}

/// What the bench reads of `workflow:runUpdated`.
#[derive(Debug, Clone, Deserialize)]
pub struct RunFrame {
    #[serde(rename = "runId")]
    pub run_id: String,
    #[serde(rename = "workflowId")]
    pub workflow_id: String,
    pub status: String,
    #[serde(rename = "startedAt", default)]
    pub started_at: Option<String>,
    #[serde(rename = "completedAt", default)]
    pub completed_at: Option<String>,
    #[serde(rename = "nodeStates", default)]
    pub steps: Vec<StepFrame>,
}

/// A broadcast and when it arrived.
#[derive(Debug, Clone)]
pub struct Note {
    pub at: Instant,
    /// Unix milliseconds at arrival, to set against a schedule.
    pub wall_ms: f64,
    pub method: String,
    pub run: Option<RunFrame>,
    pub bytes: usize,
}

#[derive(Deserialize)]
struct Frame<'a> {
    id: Option<u64>,
    method: Option<&'a str>,
    #[serde(borrow)]
    params: Option<&'a RawValue>,
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    #[serde(borrow)]
    error: Option<&'a RawValue>,
}

/// An answer and when it arrived.
pub type Answered = (Instant, Result<Value, String>);
type Reply = oneshot::Sender<Answered>;

/// One connection to vornd's `/ws`.
pub struct Live {
    out: mpsc::UnboundedSender<Message>,
    waiting: Arc<Mutex<HashMap<u64, Reply>>>,
    next: AtomicU64,
    pub notes: mpsc::UnboundedReceiver<Note>,
}

pub fn wall_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

impl Live {
    /// Connects with the token the data directory publishes.
    pub async fn connect(home: &Path) -> Result<Live, String> {
        let ep = Endpoint::find(home).map_err(|e| e.to_string())?;
        let mut req = ep
            .ws_url()
            .into_client_request()
            .map_err(|e| e.to_string())?;
        let auth = format!("Bearer {}", ep.token)
            .parse()
            .map_err(|_| "token".to_owned())?;
        req.headers_mut().insert("authorization", auth);
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| e.to_string())?;
        let (mut sink, mut stream) = ws.split();
        let (out, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let (notes_tx, notes) = mpsc::unbounded_channel();
        let waiting: Arc<Mutex<HashMap<u64, Reply>>> = Arc::default();
        tokio::spawn(async move {
            while let Some(m) = outgoing.recv().await {
                if sink.send(m).await.is_err() {
                    return;
                }
            }
        });
        let answers = Arc::clone(&waiting);
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let at = Instant::now();
                let Message::Text(text) = msg else { continue };
                let Ok(frame) = serde_json::from_str::<Frame<'_>>(&text) else {
                    continue;
                };
                if let (Some(id), None) = (frame.id, frame.method) {
                    let reply = answers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                    if let Some(reply) = reply {
                        let answer = match (frame.result, frame.error) {
                            (_, Some(e)) => Err(e.get().to_owned()),
                            (Some(r), None) => {
                                serde_json::from_str(r.get()).map_err(|e| e.to_string())
                            }
                            (None, None) => Ok(Value::Null),
                        };
                        let _ = reply.send((at, answer));
                    }
                    continue;
                }
                let Some(method) = frame.method else { continue };
                let run = (method == "workflow:runUpdated")
                    .then(|| {
                        frame
                            .params
                            .and_then(|p| serde_json::from_str::<RunFrame>(p.get()).ok())
                    })
                    .flatten();
                let note = Note {
                    at,
                    wall_ms: wall_ms(),
                    method: method.to_owned(),
                    run,
                    bytes: text.len(),
                };
                if notes_tx.send(note).is_err() {
                    return;
                }
            }
        });
        Ok(Live {
            out,
            waiting,
            next: AtomicU64::new(1),
            notes,
        })
    }

    /// Sends a call now: when it left, and where its answer will come with the time it arrived.
    pub fn send(&self, method: &str, params: Value) -> (Instant, oneshot::Receiver<Answered>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let sent = Instant::now();
        let _ = self.out.send(Message::text(frame.to_string()));
        (sent, rx)
    }

    /// Calls and waits: the answer and its round trip.
    pub async fn call(&self, method: &str, params: Value) -> Result<(Value, Duration), String> {
        let (sent, rx) = self.send(method, params);
        let (at, answer) = tokio::time::timeout(PATIENCE, rx)
            .await
            .map_err(|_| format!("{method} took too long"))?
            .map_err(|_| format!("{method}: the connection closed"))?;
        Ok((answer.map_err(|e| format!("{method}: {e}"))?, at - sent))
    }

    /// Drops the broadcasts that already arrived.
    pub fn drain(&mut self) {
        while self.notes.try_recv().is_ok() {}
    }

    /// The next broadcast named `method` within `limit`.
    pub async fn wait_for(&mut self, method: &str, limit: Duration) -> Option<Note> {
        let until = tokio::time::Instant::now() + limit;
        loop {
            let note = tokio::time::timeout_at(until, self.notes.recv())
                .await
                .ok()??;
            if note.method == method {
                return Some(note);
            }
        }
    }
}
