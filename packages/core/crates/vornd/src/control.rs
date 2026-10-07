//! The app's channel: how the app's server starts, drives and hears about
//! the sessions vornd holds.
//!
//! The server does not start terminals itself. It asks vornd here, and vornd
//! starts them in sessiond, so they outlive the server and the app. The
//! server keeps what it owns (names, groups, agents, workflows, the
//! database) and is told what each session's output meant as effects
//! ([`crate::journal`]): never the output itself, except for sessions it
//! asks to read, which it attaches to as any bytes client does.
//!
//! The endpoint is a local socket only this user can open, like the grid
//! endpoint ([`crate::grid`]). vornd names it in `run/vornd-app` under its
//! home ([`announce`]), which is where the server looks for it. A frame is a
//! 4-byte little-endian length, then a kind byte (1 for JSON, 2 for a bytes
//! frame), then the payload; the length counts the kind byte.
//!
//! The JSON is the clients' JSON-RPC. The terminal calls are the ones a
//! client sends ([`crate::terminal`]), except that the server's never count
//! toward the size rule and its `terminal:resize` is applied as it is, and
//! `vornd:spawn` is always answered. Beside them:
//!
//! - `vornd:hello` answers `{protocol, build, native, statuses,
//!   terminals, headless}`; `native` says vornd runs native work and keeps a copy of
//!   the server's session records ([`crate::registry`]), which the server
//!   then feeds, `statuses` that the copy decides the terminals' statuses,
//!   which the server then takes from it, `terminals` that vornd
//!   creates, closes and changes terminals for the clients itself, and
//!   `headless` that it starts and stops headless agents for them
//!   ([`crate::native::sessions`], [`crate::native::headless`]), which the
//!   server then follows from the copy's notes.
//! - `vornd:subscribe` answers `{connected, sessions, ended, notices}`: every
//!   session held with its latest states, the sessions that ended lately and
//!   the notifications kept, and with `native` also `registry`, the copy of
//!   the server's records. From then on the connection is sent each
//!   effect as `vornd:effect`, `vornd:activity {id}` while a session prints,
//!   `vornd:connected` when the engine (re)connects to sessiond, after
//!   which the server subscribes again, and each change to the copy as
//!   `vornd:session {gen, rev, op, ...}`. Anything may arrive twice: states
//!   carry the record they reflect, and the server drops notifications and
//!   exits it has already acted on by their `effectId`.
//! - `vornd:record {op, ...}`, a notification, is the server telling the
//!   copy what it changed: `snapshot` when it connects, then `upsert`,
//!   `remove`, `order` and `holds`.
//! - `vornd:registry` answers the copy whole, for a subscriber that missed
//!   a change.
//! - While the copy decides the statuses, the server tells it what only the
//!   server sees, as notifications: `vornd:hookStatus {id, status?, promote}`,
//!   the status an agent's hook reported and whether its hooks report the
//!   status from now on; `vornd:input {id}`, the server wrote to the
//!   terminal; and `vornd:patch {id, fields, baseRev?}`, fields of the
//!   record the server no longer sets itself (a hook session linked).
//! - `vornd:kill {id, signal}` signals the session's program (`hup`,
//!   `term`, `kill` or `int`).
//! - `vornd:closeStdin {id}` ends a piped session's input.
//! - `vornd:reach {host}` says where the server is bound (`0.0.0.0` when it
//!   takes connections from the network), and that the names a browser may
//!   load the web client from are to be read again.
//! - While vornd creates terminals, the server says what bears on that:
//!   `vornd:draining {draining, handingOver}`, a notification, whether it
//!   is winding down; and the conversations its own starts take, in the
//!   claims vornd's creates check ([`crate::claims`]): `vornd:claim
//!   {transcriptId, sessionId}` answers `{holder}`, the session already
//!   starting on it or null when the claim is taken, and the notifications
//!   `vornd:unclaim {sessionId, transcriptId?}`, `vornd:preparing
//!   {sessionId}` and `vornd:prepared {sessionId}`.
//!
//! A subscribed connection is also sent what vornd asks of the server
//! ([`crate::applink`]): `vornd:broadcast {method, params}`, a notification
//! for every client; `vornd:tokenRevoked {tokenId}`, after which the server
//! closes the sockets that authenticated with that token; and
//! `vornd:cleanupOffer {id, projectPath, worktreePath}`, the offer to clean
//! up a worktree whose last terminal vornd closed, for every client; and
//! `vornd:worktreesCleaned {paths}`, the worktrees a cleanup vornd ran
//! removed or emptied, whose sizes the server forgets.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use vorn_engine::{Effect, EffectId};
use vorn_sessiond::os;
use vorn_sessiond_wire::Sig;

use crate::applink::{AppLink, Closing};
use crate::engine::{Engine, Event};
use crate::journal::{Held, Stamped};
use crate::registry::{HookStatus, Patch, TerminalSession};
use crate::streams::{answer, exit_code, refuse, Forwarder};
use serde::Deserialize;

/// The version of this channel, which `vornd:hello` reports.
pub const APP_PROTOCOL: u64 = 1;

/// The file under `run/` that names the endpoint.
pub const ANNOUNCEMENT: &str = "vornd-app";

/// A JSON frame.
pub const KIND_TEXT: u8 = 1;
/// A bytes frame (Terminal State Protocol §14).
pub const KIND_BINARY: u8 = 2;

/// The largest frame either side accepts.
pub const MAX_FRAME: usize = 128 << 20;

/// The endpoint for this vornd under `home`: one per process, as the grid
/// endpoint is.
pub fn endpoint(home: &Path) -> String {
    let pid = std::process::id();
    #[cfg(unix)]
    {
        home.join("run")
            .join(format!("vornd-app-{pid}.sock"))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        let _ = home;
        format!(
            r"\\.\pipe\vorn-app-{}-{pid}",
            os::user_sid().unwrap_or_else(|_| "user".into())
        )
    }
}

fn announcement(home: &Path) -> PathBuf {
    home.join("run").join(ANNOUNCEMENT)
}

/// Names `endpoint` in `run/vornd-app`, written whole and renamed into
/// place, so a reader finds this vornd's endpoint or the last one's, never
/// half of one. On Windows the endpoint is a pipe, so nothing else has
/// made `run/` on a fresh home.
pub fn announce(home: &Path, endpoint: &str) -> std::io::Result<()> {
    let file = announcement(home);
    std::fs::create_dir_all(home.join("run"))?;
    let body = json!({
        "pid": std::process::id(),
        "endpoint": endpoint,
        "protocol": APP_PROTOCOL,
    });
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, body.to_string())?;
    // Windows refuses to replace a file for a moment while something has it
    // open, a reader or a scanner.
    let mut retries = if cfg!(windows) { 20 } else { 0 };
    loop {
        match std::fs::rename(&tmp, &file) {
            Err(e) if retries > 0 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                retries -= 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            r => return r,
        }
    }
}

/// Takes the announcement back, if it is still this vornd's.
pub fn withdraw(home: &Path) {
    let file = announcement(home);
    let ours = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("pid").and_then(Value::as_u64))
        == Some(u64::from(std::process::id()));
    if ours {
        let _ = std::fs::remove_file(file);
    }
}

/// Serves the app's channel on `listener` until it fails.
pub async fn serve(mut listener: os::Listener, engine: Arc<Engine>, link: Arc<AppLink>) {
    loop {
        match listener.accept().await {
            Ok(stream) => {
                let (engine, link) = (Arc::clone(&engine), Arc::clone(&link));
                tokio::spawn(async move {
                    let why = serve_conn(stream, engine, link).await;
                    info!(%why, "the app's channel closed");
                });
            }
            Err(e) => {
                warn!(%e, "the app's endpoint failed");
                listener.close();
                return;
            }
        }
    }
}

/// One connection from the app, until it ends; answers why it ended.
pub async fn serve_conn<S>(stream: S, engine: Arc<Engine>, link: Arc<AppLink>) -> String
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(stream);
    let mut conn = engine.streams().connect();
    let id = conn.id();
    let fwd = conn.forwarder();
    let subscribed = Arc::new(AtomicBool::new(false));
    // Taken before anything is answered, so nothing that happens from here
    // on is missed by a subscription; what it repeats, the app drops.
    let events = engine.subscribe();

    let writer = tokio::spawn(async move {
        while let Some(o) = conn.next().await {
            let size = o.size();
            let written = match o.msg {
                Message::Text(t) => write_frame(&mut wr, KIND_TEXT, t.as_bytes()).await,
                Message::Binary(b) => write_frame(&mut wr, KIND_BINARY, &b).await,
                Message::Close(_) => break,
                _ => Ok(()),
            };
            if let Err(e) = written {
                return e.to_string();
            }
            conn.written(size);
        }
        "the connection closed".to_owned()
    });
    // Each change to the copy of the server's records and each engine event,
    // once subscribed. A subscriber left behind sees a gap in the revisions
    // and asks again.
    let notes = engine.registry().subscribe();
    let notifier = tokio::spawn(forward_notes(
        notes,
        events,
        fwd.clone(),
        Arc::clone(&subscribed),
    ));

    // What vornd asks of the server, from the subscription on.
    let mut asks: Option<tokio::task::JoinHandle<()>> = None;

    let mut frames = Frames::default();
    let mut buf = vec![0u8; 64 << 10];
    let why = 'conn: loop {
        let n = match rd.read(&mut buf).await {
            Ok(0) => break "the app closed its channel".to_owned(),
            Ok(n) => n,
            Err(e) => break e.to_string(),
        };
        frames.push(&buf[..n]);
        loop {
            match frames.take() {
                Ok(Some((KIND_TEXT, payload))) => {
                    let Ok(text) = std::str::from_utf8(&payload) else {
                        break 'conn "a frame that is not UTF-8".to_owned();
                    };
                    let app = App {
                        engine: &engine,
                        conn: id,
                        fwd: &fwd,
                        subscribed: &subscribed,
                        link: &link,
                        asks: &mut asks,
                    };
                    call(app, text);
                }
                Ok(Some((kind, _))) => debug!(kind, "a frame kind the app does not send"),
                Ok(None) => break,
                Err(e) => break 'conn e,
            }
        }
    };
    notifier.abort();
    if let Some(asks) = asks {
        asks.abort();
    }
    writer.abort();
    engine.registry().left(id);
    why
}

/// Forwards registry notes and engine events from one task, so they reach the
/// app in the order they happened: a close's remove note is published before
/// its program is hung up, so it goes ahead of that program's exit, and the
/// app never takes the exit of a live record for a second one of a closed
/// record. `biased` keeps that order when both are ready at once.
async fn forward_notes(
    mut notes: broadcast::Receiver<Value>,
    mut events: broadcast::Receiver<Event>,
    fwd: Forwarder,
    subscribed: Arc<AtomicBool>,
) {
    let (mut notes_open, mut events_open) = (true, true);
    while notes_open || events_open {
        tokio::select! {
            biased;
            changed = notes.recv(), if notes_open => match changed {
                Ok(params) => {
                    if subscribed.load(Ordering::Acquire) {
                        fwd.send_now(&note("vornd:session", params));
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => notes_open = false,
            },
            ev = events.recv(), if events_open => match ev {
                Ok(ev) => {
                    if subscribed.load(Ordering::Acquire) {
                        if let Some(v) = event_note(&ev) {
                            fwd.send_now(&v);
                        }
                    }
                }
                // Fell behind: the states are sent again whole.
                Err(RecvError::Lagged(_)) => {
                    if subscribed.load(Ordering::Acquire) {
                        fwd.send_now(&note("vornd:connected", json!({})));
                    }
                }
                Err(RecvError::Closed) => events_open = false,
            },
        }
    }
}

/// Sends a subscribed connection what vornd asks of the server, until the
/// task is aborted, which ends its subscription.
fn forward_asks(link: &Arc<AppLink>, fwd: Forwarder) -> tokio::task::JoinHandle<()> {
    let (mut rx, listening) = link.listen();
    tokio::spawn(async move {
        let _listening = listening;
        loop {
            match rx.recv().await {
                Ok(note) => fwd.send_now(&note),
                Err(RecvError::Lagged(n)) => warn!(n, "the server fell behind vornd's asks"),
                Err(RecvError::Closed) => return,
            }
        }
    })
}

/// Writes one frame.
async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    kind: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len =
        u32::try_from(payload.len() + 1).map_err(|_| std::io::Error::other("frame too large"))?;
    let mut head = [0u8; 5];
    head[..4].copy_from_slice(&len.to_le_bytes());
    head[4] = kind;
    w.write_all(&head).await?;
    w.write_all(payload).await
}

/// Splits what the app sends into frames.
#[derive(Debug, Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, as its kind and payload; `None` while one is
    /// still arriving.
    pub fn take(&mut self) -> Result<Option<(u8, Vec<u8>)>, String> {
        let Some(head) = self.buf.get(..4) else {
            return Ok(None);
        };
        let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(format!("a frame of {len} bytes"));
        }
        if self.buf.len() < 4 + len {
            return Ok(None);
        }
        let kind = self.buf[4];
        let payload = self.buf[5..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Ok(Some((kind, payload)))
    }
}

/// One connection from the app, as a call sees it.
struct App<'a> {
    engine: &'a Arc<Engine>,
    conn: u64,
    fwd: &'a Forwarder,
    subscribed: &'a AtomicBool,
    link: &'a Arc<AppLink>,
    asks: &'a mut Option<tokio::task::JoinHandle<()>>,
}

/// Answers one JSON frame from the app.
fn call(app: App<'_>, text: &str) {
    let App {
        engine,
        conn,
        fwd,
        subscribed,
        link,
        asks,
    } = app;
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let rpc = frame.get("id").cloned();
    let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
    let params = frame.get("params").cloned().unwrap_or(Value::Null);
    let session = params.get("id").and_then(Value::as_str);
    let done = match method {
        "vornd:hello" => Ok(json!({
            "protocol": APP_PROTOCOL,
            "build": env!("CARGO_PKG_VERSION"),
            "native": engine.registry().wanted(),
            "statuses": engine.registry().decides(),
            "terminals": link.creates_terminals() && engine.registry().decides(),
            "headless": link.creates_headless() && engine.registry().decides(),
            "scripts": link.scripts().map(|s| s.mode().name()),
            "restores": link.restores() && engine.registry().owns(),
        })),
        "vornd:subscribe" => {
            if let Some(at) = params.get("bootTime").and_then(Value::as_i64) {
                engine.registry().set_boot_time(at);
            }
            let state = state(engine);
            subscribed.store(true, Ordering::Release);
            if asks.is_none() {
                *asks = Some(forward_asks(link, fwd.clone()));
            }
            Ok(state)
        }
        "vornd:record" => match engine.registry().feed(conn, &params) {
            Ok(()) => Ok(Value::Null),
            Err(e) => {
                // A note is fire-and-forget: the shadow comparison is what
                // shows the copy went wrong.
                warn!(%e, "a session record vornd could not take");
                Err(e.to_string())
            }
        },
        "vornd:registry" => Ok(engine.registry().snapshot()),
        // The server's records of its last run, taken once by a vornd with nothing of its own.
        "vornd:carry" => match params
            .get("terminals")
            .map(Vec::<TerminalSession>::deserialize)
        {
            Some(Ok(terminals)) => {
                let carried = engine
                    .registry()
                    .carry_once(terminals, crate::registry::now_ms());
                // What the holder runs from that run is live again, not offered.
                for h in engine.journal().held() {
                    if h.kind != crate::journal::Kind::Pty {
                        continue;
                    }
                    let epoch = engine
                        .head_stamp(&h.session)
                        .and_then(|s| u32::try_from(s.epoch).ok())
                        .unwrap_or(0);
                    engine.registry().adopt(&crate::registry::Held {
                        id: h.session.clone(),
                        kind: crate::registry::Kind::Terminal,
                        pid: h.pid,
                        epoch,
                    });
                }
                for id in engine.registry().restored_ids() {
                    engine.streams().expect(&id);
                }
                Ok(json!({ "carried": carried }))
            }
            Some(Err(e)) => Err(format!("vornd:carry: {e}")),
            None => Err("vornd:carry needs terminals".to_owned()),
        },
        "vornd:hookStatus" => HookStatus::try_from(&params)
            .and_then(|call| {
                let head = engine.head_stamp(&call.id);
                let now = tokio::time::Instant::now();
                engine.registry().hook_status(&call, head, now)
            })
            .map(|()| Value::Null)
            .map_err(|e| told_wrong(&e)),
        "vornd:input" => match session {
            Some(s) => engine
                .registry()
                .input(s, engine.head_stamp(s))
                .map(|()| Value::Null)
                .map_err(|e| told_wrong(&e)),
            None => Err("vornd:input needs an id".to_owned()),
        },
        "vornd:patch" => Patch::try_from(&params)
            .and_then(|call| engine.registry().patch(&call))
            .map(|()| Value::Null)
            .map_err(|e| told_wrong(&e)),
        "vornd:reach" => match params.get("host").and_then(Value::as_str) {
            Some(host) => {
                link.set_server_host(host.to_owned());
                Ok(Value::Null)
            }
            None => Err("vornd:reach needs a host".to_owned()),
        },
        "vornd:kill" => match (session, signal_of(&params)) {
            (Some(s), Some(sig)) => engine.signal(s, sig).map(|()| Value::Null),
            _ => Err("vornd:kill needs an id and a signal: hup, term, kill or int".to_owned()),
        },
        "vornd:closeStdin" => match session {
            Some(s) => engine.close_stdin(s).map(|()| Value::Null),
            None => Err("vornd:closeStdin needs an id".to_owned()),
        },
        "vornd:draining" => {
            let flag = |k| params.get(k).and_then(Value::as_bool) == Some(true);
            link.set_closing(if flag("draining") {
                Closing::Draining
            } else if flag("handingOver") {
                Closing::HandingOver
            } else {
                Closing::Open
            });
            Ok(Value::Null)
        }
        "vornd:script" | "vornd:scriptCancel" | "vornd:scriptPlan" => {
            return crate::native::script::call(engine, link.scripts(), fwd, rpc, method, params);
        }
        "vornd:claim" | "vornd:unclaim" | "vornd:preparing" | "vornd:prepared" => {
            claim(link, method, &params)
        }
        "vornd:trigger" => match link.work() {
            Some(work) => {
                let (work, fwd) = (Arc::clone(work), fwd.clone());
                tokio::spawn(async move {
                    let received = work.trigger(&params).await;
                    if let Some(rpc) = rpc {
                        match received {
                            Ok(first) => fwd.send_now(&answer(&rpc, json!({ "received": first }))),
                            Err(e) => fwd.send_now(&refuse(&rpc, &e)),
                        }
                    }
                });
                return;
            }
            None => Err("vornd does not run workflows".to_owned()),
        },
        "vornd:signedIn" => match (
            link.work(),
            params.get("connectionId").and_then(Value::as_str),
        ) {
            (Some(work), Some(connection)) => {
                let (work, connection) = (Arc::clone(work), connection.to_owned());
                tokio::spawn(async move { work.signed_in(&connection).await });
                Ok(Value::Null)
            }
            (None, _) => Err("vornd does not run workflows".to_owned()),
            (_, None) => Err("vornd:signedIn needs a connectionId".to_owned()),
        },
        "vornd:work" => match link.work() {
            Some(work) => {
                let (work, fwd) = (Arc::clone(work), fwd.clone());
                let method = params
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let inner = params.get("params").cloned().unwrap_or(Value::Null);
                tokio::spawn(async move {
                    let answered = work.answer(&method, &inner).await;
                    if let Some(rpc) = rpc {
                        match answered {
                            crate::native::Answer::Result(v) => {
                                fwd.send_now(&answer(&rpc, json!({ "result": v })))
                            }
                            crate::native::Answer::Void => fwd.send_now(&answer(&rpc, json!({}))),
                            crate::native::Answer::Error(e) => fwd.send_now(&refuse(&rpc, &e)),
                            crate::native::Answer::Forward => fwd.send_now(&refuse(
                                &rpc,
                                &format!("vornd does not answer {method}"),
                            )),
                        }
                    }
                });
                return;
            }
            None => Err("vornd does not run workflows".to_owned()),
        },
        "vornd:configChanged" => {
            if let Some(work) = link.work() {
                work.workflows_changed_elsewhere();
            }
            Ok(Value::Null)
        }
        _ => {
            if method == "vornd:spawn" {
                if let Some(name) = params.get("name").and_then(Value::as_str) {
                    link.record_spawn(name, &params);
                }
            }
            if crate::terminal::handle_for_app(engine, conn, fwd, text) {
                return;
            }
            Err(match session {
                Some(s) if method.starts_with("terminal:") => format!("no session {s}"),
                _ => format!("vornd does not answer {method}"),
            })
        }
    };
    if let Some(rpc) = rpc {
        match done {
            Ok(v) => fwd.send_now(&answer(&rpc, v)),
            Err(e) => fwd.send_now(&refuse(&rpc, &e)),
        }
    }
}

/// The server's claims on the conversations its own starts take.
fn claim(link: &AppLink, method: &str, params: &Value) -> Result<Value, String> {
    let text = |k| params.get(k).and_then(Value::as_str);
    let claims = link.claims();
    let now = std::time::Instant::now();
    match (method, text("sessionId"), text("transcriptId")) {
        ("vornd:claim", Some(session), Some(transcript)) => {
            Ok(json!({ "holder": claims.claim(transcript, session, now) }))
        }
        ("vornd:unclaim", Some(session), Some(transcript)) => {
            claims.release(transcript, session);
            Ok(Value::Null)
        }
        ("vornd:unclaim", Some(session), None) => {
            claims.release_for(session);
            Ok(Value::Null)
        }
        ("vornd:preparing", Some(session), _) => {
            claims.preparing(session);
            Ok(Value::Null)
        }
        ("vornd:prepared", Some(session), _) => {
            claims.prepared(session, now);
            Ok(Value::Null)
        }
        _ => Err(format!(
            "{method} needs a sessionId, and a claim a transcriptId"
        )),
    }
}

/// A note the registry could not take, logged: the server sends them
/// without waiting for an answer.
fn told_wrong(e: &crate::registry::RegistryError) -> String {
    warn!(%e, "a session change vornd could not take");
    e.to_string()
}

fn signal_of(params: &Value) -> Option<Sig> {
    Some(match params.get("signal")?.as_str()? {
        "hup" => Sig::Hup,
        "term" => Sig::Term,
        "kill" => Sig::Kill,
        "int" => Sig::Int,
        _ => return None,
    })
}

/// Everything a subscription starts from.
fn state(engine: &Engine) -> Value {
    let journal = engine.journal();
    let notices: Vec<Value> = journal
        .notices()
        .iter()
        .filter_map(|(id, e)| effect_json(id, e))
        .collect();
    let mut state = json!({
        "connected": engine.connected(),
        "sessions": journal.held().iter().map(held_json).collect::<Vec<_>>(),
        "ended": journal.ended().iter().map(held_json).collect::<Vec<_>>(),
        "notices": notices,
    });
    // Only to a server asked for its records: one that was not sees the
    // answer it always had.
    if engine.registry().wanted() {
        state["registry"] = engine.registry().snapshot();
    }
    state
}

fn held_json(h: &Held) -> Value {
    fn stamped<T>(s: &Option<Stamped<T>>, f: impl Fn(&T) -> Effect) -> Value {
        s.as_ref()
            .and_then(|s| effect_json(&s.id, &f(&s.value)))
            .unwrap_or(Value::Null)
    }
    json!({
        "id": h.session,
        "kind": h.kind.as_str(),
        "pid": h.pid,
        "status": stamped(&h.status, |s| Effect::Status(*s)),
        "cwd": stamped(&h.cwd, |c| Effect::Cwd(c.clone())),
        "exit": stamped(&h.exit, |&(code, signal)| Effect::Exit { code, signal }),
    })
}

/// The string an effect is known by: the same in every replay of the
/// records that caused it.
pub fn effect_key(id: &EffectId) -> String {
    format!("{}/{}/{}/{}", id.session, id.epoch, id.rseq, id.index)
}

/// An effect as the app is told it. The bell and clipboard writes are not
/// the app's.
pub fn effect_json(id: &EffectId, effect: &Effect) -> Option<Value> {
    let mut v = json!({
        "effectId": effect_key(id),
        "id": id.session,
        "epoch": id.epoch,
        "rseq": id.rseq,
        "index": id.index,
    });
    let fields = match effect {
        Effect::Status(s) => json!({ "kind": "status", "status": s }),
        Effect::Cwd(c) => json!({ "kind": "cwd", "cwd": c }),
        Effect::Exit { code, signal } => json!({
            "kind": "exit",
            "code": code,
            "signal": signal,
            "exitCode": exit_code(*code, *signal),
        }),
        Effect::Notify { title, body } => json!({ "kind": "notify", "title": title, "body": body }),
        Effect::Bell | Effect::Clipboard { .. } => return None,
    };
    if let (Value::Object(v), Value::Object(f)) = (&mut v, fields) {
        v.extend(f);
    }
    Some(v)
}

fn note(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// What a subscribed app is sent for an engine event.
fn event_note(ev: &Event) -> Option<Value> {
    match ev {
        Event::Effect(id, effect) => effect_json(id, effect).map(|v| note("vornd:effect", v)),
        Event::Activity(session) => Some(note("vornd:activity", json!({ "id": session }))),
        Event::Connected => Some(note("vornd:connected", json!({}))),
        Event::Ready { .. } | Event::Closed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx() -> EffectId {
        EffectId {
            session: "pane-1".into(),
            epoch: 3,
            rseq: 41,
            index: 2,
        }
    }

    #[test]
    fn splits_frames_however_they_arrive() {
        let mut f = Frames::default();
        let mut wire = Vec::new();
        for (kind, payload) in [(KIND_TEXT, &b"{}"[..]), (KIND_BINARY, &b"\x02abc"[..])] {
            wire.extend_from_slice(&u32::try_from(payload.len() + 1).unwrap().to_le_bytes());
            wire.push(kind);
            wire.extend_from_slice(payload);
        }
        let mut got = Vec::new();
        for b in wire {
            f.push(&[b]);
            while let Some(frame) = f.take().unwrap() {
                got.push(frame);
            }
        }
        assert_eq!(
            got,
            vec![
                (KIND_TEXT, b"{}".to_vec()),
                (KIND_BINARY, b"\x02abc".to_vec())
            ]
        );
    }

    #[test]
    fn refuses_a_frame_too_large_or_empty() {
        let mut f = Frames::default();
        f.push(&0u32.to_le_bytes());
        assert!(f.take().is_err());
        let mut f = Frames::default();
        f.push(&u32::try_from(MAX_FRAME + 1).unwrap().to_le_bytes());
        assert!(f.take().is_err());
    }

    #[test]
    fn an_effect_is_named_the_same_in_every_replay() {
        let exit = Effect::Exit {
            code: None,
            signal: Some(9),
        };
        let v = effect_json(&fx(), &exit).unwrap();
        assert_eq!(v["effectId"], "pane-1/3/41/2");
        assert_eq!(v["kind"], "exit");
        assert_eq!(v["exitCode"], 137);
        assert_eq!(effect_json(&fx(), &exit), Some(v));
        assert!(effect_json(&fx(), &Effect::Bell).is_none());
        let n = effect_json(
            &fx(),
            &Effect::Notify {
                title: "Build".into(),
                body: "done".into(),
            },
        )
        .unwrap();
        assert_eq!(
            (n["kind"].as_str(), n["title"].as_str()),
            (Some("notify"), Some("Build"))
        );
    }

    /// One end of an app channel to `engine`, speaking JSON frames.
    struct App {
        io: tokio::io::DuplexStream,
        frames: Frames,
        next: u64,
    }

    impl App {
        fn open(engine: &Arc<Engine>) -> App {
            App::open_with(engine, Arc::new(crate::applink::AppLink::default()))
        }

        fn open_with(engine: &Arc<Engine>, link: Arc<AppLink>) -> App {
            let (ours, theirs) = tokio::io::duplex(1 << 20);
            tokio::spawn(serve_conn(theirs, Arc::clone(engine), link));
            App {
                io: ours,
                frames: Frames::default(),
                next: 0,
            }
        }

        async fn send(&mut self, frame: Value) {
            write_frame(&mut self.io, KIND_TEXT, frame.to_string().as_bytes())
                .await
                .unwrap();
        }

        async fn notify(&mut self, method: &str, params: Value) {
            self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
                .await;
        }

        async fn next_frame(&mut self) -> Value {
            let mut buf = [0u8; 4096];
            loop {
                if let Some((_, payload)) = self.frames.take().unwrap() {
                    return serde_json::from_slice(&payload).unwrap();
                }
                let n =
                    tokio::time::timeout(std::time::Duration::from_secs(5), self.io.read(&mut buf))
                        .await
                        .expect("a frame within 5s")
                        .unwrap();
                assert!(n > 0, "the channel closed");
                self.frames.push(&buf[..n]);
            }
        }

        /// The answer to a call, and the notes that came before it.
        async fn call(&mut self, method: &str, params: Value) -> (Value, Vec<Value>) {
            self.next += 1;
            let rpc = self.next;
            self.send(json!({ "jsonrpc": "2.0", "id": rpc, "method": method, "params": params }))
                .await;
            let mut notes = Vec::new();
            loop {
                let frame = self.next_frame().await;
                if frame["id"] == rpc {
                    return (frame["result"].clone(), notes);
                }
                notes.push(frame);
            }
        }
    }

    fn shell(id: &str, name: &str) -> Value {
        json!({
            "id": id, "agentType": "shell", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 1, "pid": 3, "displayName": name,
        })
    }

    #[tokio::test]
    async fn a_close_is_told_before_the_exit_it_causes() {
        let engine = Engine::new(vorn_engine::Config::default());
        let mut conn = engine.streams().connect();
        let (notes_tx, notes) = broadcast::channel(8);
        let (events_tx, events) = broadcast::channel(8);
        // Both waiting at once, the exit queued first, as a busy app sees them.
        let exit = Effect::Exit {
            code: Some(0),
            signal: None,
        };
        events_tx.send(Event::Effect(fx(), exit)).unwrap();
        notes_tx
            .send(json!({ "op": "remove", "kind": "terminal", "id": "pane-1" }))
            .unwrap();
        drop((notes_tx, events_tx));
        forward_notes(
            notes,
            events,
            conn.forwarder(),
            Arc::new(AtomicBool::new(true)),
        )
        .await;
        let mut told = Vec::new();
        while told.len() < 2 {
            let Some(o) = conn.next().await else { break };
            if let Message::Text(t) = o.msg {
                let v: Value = serde_json::from_str(&t).unwrap();
                told.push(v["method"].as_str().unwrap_or_default().to_owned());
            }
        }
        assert_eq!(told, ["vornd:session", "vornd:effect"]);
    }

    #[tokio::test]
    async fn the_server_feeds_the_copy_of_its_records_only_when_asked() {
        let engine = Engine::new(vorn_engine::Config::default());
        let mut app = App::open(&engine);
        let (hello, _) = app.call("vornd:hello", Value::Null).await;
        assert_eq!(hello["native"], false);
        assert_eq!(hello["statuses"], false);
        assert_eq!(
            (&hello["terminals"], &hello["headless"]),
            (&json!(false), &json!(false))
        );
        let (state, _) = app.call("vornd:subscribe", Value::Null).await;
        assert!(state.get("registry").is_none());

        engine.registry().want();
        let mut app = App::open(&engine);
        assert_eq!(app.call("vornd:hello", Value::Null).await.0["native"], true);
        app.notify(
            "vornd:record",
            json!({ "op": "snapshot", "terminals": [shell("a", "Shell 1")], "headless": [] }),
        )
        .await;
        let (state, _) = app.call("vornd:subscribe", Value::Null).await;
        let gen = state["registry"]["gen"].clone();
        assert_eq!(state["registry"]["rev"], 1);
        assert_eq!(state["registry"]["terminals"][0]["displayName"], "Shell 1");
        assert_eq!(engine.registry().read(|r| r.terminals().len()), Some(1));

        // Each change is told, in order. They go out beside the answers, not
        // ahead of them: a subscriber orders by revision, and drops a change
        // made before its subscription that is told again after it.
        app.notify(
            "vornd:record",
            json!({ "op": "upsert", "kind": "terminal", "record": shell("a", "renamed") }),
        )
        .await;
        app.notify("vornd:record", json!({ "op": "order", "order": ["a"] }))
            .await;
        let (snapshot, mut notes) = app.call("vornd:registry", Value::Null).await;
        assert_eq!(snapshot["rev"], 3);
        assert_eq!(snapshot["terminals"][0]["rev"], 2);
        let told = |notes: &[Value]| -> Vec<(Value, Value)> {
            notes
                .iter()
                .filter(|n| n["method"] == "vornd:session" && n["params"]["rev"].as_u64() > Some(1))
                .map(|n| (n["params"]["rev"].clone(), n["params"]["op"].clone()))
                .collect()
        };
        while told(&notes).len() < 2 {
            notes.push(app.next_frame().await);
        }
        assert_eq!(
            told(&notes),
            [(json!(2), json!("upsert")), (json!(3), json!("order"))]
        );
        assert!(notes.iter().all(|n| n["params"]["gen"] == gen));

        // A note it cannot read is refused when asked, and changes nothing.
        let refused = {
            app.next += 1;
            let rpc = app.next;
            app.send(json!({ "jsonrpc": "2.0", "id": rpc, "method": "vornd:record", "params": { "op": "x" } }))
                .await;
            loop {
                let f = app.next_frame().await;
                if f["id"] == rpc {
                    break f;
                }
            }
        };
        assert!(refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown op"));
        assert_eq!(engine.registry().snapshot()["rev"], 3);

        // The server that fed it went: nothing is answered from the copy.
        drop(app);
        for _ in 0..100 {
            if engine.registry().read(|_| ()).is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the copy still answers after its server left");
    }

    #[tokio::test]
    async fn a_server_told_vornd_decides_the_statuses_sends_it_what_only_it_sees() {
        let engine = Engine::new(vorn_engine::Config::default());
        engine.registry().want();
        engine.decide_statuses();
        let link = Arc::new(AppLink::default());
        link.set_creates_headless();
        let mut app = App::open_with(&engine, link);
        let (hello, _) = app.call("vornd:hello", Value::Null).await;
        assert_eq!(hello["statuses"], true);
        // Each as vornd was started: terminals and headless agents apart.
        assert_eq!(
            (&hello["terminals"], &hello["headless"]),
            (&json!(false), &json!(true))
        );
        let mut agent = shell("a", "Claude");
        agent["agentType"] = json!("claude");
        app.notify(
            "vornd:record",
            json!({ "op": "snapshot", "terminals": [agent], "headless": [] }),
        )
        .await;
        app.call("vornd:subscribe", Value::Null).await;
        app.notify(
            "vornd:patch",
            json!({ "id": "a", "fields": { "hookSessionId": "conv" } }),
        )
        .await;
        app.notify(
            "vornd:hookStatus",
            json!({ "id": "a", "status": "waiting", "promote": true }),
        )
        .await;
        app.notify("vornd:input", json!({ "id": "a" })).await;
        // The answer comes after every note the calls before it made.
        let (snapshot, mut notes) = app.call("vornd:registry", Value::Null).await;
        let record = &snapshot["terminals"][0];
        assert_eq!(
            (
                record["hookSessionId"].as_str(),
                record["status"].as_str(),
                record["statusSource"].as_str()
            ),
            (Some("conv"), Some("waiting"), Some("hooks"))
        );
        while notes
            .iter()
            .filter(|n| n["method"] == "vornd:session")
            .count()
            < 4
        {
            notes.push(app.next_frame().await);
        }
        let told: Vec<_> = notes
            .iter()
            .filter(|n| n["method"] == "vornd:session" && n["params"]["op"] == "upsert")
            .map(|n| n["params"]["record"]["status"].clone())
            .collect();
        // Linked, waiting, then promoted; input to a terminal on hooks wakes nothing.
        assert_eq!(told, [json!("running"), json!("waiting"), json!("waiting")]);

        // A call it cannot take is refused when asked.
        app.next += 1;
        let rpc = app.next;
        app.send(json!({ "jsonrpc": "2.0", "id": rpc, "method": "vornd:hookStatus", "params": { "id": "nope", "status": "idle" } }))
            .await;
        let refused = loop {
            let f = app.next_frame().await;
            if f["id"] == rpc {
                break f;
            }
        };
        assert_eq!(
            refused["error"]["message"],
            "vornd:hookStatus: no terminal nope"
        );
    }

    #[test]
    fn the_announcement_names_the_endpoint_and_is_withdrawn_only_by_its_vornd() {
        // A fresh home: nothing has made run/ yet.
        let home = tempfile::tempdir().unwrap();
        announce(home.path(), "/x/app.sock").unwrap();
        let text = std::fs::read_to_string(home.path().join("run").join(ANNOUNCEMENT)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["endpoint"], "/x/app.sock");
        withdraw(home.path());
        assert!(!home.path().join("run").join(ANNOUNCEMENT).exists());
        // Another vornd's announcement stays.
        std::fs::write(
            home.path().join("run").join(ANNOUNCEMENT),
            r#"{"pid":1,"endpoint":"/y"}"#,
        )
        .unwrap();
        withdraw(home.path());
        assert!(home.path().join("run").join(ANNOUNCEMENT).exists());
    }

    /// The sweep can tell a dead vornd's endpoints by the pid in their names.
    #[cfg(unix)]
    #[test]
    fn the_endpoints_are_named_for_the_sweep() {
        let home = Path::new("/h");
        let pid = std::process::id();
        for endpoint in [endpoint(home), crate::grid::endpoint(home)] {
            let name = Path::new(&endpoint).file_name().unwrap().to_str().unwrap();
            assert!(
                vorn_sessiond::rundir::PID_NAMED
                    .iter()
                    .any(|p| name == format!("{p}{pid}.sock")),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn a_vornd_that_owns_the_records_says_so_and_takes_the_servers_once() {
        let engine = Engine::new(vorn_engine::Config::default());
        engine.registry().want();
        engine.decide_statuses();
        let link = Arc::new(AppLink::default());
        link.set_creates_terminals();
        link.set_creates_headless();
        let mut app = App::open_with(&engine, Arc::clone(&link));
        assert_eq!(
            app.call("vornd:hello", Value::Null).await.0["restores"],
            false
        );
        engine.registry().own_records();
        assert_eq!(
            app.call("vornd:hello", Value::Null).await.0["restores"],
            true
        );
        // When the machine came up decides which offers a reboot ended.
        let now = crate::registry::now_ms();
        let (state, _) = app
            .call("vornd:subscribe", json!({ "bootTime": now }))
            .await;
        assert!(state["registry"].get("restored").is_none());
        let mut old = shell("a", "Shell 1");
        old["savedAt"] = json!(now - 1_000);
        let mut held = shell("held", "Shell 2");
        held["savedAt"] = json!(now - 1_000);
        engine
            .journal()
            .opened("held", crate::journal::Kind::Pty, 7);
        let (carried, notes) = app
            .call("vornd:carry", json!({ "terminals": [old.clone(), held] }))
            .await;
        assert_eq!(carried, json!({ "carried": 2 }));
        drop(notes);
        let offered = engine.registry().restored().unwrap();
        assert_eq!(offered[0]["session"]["id"], "a");
        assert_eq!(offered[0]["rebooted"], true);
        assert!(engine.streams().expects("a"));
        assert!(!engine.streams().expects("held"));
        // The one the holder runs is live again, not offered.
        let snapshot = engine.registry().snapshot();
        assert_eq!(snapshot["terminals"][0]["id"], "held");
        assert_eq!(snapshot["terminals"][0]["pid"], 7);
        // Taken once: a later server's records are not.
        let (again, _) = app
            .call(
                "vornd:carry",
                json!({ "terminals": [shell("b", "Shell 2")] }),
            )
            .await;
        assert_eq!(again, json!({ "carried": 0 }));
        assert_eq!(engine.registry().restored().unwrap().len(), 1);
        let (state, _) = app.call("vornd:subscribe", Value::Null).await;
        assert_eq!(state["registry"]["restored"].as_array().unwrap().len(), 1);
    }
}
