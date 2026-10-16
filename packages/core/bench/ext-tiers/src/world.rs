//! What lies behind the gate in the bench: 32 terminals, two projects'
//! worktrees, and permission requests that resolve; for Tasks, a board in a
//! real store ([`Board`]). Every tier reaches the same one through the same gate.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::Notify;
use vorn_extensions::api::types::{Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, World};

use crate::tasks::Board;

/// The busy terminals.
pub const TERMINALS: usize = 32;

pub struct Fake {
    sessions: Vec<Value>,
    worktrees: Mutex<Vec<Value>>,
    /// The task board, when Tasks is measured.
    board: Option<Arc<Board>>,
    /// Core calls answered.
    pub calls: AtomicU64,
    /// Publishes that changed a row.
    pub published: AtomicU64,
    /// Woken on every publish.
    pub moved: Notify,
}

pub fn session_id(i: usize) -> String {
    format!("s{i}")
}

fn worktree(i: usize) -> Value {
    let project = if i.is_multiple_of(2) {
        "/p/api"
    } else {
        "/p/web"
    };
    let sessions: Vec<String> = if i.is_multiple_of(5) {
        vec![session_id(i)]
    } else {
        Vec::new()
    };
    json!({
        "path": format!("{project}/.worktrees/w{i}"),
        "projectPath": project,
        "projectName": project.trim_start_matches("/p/"),
        "branch": format!("feature/{i}"),
        "main": i == 0,
        "orphan": i == 7,
        "dirty": i.is_multiple_of(3),
        "sessionIds": sessions,
        "sizeBytes": 50_000_000 * (i as u64 + 1),
        "artifactBytes": if i.is_multiple_of(4) { 10_000_000 } else { 0 },
        "verdict": if i.is_multiple_of(3) { "keep" } else { "safe" },
        "reasons": ["merged"],
    })
}

impl Fake {
    /// The world with a task board too.
    pub fn with_board(board: Arc<Board>) -> Fake {
        Fake {
            board: Some(board),
            ..Fake::new()
        }
    }

    /// Counts a publish and wakes whoever waits for one.
    pub fn saw_publish(&self) {
        self.published.fetch_add(1, Ordering::SeqCst);
        self.moved.notify_waiters();
    }

    pub fn new() -> Fake {
        let sessions = (0..TERMINALS)
            .map(|i| {
                json!({
                    "id": session_id(i),
                    "name": format!("agent {i}"),
                    "agent": if i.is_multiple_of(2) { "claude" } else { "codex" },
                    "status": "working",
                })
            })
            .collect();
        Fake {
            sessions,
            worktrees: Mutex::new((0..20).map(worktree).collect()),
            board: None,
            calls: AtomicU64::new(0),
            published: AtomicU64::new(0),
            moved: Notify::new(),
        }
    }

    fn answer(&self, method: &str, args: &Value) -> Result<Value, ExtError> {
        let mut worktrees = self
            .worktrees
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(match method {
            "sessions.list" => Value::Array(self.sessions.clone()),
            "worktrees.list" => {
                let project = args["project"].as_str();
                Value::Array(
                    worktrees
                        .iter()
                        .filter(|w| project.is_none_or(|p| w["projectPath"] == p))
                        .cloned()
                        .collect(),
                )
            }
            "worktrees.remove" => {
                let path = args["worktree"].as_str().unwrap_or_default();
                let before = worktrees.len();
                worktrees.retain(|w| w["path"] != path);
                json!({ "removed": worktrees.len() < before })
            }
            "worktrees.reclaim" | "worktrees.prune" => {
                let path = args["worktree"].as_str().unwrap_or_default();
                if let Some(w) = worktrees.iter_mut().find(|w| w["path"] == path) {
                    w["artifactBytes"] = json!(0);
                }
                Value::Null
            }
            "sessions.close" | "permissions.resolve" => Value::Null,
            other => {
                return Err(ExtError::new(
                    "unknown-method",
                    format!("the bench has no {other}"),
                ))
            }
        })
    }
}

impl World for Fake {
    fn call(&self, _: &Caller, method: &str, args: Value) -> Answer<Value> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(board) = self.board.as_ref().filter(|_| Board::answers(method)) {
            // Off the runtime, as vornd reads its store.
            let (board, method) = (Arc::clone(board), method.to_owned());
            return Box::pin(async move {
                tokio::task::spawn_blocking(move || board.answer(&method, &args))
                    .await
                    .map_err(|e| ExtError::new("failed", e.to_string()))?
                    .map_err(|e| ExtError::new("failed", e))
            });
        }
        let answer = self.answer(method, &args);
        Box::pin(async move { answer })
    }

    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "the bench fetches nothing")) })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn published(&self, _: &Caller, _: &str, _: &[Value]) {
        self.saw_publish();
    }
}
