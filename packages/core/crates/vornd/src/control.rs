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
//! - `vornd:hello` answers `{protocol, build}`.
//! - `vornd:subscribe` answers `{connected, sessions, ended, notices}`: every
//!   session held with its latest states, the sessions that ended lately and
//!   the notifications kept. From then on the connection is sent each
//!   effect as `vornd:effect`, `vornd:activity {id}` while a session prints,
//!   and `vornd:connected` when the engine (re)connects to sessiond, after
//!   which the server subscribes again. Anything may arrive twice: states
//!   carry the record they reflect, and the server drops notifications and
//!   exits it has already acted on by their `effectId`.
//! - `vornd:kill {id, signal}` signals the session's program (`hup`,
//!   `term`, `kill` or `int`).
//! - `vornd:closeStdin {id}` ends a piped session's input.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};
use vorn_engine::{Effect, EffectId};
use vorn_sessiond::os;
use vorn_sessiond_wire::Sig;

use crate::engine::{Engine, Event};
use crate::journal::{Held, Stamped};
use crate::streams::{answer, exit_code, refuse, Forwarder};

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
pub async fn serve(mut listener: os::Listener, engine: Arc<Engine>) {
    loop {
        match listener.accept().await {
            Ok(stream) => {
                let engine = Arc::clone(&engine);
                tokio::spawn(async move {
                    let why = serve_conn(stream, engine).await;
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
pub async fn serve_conn<S>(stream: S, engine: Arc<Engine>) -> String
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
    let mut events = engine.subscribe();

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
    let notifier = {
        let (fwd, subscribed) = (fwd.clone(), Arc::clone(&subscribed));
        tokio::spawn(async move {
            loop {
                match events.recv().await {
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
                    Err(RecvError::Closed) => return,
                }
            }
        })
    };

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
                    call(&engine, id, &fwd, &subscribed, text);
                }
                Ok(Some((kind, _))) => debug!(kind, "a frame kind the app does not send"),
                Ok(None) => break,
                Err(e) => break 'conn e,
            }
        }
    };
    notifier.abort();
    writer.abort();
    why
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

/// Answers one JSON frame from the app.
fn call(engine: &Arc<Engine>, conn: u64, fwd: &Forwarder, subscribed: &AtomicBool, text: &str) {
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
        })),
        "vornd:subscribe" => {
            let state = state(engine);
            subscribed.store(true, Ordering::Release);
            Ok(state)
        }
        "vornd:kill" => match (session, signal_of(&params)) {
            (Some(s), Some(sig)) => engine.signal(s, sig).map(|()| Value::Null),
            _ => Err("vornd:kill needs an id and a signal: hup, term, kill or int".to_owned()),
        },
        "vornd:closeStdin" => match session {
            Some(s) => engine.close_stdin(s).map(|()| Value::Null),
            None => Err("vornd:closeStdin needs an id".to_owned()),
        },
        _ => {
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
    json!({
        "connected": engine.connected(),
        "sessions": journal.held().iter().map(held_json).collect::<Vec<_>>(),
        "ended": journal.ended().iter().map(held_json).collect::<Vec<_>>(),
        "notices": notices,
    })
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
}
