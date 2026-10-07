//! Keeping a session holder running for vornd.
//!
//! vornd finds the vorn-sessiond of its own build announced under
//! `$VORN_HOME/run`, or installs and starts one, and stays connected so it
//! knows when that one goes away. A sessiond of another build, including one
//! of the same version running a different binary, is drained: it
//! takes no new sessions and exits after its last one ends. One that speaks a
//! protocol this vornd cannot is left alone, still holding its sessions, and
//! reported so the app can ask before ending them.
//!
//! With the session engine built in, the connection to the current sessiond
//! is also where every session it holds is attached and parsed (see
//! [`crate::engine`]).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tracing::{info, warn};
use vorn_sessiond::launch::{self, Instance};
use vorn_sessiond::os;
use vorn_sessiond_wire::{
    Drain, FrameReader, Hello, Message, Nonce, ToSessiond, ToVornd, Welcome, PROTO,
};

/// The sessiond protocols this vornd speaks.
pub const SESSIOND_PROTOS: std::ops::RangeInclusive<u16> = PROTO..=PROTO;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
const PING_EVERY: Duration = Duration::from_secs(15);
const RESTART_AFTER: Duration = Duration::from_secs(1);

/// Where the holder lives and what to run.
#[derive(Debug, Clone)]
pub struct HolderConfig {
    /// `$VORN_HOME`.
    pub home: PathBuf,
    /// The sessiond binary shipped with this build.
    pub bundled: PathBuf,
}

/// One running sessiond, as vornd last saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderInstance {
    pub pid: u32,
    pub instance: u128,
    pub build: String,
    pub proto: u16,
    /// Sessions it still holds, when it answered.
    pub sessions: Option<usize>,
    /// Whether this vornd can talk to it.
    pub compatible: bool,
}

#[derive(Debug, Default)]
struct State {
    current: Option<HolderInstance>,
    older: Vec<HolderInstance>,
    error: Option<String>,
}

/// What vornd knows about the session holders, for the health check.
#[derive(Debug, Default)]
pub struct Holder {
    state: Mutex<State>,
    #[cfg(feature = "engine")]
    engine: Option<std::sync::Arc<crate::engine::Engine>>,
}

impl Holder {
    /// A holder that has seen no sessiond yet.
    pub fn new() -> Holder {
        Holder::default()
    }

    /// A holder that runs every session of the current sessiond through
    /// `engine`.
    #[cfg(feature = "engine")]
    pub fn with_engine(engine: std::sync::Arc<crate::engine::Engine>) -> Holder {
        Holder {
            engine: Some(engine),
            ..Holder::default()
        }
    }

    #[cfg(feature = "engine")]
    pub fn engine(&self) -> Option<&std::sync::Arc<crate::engine::Engine>> {
        self.engine.as_ref()
    }

    /// Stays on `conn` until it ends, running its sessions when there is an
    /// engine; answers why it ended.
    async fn hold(&self, mut conn: Conn, welcome: Welcome) -> String {
        #[cfg(feature = "engine")]
        if let Some(engine) = &self.engine {
            return engine.run(conn, welcome).await;
        }
        let _ = welcome;
        conn.hold().await
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The holder's part of the health check.
    pub fn report(&self) -> Value {
        let s = self.state();
        let one = |i: &HolderInstance| {
            json!({
                "pid": i.pid,
                "instance": format!("{:x}", i.instance),
                "build": i.build,
                "proto": i.proto,
                "sessions": i.sessions,
                "compatible": i.compatible,
            })
        };
        json!({
            "current": s.current.as_ref().map(one),
            "older": s.older.iter().map(one).collect::<Vec<_>>(),
            "error": s.error,
        })
    }

    /// The sessiond new sessions go to, while one is up.
    pub fn current(&self) -> Option<HolderInstance> {
        self.state().current.clone()
    }

    /// The sessionds of other builds found at the last start.
    pub fn older(&self) -> Vec<HolderInstance> {
        self.state().older.clone()
    }
}

/// Keep a sessiond of this build running for as long as vornd runs: find or
/// start one, drain the others, and start another if it goes away.
pub async fn keep(cfg: HolderConfig, holder: std::sync::Arc<Holder>) {
    let version = match version_of(&cfg.bundled).await {
        Ok(v) => v,
        Err(err) => {
            warn!(bundled = %cfg.bundled.display(), %err, "no session holder");
            holder.state().error = Some(format!("cannot run {}: {err}", cfg.bundled.display()));
            return;
        }
    };
    loop {
        match up(&cfg, &version, &holder).await {
            Ok((conn, welcome)) => {
                holder.state().error = None;
                let why = holder.hold(conn, welcome).await;
                warn!(%why, "session holder went away; starting another");
                let mut s = holder.state();
                s.current = None;
                s.error = Some(format!("the session holder went away: {why}"));
            }
            Err(err) => {
                warn!(%err, "could not start the session holder");
                holder.state().error = Some(err.to_string());
            }
        }
        tokio::time::sleep(RESTART_AFTER).await;
    }
}

/// Connects to the sessiond at `endpoint` as its vornd and stays until the
/// connection ends, as [`keep`] does with the one it finds or starts.
/// Answers why it ended. Dropping the future drops the connection, which
/// is what a vornd killed looks like to sessiond.
pub async fn connect(endpoint: &str, holder: &Holder) -> io::Result<String> {
    let (conn, welcome) = Conn::open(endpoint).await?;
    Ok(holder.hold(conn, welcome).await)
}

/// Find or start this build's sessiond, drain the rest, and connect.
async fn up(cfg: &HolderConfig, version: &str, holder: &Holder) -> io::Result<(Conn, Welcome)> {
    let home = cfg.home.clone();
    let bundled = cfg.bundled.clone();
    let version_owned = version.to_owned();
    // Resolving paths touches the disk, so the sorting happens off the runtime.
    let (installed, mine, others) = tokio::task::spawn_blocking(move || {
        let installed = launch::install(&bundled, &home, &version_owned)?;
        let (mine, others): (Vec<Instance>, Vec<Instance>) = launch::running(&home)
            .into_iter()
            .partition(|i| i.build == version_owned && i.proto == PROTO && i.runs(&installed));
        io::Result::Ok((installed, mine, others))
    })
    .await
    .map_err(io::Error::other)??;

    let mut older = Vec::new();
    for i in others {
        older.push(drain(&i).await);
    }
    holder.state().older = older;

    let instance = match mine.into_iter().next_back() {
        Some(i) => {
            info!(pid = i.pid, build = %i.build, "found the session holder");
            i
        }
        None => {
            let home = cfg.home.clone();
            let i = tokio::task::spawn_blocking(move || {
                launch::start(&installed, &home, START_TIMEOUT)
            })
            .await
            .map_err(io::Error::other)??;
            info!(pid = i.pid, build = %i.build, "started the session holder");
            i
        }
    };
    let (conn, welcome) = Conn::open(&instance.endpoint).await?;
    holder.state().current = Some(HolderInstance {
        pid: instance.pid,
        instance: instance.instance,
        build: instance.build,
        proto: welcome.proto,
        sessions: Some(live(&welcome)),
        compatible: true,
    });
    Ok((conn, welcome))
}

/// Tell an older sessiond to take no new sessions and exit after its last
/// one. One whose protocol this vornd does not speak is only reported.
async fn drain(i: &Instance) -> HolderInstance {
    let mut seen = HolderInstance {
        pid: i.pid,
        instance: i.instance,
        build: i.build.clone(),
        proto: i.proto,
        sessions: None,
        compatible: SESSIOND_PROTOS.contains(&i.proto),
    };
    if !seen.compatible {
        warn!(
            pid = i.pid,
            proto = i.proto,
            "an older session holder speaks a protocol this vornd does not; leaving it"
        );
        return seen;
    }
    match Conn::open(&i.endpoint).await {
        Ok((mut conn, welcome)) => {
            seen.sessions = Some(live(&welcome));
            if conn.send(&ToSessiond::Drain(Drain)).await.is_ok() {
                info!(pid = i.pid, build = %i.build, sessions = live(&welcome), "draining an older session holder");
            }
        }
        Err(err) => warn!(pid = i.pid, %err, "could not reach an older session holder"),
    }
    seen
}

fn live(w: &Welcome) -> usize {
    w.sessions.iter().filter(|s| s.exited.is_none()).count()
}

/// What `bundled --version` prints.
async fn version_of(bundled: &Path) -> io::Result<String> {
    let out = tokio::process::Command::new(bundled)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || v.is_empty() {
        return Err(io::Error::other(format!(
            "--version failed ({})",
            out.status
        )));
    }
    Ok(v)
}

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

/// A connection to one sessiond.
pub(crate) struct Conn {
    rd: Reader,
    wr: Writer,
}

/// The half of a connection sessiond's messages come in on.
pub(crate) struct Reader {
    r: ReadHalf<Box<dyn Duplex>>,
    frames: FrameReader,
    /// Kept across reads: `hold` starts a new read on every ping.
    buf: Vec<u8>,
}

/// The half of a connection messages to sessiond go out on.
pub(crate) struct Writer {
    w: WriteHalf<Box<dyn Duplex>>,
}

impl Reader {
    pub(crate) async fn recv(&mut self) -> io::Result<ToVornd> {
        loop {
            match self.frames.read::<ToVornd>() {
                Ok(Some(m)) => return Ok(m),
                Ok(None) => {}
                Err(e) => return Err(io::Error::other(format!("{e:?}"))),
            }
            let n = self.r.read(&mut self.buf).await?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            self.frames.push(&self.buf[..n]);
        }
    }
}

impl Writer {
    pub(crate) async fn send(&mut self, m: &ToSessiond) -> io::Result<()> {
        self.w.write_all(&m.encode()).await
    }
}

impl Conn {
    async fn open(endpoint: &str) -> io::Result<(Conn, Welcome)> {
        let s: Box<dyn Duplex> = Box::new(os::connect(endpoint).await?);
        let (r, w) = tokio::io::split(s);
        let mut conn = Conn {
            rd: Reader {
                r,
                frames: FrameReader::default(),
                buf: vec![0u8; 16 << 10],
            },
            wr: Writer { w },
        };
        conn.send(&ToSessiond::Hello(Hello {
            proto_min: *SESSIOND_PROTOS.start(),
            proto_max: *SESSIOND_PROTOS.end(),
            vornd_instance: instance_id(),
            vornd_build: env!("CARGO_PKG_VERSION").into(),
        }))
        .await?;
        match tokio::time::timeout(ANSWER_TIMEOUT, conn.recv()).await {
            Ok(Ok(ToVornd::Welcome(w))) => Ok((conn, w)),
            Ok(Ok(_)) => Err(io::Error::other("answered without a Welcome")),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "no Welcome")),
        }
    }

    async fn send(&mut self, m: &ToSessiond) -> io::Result<()> {
        self.wr.send(m).await
    }

    async fn recv(&mut self) -> io::Result<ToVornd> {
        self.rd.recv().await
    }

    /// The two halves, for a reader and a writer that never wait on each
    /// other.
    #[cfg(feature = "engine")]
    pub(crate) fn split(self) -> (Reader, Writer) {
        (self.rd, self.wr)
    }

    /// Stay connected, pinging, until the connection ends; answers why.
    async fn hold(&mut self) -> String {
        let mut nonce = 0u64;
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                r = self.recv() => if let Err(e) = r { return e.to_string() },
                _ = ping.tick() => {
                    nonce += 1;
                    if let Err(e) = self.send(&ToSessiond::Ping(Nonce { nonce })).await {
                        return e.to_string();
                    }
                }
            }
        }
    }
}

fn instance_id() -> u128 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    nanos ^ (u128::from(std::process::id()) << 96)
}
