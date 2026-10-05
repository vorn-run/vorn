//! vornd as the Node server's process backend: the link.
//!
//! With the Native daemon switch on, the Node server stops spawning PTYs and
//! piped agents itself and asks vornd instead, over one WebSocket that vornd
//! opens to the server's own endpoint. The server keeps what it owns today
//! (names, groups, agents, workflows and the database); vornd owns the
//! processes, through sessiond, and the parse (Session Recovery Contract §3).
//!
//! The socket authenticates with the credential the app started the server
//! with, which the app hands vornd in [`TOKEN_ENV`] rather than on its
//! command line, and claims the role with [`IDENTIFY`], which the server
//! accepts only from that credential. Nothing a client of the server can call
//! changes. Over the link:
//!
//! - the server asks: `vornd:list`, `vornd:follow`, `vornd:spawn`,
//!   `vornd:write`, `vornd:resize`, `vornd:signal` and `vornd:closeStdin`;
//! - vornd tells: `vornd:records`, every record once it is applied, in order,
//!   for what the server still does with output itself (its scrollback and
//!   history, analysis lines, headless output), `vornd:effect`, every
//!   effect with its `effect_id` (RC §7), and `vornd:held` whenever the
//!   engine connects to a sessiond, so the server lists again and ends the
//!   sessions a sessiond that died took with it (RC §6 flow E).
//!
//! Delivery follows RC §7. Records and effects go out once per vornd life,
//! and a replay after a restart sends them again with the same positions and
//! ids; the server drops records below its cursor and effects it has seen.
//! While no server is linked (between an app restart and the server's
//! answer, say), what the engine produces waits in a [`Backlog`]: effects
//! are always kept, records past [`BACKLOG_BYTES`] are dropped oldest first,
//! and the server sees the hole in the offsets. The server asks for the
//! backlog with `vornd:follow` once it has listed the sessions, so no record
//! arrives for a session it has not taken on yet.

use std::collections::{BTreeMap, VecDeque};
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use vorn_engine::{Effect, EffectId};
use vorn_sessiond_wire::{valid_session_name, Io, Kind, Sig, SpawnSpec, Stdin};
use vorn_term_proto::{Entry, Record};

use crate::engine::{Engine, Tapped};
use crate::streams::{answer, refuse};

/// Where the app puts the server's credential for vornd. vornd takes it out
/// of its own environment at once, so nothing it starts inherits it.
pub const TOKEN_ENV: &str = crate::node_link_token_env();

/// The call that claims the link.
pub const IDENTIFY: &str = "vornd:identify";

/// What the link speaks; the server refuses a version it does not know.
pub const LINK_PROTOCOL: u64 = 1;

/// Records kept for a server that is not linked, past which the oldest are
/// dropped. Effects are kept whatever their number: they are few, and
/// dropping one would lose an exit.
pub const BACKLOG_BYTES: usize = 32 << 20;

/// How long to wait before dialling the server again.
const RETRY: Duration = Duration::from_millis(500);

/// How long the server has to answer the identify call.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// The default PTY size when the server names none.
const DEFAULT_SIZE: (u16, u16) = (80, 24);

/// Where the server is and how to prove the link is the app's.
#[derive(Clone)]
pub struct LinkConfig {
    pub upstream: SocketAddr,
    pub token: String,
}

impl std::fmt::Debug for LinkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the credential, not even in a debug log.
        f.debug_struct("LinkConfig")
            .field("upstream", &self.upstream)
            .finish_non_exhaustive()
    }
}

/// What the engine produced while no server was following, oldest first.
#[derive(Debug, Default)]
pub struct Backlog {
    items: VecDeque<Tapped>,
    /// Bytes of record data held.
    bytes: usize,
    /// Records dropped to stay under the cap, for the log.
    dropped: u64,
}

impl Backlog {
    /// Keeps `t`, dropping the oldest records before it while the record
    /// bytes held are over [`BACKLOG_BYTES`]. Effects are never dropped.
    pub fn push(&mut self, t: Tapped) {
        self.bytes += weight(&t);
        self.items.push_back(t);
        while self.bytes > BACKLOG_BYTES {
            // Never the batch just kept: the newest output is what the
            // server most needs, and a lone batch over the cap is all there is.
            let older = self.items.len() - 1;
            let Some(at) = self
                .items
                .iter()
                .take(older)
                .position(|t| matches!(t, Tapped::Records(..)))
            else {
                break;
            };
            if let Some(gone) = self.items.remove(at) {
                self.bytes -= weight(&gone);
                self.dropped += 1;
            }
        }
    }

    /// Everything held, oldest first, and how many record batches were
    /// dropped to make room.
    pub fn drain(&mut self) -> (Vec<Tapped>, u64) {
        self.bytes = 0;
        (
            self.items.drain(..).collect(),
            std::mem::take(&mut self.dropped),
        )
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The record bytes a tapped item holds; effects weigh nothing.
fn weight(t: &Tapped) -> usize {
    match t {
        Tapped::Records(_, entries) => entries
            .iter()
            .map(|e| match &e.rec {
                Record::Data { bytes, .. } => bytes.len(),
                _ => 0,
            })
            .sum(),
        Tapped::Effect(..) | Tapped::Held => 0,
    }
}

/// An effect's id as the server stores it: `session:epoch:rseq:index`.
/// Session names hold no colon ([`valid_session_name`], and sessiond's own
/// `xxxxxxxx-n`), so the four parts never run together.
pub fn effect_key(id: &EffectId) -> String {
    format!("{}:{}:{}:{}", id.session, id.epoch, id.rseq, id.index)
}

/// `vornd:records` for one applied batch. Data goes as base64, since a
/// batch may end inside a UTF-8 sequence that the next one completes.
pub fn records_note(session: &str, entries: &[Entry]) -> Value {
    let records: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut r = json!({
                "epoch": e.hdr.epoch,
                "rseq": e.hdr.rseq,
                "offset": e.hdr.start_offset,
            });
            match &e.rec {
                Record::Data { bytes, .. } => {
                    r["data"] = json!(data_encoding::BASE64.encode(bytes));
                }
                Record::Resize { cols, rows, .. } => r["resize"] = json!([cols, rows]),
                Record::Gap { lost_bytes, .. } => r["gap"] = json!(lost_bytes),
                Record::Exit { code, signal } => {
                    r["exit"] = json!({ "code": code, "signal": signal });
                }
            }
            r
        })
        .collect();
    note(
        "vornd:records",
        json!({ "id": session, "records": records }),
    )
}

/// `vornd:effect` for one effect. A clipboard write goes without its
/// contents: the server has nothing to do with them, and they are the one
/// effect that must never be repeated or kept.
pub fn effect_note(id: &EffectId, effect: &Effect) -> Value {
    let mut p = json!({
        "id": id.session,
        "effect": effect_key(id),
        "epoch": id.epoch,
        "rseq": id.rseq,
        "index": id.index,
    });
    match effect {
        Effect::Bell => p["kind"] = json!("bell"),
        Effect::Clipboard { .. } => p["kind"] = json!("clipboard"),
        Effect::Notify { title, body } => {
            p["kind"] = json!("notify");
            p["title"] = json!(title);
            p["body"] = json!(body);
        }
        Effect::Cwd(cwd) => {
            p["kind"] = json!("cwd");
            p["cwd"] = json!(cwd);
        }
        Effect::Status(status) => {
            p["kind"] = json!("status");
            p["status"] = json!(status);
        }
        Effect::Exit { code, signal } => {
            p["kind"] = json!("exit");
            p["code"] = json!(code);
            p["signal"] = json!(signal);
        }
    }
    note("vornd:effect", p)
}

fn note(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// A `vornd:spawn` as the server asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    /// The server's terminal id, which becomes the session's name.
    pub name: String,
    pub spec: SpawnSpec,
    /// For a piped agent: what to write to its stdin before closing it.
    pub stdin: Option<Vec<u8>>,
}

/// Reads `vornd:spawn {id, argv, cwd, env, cols?, rows?, piped?, shell?, stdin?}`.
///
/// `env` is the program's whole environment: sessiond lets a program inherit
/// its own only when the spec names none, and the server always names it.
/// `shell` runs `argv` joined into one line through the platform's shell, as
/// Node's `shell: true` does; the server has already quoted it for that shell.
pub fn parse_spawn(p: &Value) -> Result<SpawnRequest, String> {
    let name = p
        .get("id")
        .and_then(Value::as_str)
        .filter(|n| valid_session_name(n))
        .ok_or("vornd:spawn needs an id of letters, digits, - and _")?
        .to_owned();
    let argv: Vec<String> = p
        .get("argv")
        .and_then(Value::as_array)
        .ok_or("vornd:spawn needs argv")?
        .iter()
        .map(|a| {
            a.as_str()
                .map(str::to_owned)
                .ok_or("argv holds only strings")
        })
        .collect::<Result<_, _>>()?;
    if argv.is_empty() {
        return Err("vornd:spawn needs argv".into());
    }
    let argv = if p.get("shell").and_then(Value::as_bool) == Some(true) {
        shell_argv(&argv.join(" "))
    } else {
        argv
    };
    let cwd = p
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or("vornd:spawn needs a cwd")?
        .to_owned();
    // Sorted, so the same request always makes the same spec.
    let env: BTreeMap<String, String> = match p.get("env") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(Value::Object(m)) => m
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect(),
        Some(_) => return Err("env is an object of strings".into()),
    };
    let size = |k: &str, default: u16| -> Result<u16, String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|&n| n > 0)
                .ok_or_else(|| format!("{k} is a size from 1 to 65535")),
        }
    };
    let stdin = p
        .get("stdin")
        .and_then(Value::as_str)
        .map(|s| s.as_bytes().to_vec());
    let io = if p.get("piped").and_then(Value::as_bool) == Some(true) {
        Io::Piped {
            stdin: if stdin.is_some() {
                Stdin::Pipe
            } else {
                Stdin::Null
            },
        }
    } else {
        if stdin.is_some() {
            return Err("stdin is for a piped agent".into());
        }
        Io::Pty {
            cols: size("cols", DEFAULT_SIZE.0)?,
            rows: size("rows", DEFAULT_SIZE.1)?,
        }
    };
    Ok(SpawnRequest {
        name,
        spec: SpawnSpec {
            argv,
            cwd,
            env: env.into_iter().collect(),
            io,
            ring_bytes: None,
        },
        stdin,
    })
}

/// `line` through the platform's shell, the way Node runs `shell: true`.
#[cfg(windows)]
fn shell_argv(line: &str) -> Vec<String> {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_owned());
    vec![
        comspec,
        "/d".into(),
        "/s".into(),
        "/c".into(),
        line.to_owned(),
    ]
}

/// `line` through the platform's shell, the way Node runs `shell: true`.
#[cfg(not(windows))]
fn shell_argv(line: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), line.to_owned()]
}

/// A signal by the name the server uses for it.
pub fn parse_signal(name: &str) -> Option<Sig> {
    match name.to_ascii_lowercase().trim_start_matches("sig") {
        "hup" => Some(Sig::Hup),
        "int" => Some(Sig::Int),
        "term" => Some(Sig::Term),
        "kill" => Some(Sig::Kill),
        _ => None,
    }
}

/// An epoch for a session spawned under a name, which may have named an
/// earlier session: random, so no cursor from that one names a place in
/// this one, and never 0, the epoch of sessiond's own unnamed sessions.
fn fresh_epoch() -> u32 {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(COUNT.fetch_add(1, Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    (h.finish() as u32).max(1)
}

/// What the link reports in `vornd:list`: every session the current
/// sessiond holds, and the last few that ended, with how they ended.
pub fn list(engine: &Engine) -> Value {
    let report = engine.report();
    let held = engine.held();
    let briefs: BTreeMap<&str, &Value> = report["sessions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| Some((b["session"].as_str()?, b)))
                .collect()
        })
        .unwrap_or_default();
    let mut ids: Vec<&String> = held.keys().collect();
    ids.sort();
    let sessions: Vec<Value> = ids
        .into_iter()
        .map(|id| {
            let h = held[id];
            let b = briefs.get(id.as_str()).copied().unwrap_or(&Value::Null);
            json!({
                "id": id,
                "kind": match h.kind { Kind::Pty => "pty", Kind::Piped => "piped" },
                "pid": h.pid,
                "epoch": h.epoch,
                "state": b.get("state").cloned().unwrap_or(Value::Null),
                "cursor": b.get("cursor").cloned().unwrap_or(Value::Null),
                "cols": b.get("cols").cloned().unwrap_or(Value::Null),
                "rows": b.get("rows").cloned().unwrap_or(Value::Null),
                "exited": b.get("exited").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let ended: Vec<Value> = report["closed"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|b| !b["exited"].is_null())
                .map(|b| json!({ "id": b["session"], "exited": b["exited"] }))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "connected": report["connected"],
        "sessions": sessions,
        "ended": ended,
    })
}

/// Keeps the link to the server for as long as vornd runs: dials it once
/// the engine holds sessions, claims the link, serves it until it drops,
/// and dials again. `linked` says whether a server is linked now.
pub async fn run(engine: Arc<Engine>, cfg: LinkConfig, linked: watch::Sender<bool>) {
    let mut tap = engine.tap();
    let mut backlog = Backlog::default();
    let mut said_waiting = false;
    loop {
        if engine.connected() {
            match dial(&cfg).await {
                Ok(ws) => {
                    said_waiting = false;
                    let why = serve(&engine, ws, &mut tap, &mut backlog, &linked).await;
                    linked.send_replace(false);
                    info!(%why, "the server link ended; dialling again");
                }
                Err(e) if !said_waiting => {
                    said_waiting = true;
                    warn!(upstream = %cfg.upstream, %e, "cannot link to the server yet");
                }
                Err(e) => debug!(%e, "cannot link to the server yet"),
            }
        }
        if !collect(&mut tap, &mut backlog, RETRY).await {
            return;
        }
    }
}

/// Keeps what the engine taps for `dur`. False when the engine is gone.
async fn collect(
    tap: &mut mpsc::UnboundedReceiver<Tapped>,
    backlog: &mut Backlog,
    dur: Duration,
) -> bool {
    let deadline = tokio::time::sleep(dur);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => return true,
            t = tap.recv() => match t {
                Some(t) => backlog.push(t),
                None => return false,
            },
        }
    }
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn dial(cfg: &LinkConfig) -> Result<Ws, String> {
    let mut req = format!("ws://{}/ws", cfg.upstream)
        .into_client_request()
        .map_err(|e| e.to_string())?;
    let bearer = HeaderValue::from_str(&format!("Bearer {}", cfg.token))
        .map_err(|_| "the server credential is not a header value".to_owned())?;
    req.headers_mut().insert(header::AUTHORIZATION, bearer);
    let dialled = tokio::time::timeout(
        IDENTIFY_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(req, None, false),
    )
    .await
    .map_err(|_| "the server did not accept the link in time".to_owned())?;
    dialled.map(|(ws, _)| ws).map_err(|e| e.to_string())
}

/// Serves one linked socket until it drops; answers why it ended.
async fn serve(
    engine: &Arc<Engine>,
    ws: Ws,
    tap: &mut mpsc::UnboundedReceiver<Tapped>,
    backlog: &mut Backlog,
    linked: &watch::Sender<bool>,
) -> String {
    let (mut tx, mut rx) = ws.split();
    let identify = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": IDENTIFY,
        "params": { "protocol": LINK_PROTOCOL, "pid": std::process::id() },
    });
    if let Err(e) = tx.send(Message::text(identify.to_string())).await {
        return e.to_string();
    }
    // The server greets every socket first; the answer is the frame with
    // the identify call's id.
    let claimed = tokio::time::timeout(IDENTIFY_TIMEOUT, async {
        while let Some(frame) = rx.next().await {
            let Ok(Message::Text(text)) = frame else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            // An answer, not a call of the server's own that happens to
            // share the id.
            if v.get("id") != Some(&json!(1)) || v.get("method").is_some() {
                continue;
            }
            return match v.pointer("/result/ok") {
                Some(Value::Bool(true)) => Ok(()),
                _ => Err(v
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the server refused the link")
                    .to_owned()),
            };
        }
        Err("the server closed the link".to_owned())
    })
    .await;
    match claimed {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return e,
        Err(_) => return "the server did not answer the identify call".to_owned(),
    }
    info!("linked to the server as its process backend");
    linked.send_replace(true);

    let (reply_tx, mut replies) = mpsc::unbounded_channel::<Value>();
    let mut following = false;
    loop {
        tokio::select! {
            frame = rx.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let Ok(Value::Object(call)) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    let call = Value::Object(call);
                    let Some(method) = call.get("method").and_then(Value::as_str) else {
                        // An answer to nothing vornd asked.
                        continue;
                    };
                    if method == "vornd:follow" && !following {
                        following = true;
                        let (held, dropped) = backlog.drain();
                        if dropped > 0 {
                            warn!(dropped, "record batches dropped while no server was linked");
                        }
                        if let Some(rpc) = call.get("id") {
                            if let Err(e) = send(&mut tx, &answer(rpc, json!({ "ok": true }))).await {
                                return e;
                            }
                        }
                        for t in held {
                            if let Err(e) = send(&mut tx, &tapped_note(&t)).await {
                                return e;
                            }
                        }
                        continue;
                    }
                    if let Some(v) = handle(engine, method, &call, &reply_tx) {
                        if let Err(e) = send(&mut tx, &v).await {
                            return e;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => return "the server closed the link".to_owned(),
                Some(Err(e)) => return e.to_string(),
                Some(Ok(_)) => {}
            },
            Some(v) = replies.recv() => {
                if let Err(e) = send(&mut tx, &v).await {
                    return e;
                }
            }
            t = tap.recv() => match t {
                Some(t) if following => {
                    if let Err(e) = send(&mut tx, &tapped_note(&t)).await {
                        return e;
                    }
                }
                Some(t) => backlog.push(t),
                None => return "the engine stopped".to_owned(),
            },
        }
    }
}

async fn send(
    tx: &mut futures_util::stream::SplitSink<Ws, Message>,
    v: &Value,
) -> Result<(), String> {
    tx.send(Message::text(v.to_string()))
        .await
        .map_err(|e| e.to_string())
}

fn tapped_note(t: &Tapped) -> Value {
    match t {
        Tapped::Records(session, entries) => records_note(session, entries),
        Tapped::Effect(id, effect) => effect_note(id, effect),
        // The server lists again and ends what is gone.
        Tapped::Held => note("vornd:held", json!({})),
    }
}

/// Answers one call from the server, or nothing for a notification. A
/// spawn answers later, through `replies`.
fn handle(
    engine: &Arc<Engine>,
    method: &str,
    call: &Value,
    replies: &mpsc::UnboundedSender<Value>,
) -> Option<Value> {
    let rpc = call.get("id").cloned();
    let p = call.get("params").cloned().unwrap_or(Value::Null);
    let session = || {
        p.get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{method} needs an id"))
    };
    let done: Result<Value, String> = match method {
        "vornd:list" => Ok(list(engine)),
        "vornd:follow" => Ok(json!({ "ok": true })),
        "vornd:spawn" => {
            let req = match parse_spawn(&p) {
                Ok(r) => r,
                Err(e) => return rpc.map(|rpc| refuse(&rpc, &e)),
            };
            let (engine, replies) = (Arc::clone(engine), replies.clone());
            tokio::spawn(async move {
                let done = spawn(&engine, req).await;
                if let Some(rpc) = rpc {
                    let v = match done {
                        Ok(v) => answer(&rpc, v),
                        Err(e) => refuse(&rpc, &e),
                    };
                    let _ = replies.send(v);
                }
            });
            return None;
        }
        "vornd:write" => session().and_then(|id| {
            let data = p
                .get("data")
                .and_then(Value::as_str)
                .ok_or("vornd:write needs data")?;
            engine.write(&id, data.as_bytes().to_vec())?;
            Ok(Value::Null)
        }),
        "vornd:resize" => session().and_then(|id| {
            let size = |k: &str| {
                p.get(k)
                    .and_then(Value::as_u64)
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|&n| n > 0)
            };
            match (size("cols"), size("rows")) {
                (Some(cols), Some(rows)) => engine.resize(&id, cols, rows).map(|()| Value::Null),
                _ => Err("vornd:resize needs cols and rows from 1 to 65535".into()),
            }
        }),
        "vornd:signal" => session().and_then(|id| {
            let sig = p
                .get("signal")
                .and_then(Value::as_str)
                .and_then(parse_signal)
                .ok_or("vornd:signal needs hup, int, term or kill")?;
            engine.signal(&id, sig).map(|()| Value::Null)
        }),
        "vornd:closeStdin" => {
            session().and_then(|id| engine.close_stdin(&id).map(|()| Value::Null))
        }
        other => Err(format!("vornd does not answer {other}")),
    };
    let rpc = rpc?;
    Some(match done {
        Ok(v) => answer(&rpc, v),
        Err(e) => refuse(&rpc, &e),
    })
}

/// Spawns under the server's name and, for a piped agent given input,
/// writes it and closes stdin. The write follows the spawn on the engine's
/// one command queue, so it reaches the program.
async fn spawn(engine: &Engine, req: SpawnRequest) -> Result<Value, String> {
    let epoch = fresh_epoch();
    let held = engine.spawn_as(&req.name, epoch, req.spec).await?;
    if let Some(bytes) = req.stdin {
        if !bytes.is_empty() {
            engine.write(&req.name, bytes)?;
        }
        engine.close_stdin(&req.name)?;
    }
    Ok(json!({ "id": req.name, "pid": held.pid, "epoch": held.epoch }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::{GapReason, RecordHeader, Stream};

    fn entry(rseq: u64, offset: u64, rec: Record) -> Entry {
        Entry {
            hdr: RecordHeader {
                epoch: 7,
                rseq,
                start_offset: offset,
            },
            at_ns: 0,
            rec,
        }
    }

    fn data(bytes: &[u8]) -> Record {
        Record::Data {
            stream: Stream::Pty,
            bytes: bytes.to_vec(),
        }
    }

    fn fx(rseq: u64, index: u32) -> EffectId {
        EffectId {
            session: "s-1".into(),
            epoch: 7,
            rseq,
            index,
        }
    }

    #[test]
    fn records_carry_their_place_and_split_utf8_survives() {
        // "é" is c3 a9: a batch may end between the two.
        let note = records_note(
            "s-1",
            &[
                entry(4, 100, data(b"a\xc3")),
                entry(5, 102, data(b"\xa9")),
                entry(
                    6,
                    103,
                    Record::Resize {
                        cols: 100,
                        rows: 30,
                        px_w: 0,
                        px_h: 0,
                        req: None,
                    },
                ),
                entry(
                    7,
                    103,
                    Record::Gap {
                        lost_bytes: 9,
                        reason: GapReason::SpoolFull,
                    },
                ),
                entry(
                    8,
                    112,
                    Record::Exit {
                        code: Some(3),
                        signal: None,
                    },
                ),
            ],
        );
        assert_eq!(note["method"], "vornd:records");
        let r = &note["params"]["records"];
        assert_eq!(note["params"]["id"], "s-1");
        assert_eq!(r[0]["rseq"], 4);
        assert_eq!(r[0]["offset"], 100);
        assert_eq!(r[0]["epoch"], 7);
        let mut joined = data_encoding::BASE64
            .decode(r[0]["data"].as_str().unwrap().as_bytes())
            .unwrap();
        joined.extend(
            data_encoding::BASE64
                .decode(r[1]["data"].as_str().unwrap().as_bytes())
                .unwrap(),
        );
        assert_eq!(String::from_utf8(joined).unwrap(), "aé");
        assert_eq!(r[2]["resize"], json!([100, 30]));
        assert_eq!(r[3]["gap"], 9);
        assert_eq!(r[4]["exit"], json!({ "code": 3, "signal": null }));
    }

    #[test]
    fn effects_are_named_the_same_way_every_time() {
        let n = effect_note(
            &fx(9, 2),
            &Effect::Notify {
                title: "done".into(),
                body: "build passed".into(),
            },
        );
        assert_eq!(n["params"]["effect"], "s-1:7:9:2");
        assert_eq!(n["params"]["kind"], "notify");
        assert_eq!(n["params"]["title"], "done");
        assert_eq!(effect_key(&fx(9, 2)), "s-1:7:9:2");
        assert_ne!(effect_key(&fx(9, 2)), effect_key(&fx(9, 3)));
        let exit = effect_note(
            &fx(10, 0),
            &Effect::Exit {
                code: None,
                signal: Some(9),
            },
        );
        assert_eq!(exit["params"]["kind"], "exit");
        assert_eq!(exit["params"]["signal"], 9);
        assert_eq!(
            effect_note(&fx(1, 0), &Effect::Status(2))["params"]["status"],
            2
        );
    }

    #[test]
    fn a_clipboard_write_never_leaves_vornd() {
        let n = effect_note(
            &fx(3, 0),
            &Effect::Clipboard {
                location: vorn_screen::ClipboardTarget::Standard,
                contents: vec![("text/plain".into(), "secret".into())],
            },
        );
        assert_eq!(n["params"]["kind"], "clipboard");
        assert!(!n.to_string().contains("secret"));
    }

    #[test]
    fn the_backlog_keeps_every_effect_and_drops_the_oldest_records() {
        let mut b = Backlog::default();
        let big = vec![b'x'; BACKLOG_BYTES / 2 + 1];
        b.push(Tapped::Effect(fx(0, 0), Effect::Status(1)));
        b.push(Tapped::Records("s-1".into(), vec![entry(0, 0, data(&big))]));
        b.push(Tapped::Effect(fx(1, 0), Effect::Bell));
        b.push(Tapped::Records(
            "s-1".into(),
            vec![entry(1, big.len() as u64, data(&big))],
        ));
        // Over the cap: the first records go, both effects stay, in order.
        let (held, dropped) = b.drain();
        assert_eq!(dropped, 1);
        assert_eq!(held.len(), 3);
        assert!(matches!(held[0], Tapped::Effect(ref id, _) if id.rseq == 0));
        assert!(matches!(held[1], Tapped::Effect(ref id, _) if id.rseq == 1));
        assert!(matches!(&held[2], Tapped::Records(_, e) if e[0].hdr.rseq == 1));
        assert!(b.is_empty());
        // A lone batch over the cap is kept: nothing older to drop.
        b.push(Tapped::Records(
            "s-1".into(),
            vec![entry(0, 0, data(&vec![b'y'; BACKLOG_BYTES + 1]))],
        ));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn a_spawn_is_read_strictly() {
        let p = json!({
            "id": "0f9c2d1e-3b4a",
            "argv": ["/bin/zsh", "-l"],
            "cwd": "/w",
            "env": { "B": "2", "A": "1", "N": 3 },
            "cols": 100,
            "rows": 30,
        });
        let r = parse_spawn(&p).unwrap();
        assert_eq!(r.name, "0f9c2d1e-3b4a");
        assert_eq!(
            r.spec.io,
            Io::Pty {
                cols: 100,
                rows: 30
            }
        );
        // Sorted, strings only.
        assert_eq!(
            r.spec.env,
            vec![("A".into(), "1".into()), ("B".into(), "2".into())]
        );
        assert_eq!(r.stdin, None);

        let piped = parse_spawn(&json!({
            "id": "h1", "argv": ["claude", "-p"], "cwd": "/w", "piped": true, "stdin": "go",
        }))
        .unwrap();
        assert_eq!(piped.spec.io, Io::Piped { stdin: Stdin::Pipe });
        assert_eq!(piped.stdin.as_deref(), Some(&b"go"[..]));
        #[cfg(not(windows))]
        assert_eq!(
            parse_spawn(&json!({
                "id": "h3", "argv": ["gemini", "--model", "'a b'"], "cwd": "/w", "piped": true, "shell": true,
            }))
            .unwrap()
            .spec
            .argv,
            ["/bin/sh", "-c", "gemini --model 'a b'"]
        );
        let closed = parse_spawn(&json!({
            "id": "h2", "argv": ["codex"], "cwd": "/w", "piped": true,
        }))
        .unwrap();
        assert_eq!(closed.spec.io, Io::Piped { stdin: Stdin::Null });

        for bad in [
            json!({ "argv": ["sh"], "cwd": "/" }),
            json!({ "id": "../x", "argv": ["sh"], "cwd": "/" }),
            json!({ "id": "a", "argv": [], "cwd": "/" }),
            json!({ "id": "a", "argv": ["sh", 1], "cwd": "/" }),
            json!({ "id": "a", "argv": ["sh"] }),
            json!({ "id": "a", "argv": ["sh"], "cwd": "/", "cols": 0 }),
            json!({ "id": "a", "argv": ["sh"], "cwd": "/", "rows": 70000 }),
            json!({ "id": "a", "argv": ["sh"], "cwd": "/", "env": ["A=1"] }),
            json!({ "id": "a", "argv": ["sh"], "cwd": "/", "stdin": "x" }),
        ] {
            assert!(parse_spawn(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn signals_by_any_of_their_names() {
        assert_eq!(parse_signal("SIGHUP"), Some(Sig::Hup));
        assert_eq!(parse_signal("term"), Some(Sig::Term));
        assert_eq!(parse_signal("Kill"), Some(Sig::Kill));
        assert_eq!(parse_signal("int"), Some(Sig::Int));
        assert_eq!(parse_signal("usr1"), None);
    }

    #[test]
    fn a_fresh_epoch_is_never_sessionds_own() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let e = fresh_epoch();
            assert_ne!(e, 0);
            seen.insert(e);
        }
        assert!(seen.len() > 990, "epochs repeat: {}", seen.len());
    }

    #[test]
    fn the_credential_stays_out_of_debug_output() {
        let cfg = LinkConfig {
            upstream: ([127, 0, 0, 1], 1).into(),
            token: "hunter2".into(),
        };
        assert!(!format!("{cfg:?}").contains("hunter2"));
    }
}
