//! What lies behind the gate in the bench: 32 sessions, two projects'
//! worktrees and the feeds the fixture server holds, with a log of every
//! call so two tiers' runs can be compared call by call.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vorn_extensions::api::types::{Header, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, World};

/// The busy terminals.
pub const TERMINALS: usize = 32;
/// The worktrees across both projects.
pub const WORKTREES: usize = 20;
/// The host feeds are fetched from in the Wasm tier, which [`Bench::fetch`] serves from the fixtures.
pub const FEED_HOST: &str = "feeds.bench";

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Worktree `i`: every fifth has a session, every third has changes, number 7 is gone from disk.
pub fn worktree(i: usize) -> Value {
    let project = project(i);
    let sessions: Vec<String> = if i.is_multiple_of(5) { vec![format!("s{i}")] } else { Vec::new() };
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
        "verdict": if i.is_multiple_of(3) { "keep" } else { "remove" },
        "reasons": "merged",
    })
}

pub fn project(i: usize) -> &'static str {
    if i.is_multiple_of(2) {
        "/p/api"
    } else {
        "/p/web"
    }
}

fn session(i: usize) -> Value {
    let status = ["working", "idle", "waiting", "done"][i % 4];
    json!({ "id": format!("s{i}"), "name": format!("agent {i}"), "agent": "claude", "status": status })
}

/// The world two tiers of one extension are run against, counting what reaches it.
pub struct Bench {
    sessions: Vec<Value>,
    worktrees: Mutex<Vec<Value>>,
    /// Every call, in order: `(method, args)`.
    log: Mutex<Vec<(String, Value)>>,
    /// Whether calls and publishes are logged: on to compare tiers, off to measure them.
    logging: bool,
    fixtures: Option<SocketAddr>,
    pub calls: AtomicU64,
    pub published: AtomicU64,
}

impl Bench {
    /// A world whose fetches of [`FEED_HOST`] are served by the fixture server at `fixtures`.
    pub fn new(fixtures: Option<SocketAddr>) -> Bench {
        Bench {
            sessions: (0..TERMINALS).map(session).collect(),
            worktrees: Mutex::new((0..WORKTREES).map(worktree).collect()),
            log: Mutex::default(),
            logging: false,
            fixtures,
            calls: AtomicU64::new(0),
            published: AtomicU64::new(0),
        }
    }

    /// As [`Bench::new`], logging every call and publish.
    pub fn logged(fixtures: Option<SocketAddr>) -> Bench {
        Bench { logging: true, ..Bench::new(fixtures) }
    }

    /// The calls and publishes logged so far, and forgets them.
    pub fn take_log(&self) -> Vec<(String, Value)> {
        std::mem::take(&mut *guard(&self.log))
    }

    pub fn published(&self) -> u64 {
        self.published.load(Ordering::SeqCst)
    }

    fn answer(&self, method: &str, args: &Value) -> Result<Value, ExtError> {
        let mut worktrees = guard(&self.worktrees);
        match method {
            "sessions.list" => Ok(Value::Array(self.sessions.clone())),
            "sessions.close" | "permissions.resolve" | "worktrees.reclaim" => Ok(Value::Null),
            "worktrees.list" => {
                let project = args["project"].as_str();
                Ok(Value::Array(
                    worktrees.iter().filter(|w| project.is_none_or(|p| w["projectPath"] == p)).cloned().collect(),
                ))
            }
            "worktrees.remove" | "worktrees.prune" => {
                let path = args["worktree"].as_str().unwrap_or_default();
                let before = worktrees.len();
                worktrees.retain(|w| w["path"] != path);
                Ok(json!({ "removed": worktrees.len() < before }))
            }
            other => Err(ExtError::new("unknown-method", format!("the bench has no {other}"))),
        }
    }
}

impl World for Bench {
    fn call(&self, _: &Caller, method: &str, args: Value) -> Answer<Value> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let answer = self.answer(method, &args);
        if self.logging {
            guard(&self.log).push((method.to_owned(), args));
        }
        Box::pin(async move { answer })
    }

    fn fetch(&self, _: &Caller, request: Request) -> Answer<Response> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let fixtures = self.fixtures;
        Box::pin(async move {
            let addr = fixtures.ok_or_else(|| ExtError::new("network", "the bench serves no feeds"))?;
            let path = request
                .url
                .strip_prefix(&format!("https://{FEED_HOST}"))
                .ok_or_else(|| ExtError::new("network", format!("the bench serves no {}", request.url)))?;
            get(addr, path, &request.headers).await.map_err(|e| ExtError::new("network", e.to_string()))
        })
    }

    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }

    fn published(&self, _: &Caller, collection: &str, ops: &[Value]) {
        self.published.fetch_add(1, Ordering::SeqCst);
        if self.logging {
            guard(&self.log).push((format!("publish {collection}"), Value::Array(ops.to_vec())));
        }
    }
}

/// `GET path` from the fixture server, which closes the connection after each answer.
async fn get(addr: SocketAddr, path: &str, headers: &[Header]) -> std::io::Result<Response> {
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let mut head = format!("GET {path} HTTP/1.1\r\nHost: {FEED_HOST}\r\nConnection: close\r\n");
    for h in headers {
        head.push_str(&format!("{}: {}\r\n", h.name, h.value));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).unwrap_or(502);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| Header { name: n.trim().to_ascii_lowercase(), value: v.trim().to_owned() })
        .collect();
    Ok(Response { status, headers, body: body.to_owned() })
}
