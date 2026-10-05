//! Grid mode's endpoint (Terminal State Protocol §7): where the native app
//! attaches to sessions as a mirror of their screens rather than as bytes.
//!
//! The endpoint is a local socket only this user can open, the same kind
//! sessiond serves on: a Unix socket in the 0700 `run/` directory with the
//! peer's UID checked, or a named pipe whose DACL grants only this user. It
//! exists only while the session engine does, so with the Native daemon
//! switch off nothing listens.
//!
//! A connection says Hello and gets Welcome (or Error 426 for a protocol
//! major this vornd cannot serve), then attaches to sessions. vornd names
//! each attachment with a `sid`, routes the client's requests to the
//! session's actor ([`crate::engine::Engine::grid_input`]), and writes what
//! the actor answers. A client that stops reading is disconnected once its
//! queue fills ([`crate::engine::GRID_QUEUE`]), and resumes with a snapshot.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};
use vorn_engine::{GridIn, Peer};
use vorn_sessiond::os;
use vorn_term_proto::msg::{
    caps, major_supported, AttachMode, ClientMsg, FrameReader, Hello, MsgError, ServerMsg, Welcome,
    PROTO_MAJOR, PROTO_MINOR,
};

use crate::engine::Engine;

/// Error codes a grid connection can get.
pub mod code {
    /// A protocol major this vornd cannot serve.
    pub const UPGRADE: u16 = 426;
    /// No such session.
    pub const NOT_FOUND: u16 = 404;
    /// Something this endpoint does not serve (bytes mode is the
    /// WebSocket's).
    pub const NOT_SERVED: u16 = 501;
    /// The first message was not a Hello, or a frame did not decode.
    pub const BAD_REQUEST: u16 = 400;
}

/// The endpoint for this vornd under `home`: one per process, so a vornd
/// handing over to a newer one never takes its endpoint.
pub fn endpoint(home: &Path) -> String {
    let pid = std::process::id();
    #[cfg(unix)]
    {
        home.join("run")
            .join(format!("vornd-grid-{pid}.sock"))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        let _ = home;
        format!(
            r"\\.\pipe\vorn-grid-{}-{pid}",
            os::user_sid().unwrap_or_else(|_| "user".into())
        )
    }
}

/// The answer to a client's Hello: Welcome with the capabilities both
/// sides have, or the Error that ends the connection (TP §13).
pub fn welcome(hello: &Hello, build: &str, instance: u64) -> Result<Welcome, ServerMsg> {
    if !major_supported(hello.proto_major) {
        return Err(ServerMsg::Error {
            code: code::UPGRADE,
            message: format!(
                "grid protocol {} is not served here; this vornd speaks {PROTO_MAJOR} and {}",
                hello.proto_major,
                PROTO_MAJOR.saturating_sub(1)
            ),
        });
    }
    Ok(Welcome {
        // Served in the client's major, which is this one or the last.
        proto_major: hello.proto_major,
        proto_minor: PROTO_MINOR,
        caps: hello.caps & caps::ALL,
        vornd_build: build.to_owned(),
        vornd_instance: instance,
    })
}

/// Accepts grid connections until the listener fails.
pub async fn serve(mut listener: os::Listener, engine: Arc<Engine>, build: String, instance: u64) {
    loop {
        match listener.accept().await {
            Ok(stream) => {
                let engine = Arc::clone(&engine);
                let build = build.clone();
                tokio::spawn(async move {
                    let why = serve_conn(stream, engine, &build, instance).await;
                    debug!(%why, "grid connection ended");
                });
            }
            Err(e) => {
                warn!(%e, "grid endpoint failed");
                listener.close();
                return;
            }
        }
    }
}

/// How long one write to a grid client may take. A client that stops
/// reading is disconnected then, its attachments released, and it resumes
/// with a snapshot when it comes back (TP §11).
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// One grid connection, until it ends; answers why it ended.
pub async fn serve_conn<S>(stream: S, engine: Arc<Engine>, build: &str, instance: u64) -> String
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    serve_conn_with(stream, engine, build, instance, WRITE_TIMEOUT).await
}

/// [`serve_conn`] with its write timeout given.
pub async fn serve_conn_with<S>(
    stream: S,
    engine: Arc<Engine>,
    build: &str,
    instance: u64,
    write_timeout: Duration,
) -> String
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut rd, mut wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new();
    let mut buf = vec![0u8; 64 << 10];
    let mut out = Vec::new();

    // Hello first.
    let hello = loop {
        match next_message(&mut reader) {
            Ok(Some(ClientMsg::Hello(h))) => break h,
            Ok(Some(_)) => {
                let _ = tokio::time::timeout(
                    write_timeout,
                    refuse(&mut wr, code::BAD_REQUEST, "Hello first"),
                )
                .await;
                return "no Hello".into();
            }
            Ok(None) => {}
            Err(e) => {
                let _ = tokio::time::timeout(
                    write_timeout,
                    refuse(&mut wr, code::BAD_REQUEST, &e.to_string()),
                )
                .await;
                return e.to_string();
            }
        }
        match rd.read(&mut buf).await {
            Ok(0) => return "closed before Hello".into(),
            Ok(n) => reader.push(&buf[..n]),
            Err(e) => return e.to_string(),
        }
    };
    match welcome(&hello, build, instance) {
        Ok(w) => {
            info!(build = %hello.build, client = ?hello.client, major = hello.proto_major, "grid client");
            ServerMsg::Welcome(w).encode(&mut out);
            if let Err(e) = write(&mut wr, &out, write_timeout).await {
                return e;
            }
            out.clear();
        }
        Err(refusal) => {
            refusal.encode(&mut out);
            if write(&mut wr, &out, write_timeout).await.is_ok() {
                let _ = tokio::time::timeout(write_timeout, wr.shutdown()).await;
            }
            return "unsupported protocol".into();
        }
    }

    let (conn, mut queue) = engine.grid_open();
    let mut c = Conn {
        engine: &engine,
        conn,
        sids: HashMap::new(),
        next_sid: 0,
    };
    let why = loop {
        // Whatever arrived already, before waiting for more.
        loop {
            match next_message(&mut reader) {
                Ok(Some(m)) => {
                    if let Some(reply) = c.request(m) {
                        reply.encode(&mut out);
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    ServerMsg::Error {
                        code: code::BAD_REQUEST,
                        message: e.to_string(),
                    }
                    .encode(&mut out);
                    let _ = write(&mut wr, &out, write_timeout).await;
                    c.close();
                    return e.to_string();
                }
            }
        }
        if !out.is_empty() {
            // A client that stops reading parks this write: the timeout
            // ends the connection and releases its attachments.
            if let Err(e) = write(&mut wr, &out, write_timeout).await {
                break e;
            }
            out.clear();
        }
        tokio::select! {
            r = rd.read(&mut buf) => match r {
                Ok(0) => break "closed".to_owned(),
                Ok(n) => reader.push(&buf[..n]),
                Err(e) => break e.to_string(),
            },
            m = queue.recv() => match m {
                Some(m) => {
                    m.encode(&mut out);
                    // Everything else waiting goes in the same write.
                    while let Ok(m) = queue.try_recv() {
                        m.encode(&mut out);
                    }
                }
                None => break "fell behind".to_owned(),
            },
        }
    };
    c.close();
    why
}

/// Writes `bytes`, or fails once that has taken `limit`.
async fn write<W: AsyncWrite + Unpin>(
    w: &mut W,
    bytes: &[u8],
    limit: Duration,
) -> Result<(), String> {
    match tokio::time::timeout(limit, w.write_all(bytes)).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err(format!("a write took longer than {limit:?}")),
    }
}

fn next_message(reader: &mut FrameReader) -> Result<Option<ClientMsg>, MsgError> {
    loop {
        let Some((kind, payload)) = reader.next_frame()? else {
            return Ok(None);
        };
        // Optional kinds this build does not know are skipped.
        if let Some(m) = ClientMsg::decode(kind, payload)? {
            return Ok(Some(m));
        }
    }
}

async fn refuse<W: AsyncWrite + Unpin>(w: &mut W, code: u16, message: &str) -> std::io::Result<()> {
    let mut out = Vec::new();
    ServerMsg::Error {
        code,
        message: message.to_owned(),
    }
    .encode(&mut out);
    w.write_all(&out).await?;
    w.shutdown().await
}

/// One connection's attachments.
struct Conn<'a> {
    engine: &'a Engine,
    conn: u64,
    /// The session each attachment is on.
    sids: HashMap<u32, String>,
    next_sid: u32,
}

impl Conn<'_> {
    /// Routes one request; an answer vornd gives itself comes back.
    fn request(&mut self, m: ClientMsg) -> Option<ServerMsg> {
        let conn = self.conn;
        let peer = |sid| Peer { conn, sid };
        let mut attaching = None;
        let routed = match m {
            ClientMsg::Hello(_) => {
                return Some(error(code::BAD_REQUEST, "Hello was already said"));
            }
            ClientMsg::Attach(a) => {
                if a.mode == AttachMode::Bytes {
                    return Some(error(
                        code::NOT_SERVED,
                        "bytes mode is served on the WebSocket",
                    ));
                }
                if !self.engine.has_session(&a.session) {
                    return Some(error(code::NOT_FOUND, &format!("no session {}", a.session)));
                }
                self.next_sid += 1;
                let sid = self.next_sid;
                self.sids.insert(sid, a.session.clone());
                attaching = Some(sid);
                let session = a.session.clone();
                (
                    session,
                    GridIn::Attach {
                        peer: peer(sid),
                        attach: a,
                    },
                )
            }
            ClientMsg::Detach { sid } => {
                let s = self.sids.remove(&sid)?;
                (s, GridIn::Detach { peer: peer(sid) })
            }
            ClientMsg::SetVisible { sid, visible } => (
                self.session(sid)?,
                GridIn::SetVisible {
                    peer: peer(sid),
                    visible,
                },
            ),
            ClientMsg::Ack { sid, rev } => (
                self.session(sid)?,
                GridIn::Ack {
                    peer: peer(sid),
                    rev,
                },
            ),
            ClientMsg::Input {
                sid,
                input_seq,
                event,
            } => (
                self.session(sid)?,
                GridIn::Input {
                    peer: peer(sid),
                    input_seq,
                    event,
                },
            ),
            ClientMsg::FetchHistory {
                sid,
                req,
                sb_epoch,
                from_line,
                count,
            } => (
                self.session(sid)?,
                GridIn::FetchHistory {
                    peer: peer(sid),
                    req,
                    sb_epoch,
                    from_line,
                    count,
                },
            ),
            ClientMsg::SelectAt { sid, req, at, kind } => (
                self.session(sid)?,
                GridIn::SelectAt {
                    peer: peer(sid),
                    req,
                    at,
                    kind,
                },
            ),
            ClientMsg::Copy {
                sid,
                req,
                from,
                to,
                rect,
                format,
            } => (
                self.session(sid)?,
                GridIn::Copy {
                    peer: peer(sid),
                    req,
                    from,
                    to,
                    rect,
                    format,
                },
            ),
            ClientMsg::Search {
                sid,
                req,
                query,
                regex,
                case,
                from_line,
            } => (
                self.session(sid)?,
                GridIn::Search {
                    peer: peer(sid),
                    req,
                    query,
                    regex,
                    case,
                    from_line,
                },
            ),
            // Size policy and default colours are not served yet; a client
            // that sends them carries on.
            ClientMsg::Unhandled(kind) => {
                debug!(kind, "grid request not served yet");
                return None;
            }
        };
        let (session, m) = routed;
        match self.engine.grid_input(&session, m) {
            Ok(()) => None,
            Err(e) => {
                // Fail closed: no attachment is left behind for an attach
                // nobody will answer.
                if let Some(sid) = attaching {
                    self.sids.remove(&sid);
                }
                Some(error(code::NOT_FOUND, &e))
            }
        }
    }

    fn session(&self, sid: u32) -> Option<String> {
        self.sids.get(&sid).cloned()
    }

    fn close(&mut self) {
        let sessions: Vec<String> = self.sids.drain().map(|(_, s)| s).collect();
        self.engine.grid_close(self.conn, sessions);
    }
}

fn error(code: u16, message: &str) -> ServerMsg {
    ServerMsg::Error {
        code,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use vorn_engine::Config;
    use vorn_term_proto::msg::{Attach, ClientKind};

    fn hello(major: u16, caps: u64) -> Hello {
        Hello {
            proto_major: major,
            proto_minor: 9,
            caps,
            client: ClientKind::Native,
            build: "app".into(),
        }
    }

    /// TP-T18's handshake: this major and the one before attach with the
    /// capabilities both sides have; any other gets Error 426.
    #[test]
    fn versions_meet_on_common_capabilities_or_fail_with_426() {
        let w = welcome(&hello(PROTO_MAJOR, caps::ALL | 1 << 40), "vornd", 7).unwrap();
        assert_eq!(w.caps, caps::ALL);
        assert_eq!(w.proto_major, PROTO_MAJOR);
        let older = welcome(&hello(PROTO_MAJOR - 1, caps::GRID), "vornd", 7).unwrap();
        assert_eq!(older.caps, caps::GRID);
        assert_eq!(older.proto_major, PROTO_MAJOR - 1);
        for major in [PROTO_MAJOR + 1, PROTO_MAJOR + 7] {
            match welcome(&hello(major, caps::ALL), "vornd", 7) {
                Err(ServerMsg::Error { code: c, .. }) => assert_eq!(c, code::UPGRADE),
                other => panic!("{other:?}"),
            }
        }
    }

    /// A client that stops reading is let go once a write has taken the
    /// write timeout, instead of parking its connection, and its
    /// attachments, for ever.
    #[tokio::test]
    async fn a_client_that_stops_reading_is_let_go() {
        let (mut app, theirs) = tokio::io::duplex(256);
        let engine = Engine::new(Config::default());
        let serving = tokio::spawn(async move {
            serve_conn_with(theirs, engine, "test", 1, Duration::from_millis(100)).await
        });
        let mut out = Vec::new();
        ClientMsg::Hello(hello(PROTO_MAJOR, caps::ALL)).encode(&mut out);
        // Requests that each get an error back, more than the pipe holds,
        // and never a read.
        for _ in 0..64 {
            ClientMsg::Attach(Attach {
                session: "none".into(),
                ..Attach::default()
            })
            .encode(&mut out);
        }
        // The write blocks once both directions fill, and fails once the
        // server lets go, so it runs on its own and its result is ignored.
        let writing = tokio::spawn(async move {
            let _ = app.write_all(&out).await;
            app
        });
        let why = tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .expect("the connection was let go")
            .unwrap();
        assert!(why.contains("longer than"), "{why}");
        drop(writing.await);
    }
}
