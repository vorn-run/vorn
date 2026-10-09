//! Keeping a session holder running for vornd.
//!
//! vornd finds the vorn-sessiond of its own build announced under
//! `$VORN_HOME/run`, or installs and starts one, and stays connected so it
//! knows when that one goes away. A sessiond of another build, including one
//! of the same version running a different binary, hands its live sessions
//! to this build's sessiond where it can (Unix, both speaking a handoff
//! protocol) and stays only to reap their programs. Otherwise, or when the
//! handoff fails, it is drained: it keeps every session, takes no new ones
//! and exits after its last one ends. One that speaks a protocol this vornd
//! cannot is left alone, still holding its sessions, and reported so the app
//! can ask before ending them.
//!
//! With the session engine built in, the connection to the current sessiond
//! is also where every session it holds is attached and parsed (see
//! [`crate::engine`]).
//!
//! A holder that is only slow is never replaced, since a second one would
//! split the sessions between two: one whose process is alive and whose
//! endpoint takes connections is waited for, round after round, and so is
//! one this vornd started that has not announced itself yet. Only one that
//! is gone (no announcement, a dead process, a refused connection) is
//! started anew.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tracing::{info, warn};
use vorn_sessiond::launch::{self, Instance, Starting};
use vorn_sessiond::os;
#[cfg(unix)]
use vorn_sessiond_wire::Adopt;
use vorn_sessiond_wire::{
    Drain, FrameReader, Hello, Message, Nonce, ToSessiond, ToVornd, Welcome, PROTO,
};

/// The sessiond protocols this vornd speaks.
pub const SESSIOND_PROTOS: std::ops::RangeInclusive<u16> = PROTO..=PROTO;

/// How long vornd waits on the session holder in each round of [`keep`].
const PATIENCE: Patience = Patience {
    start: Duration::from_secs(10),
    answer: Wait {
        warn: Duration::from_secs(5),
        give_up: Duration::from_secs(60),
    },
};
/// An older sessiond is only asked to drain, so it is not waited on for long.
const OLDER_ANSWER: Wait = Wait {
    warn: Duration::from_secs(5),
    give_up: Duration::from_secs(5),
};
const PING_EVERY: Duration = Duration::from_secs(15);
const RESTART_AFTER: Duration = Duration::from_secs(1);
/// A handoff moves fds and ring copies over a local socket; this is far past
/// any number of sessions, and only bounds a sessiond that hangs.
#[cfg(unix)]
const ADOPT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait for a sessiond's Welcome: past `warn` it is logged as
/// slow, past `give_up` this round ends and the next one asks again.
#[derive(Debug, Clone, Copy)]
struct Wait {
    warn: Duration,
    give_up: Duration,
}

/// How long one round waits for a sessiond it starts to announce itself, and
/// for the current one to answer.
#[derive(Debug, Clone, Copy)]
struct Patience {
    start: Duration,
    answer: Wait,
}

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
    /// Whether it handed its sessions to the current sessiond instead of
    /// draining; it then only reaps their programs.
    pub handed_off: bool,
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
                "handedOff": i.handed_off,
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
    // One this vornd started that has not announced itself yet.
    let mut starting = None;
    loop {
        match up(&cfg, &version, &holder, &mut starting, PATIENCE).await {
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
    let (conn, welcome) = Conn::open(endpoint, PATIENCE.answer).await?;
    Ok(holder.hold(conn, welcome).await)
}

/// Find or start this build's sessiond, have it adopt the others' sessions
/// or drain them, and connect.
async fn up(
    cfg: &HolderConfig,
    version: &str,
    holder: &Holder,
    starting: &mut Option<Starting>,
    patience: Patience,
) -> io::Result<(Conn, Welcome)> {
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

    let instance = match reachable(mine).await {
        Some(i) => {
            info!(pid = i.pid, build = %i.build, "found the session holder");
            *starting = None;
            i
        }
        None => start(installed, cfg.home.clone(), starting, patience.start).await?,
    };
    let mut older = Vec::new();
    for i in others {
        #[cfg(unix)]
        older.push(retire(&instance, &i).await);
        // ConPTY handles cannot move between processes: an older one drains.
        #[cfg(windows)]
        older.push(drain(&i, HolderInstance::older(&i)).await);
    }
    holder.state().older = older;
    // After the adoptions, so the Welcome lists what was adopted.
    let (conn, welcome) = Conn::open(&instance.endpoint, patience.answer).await?;
    holder.state().current = Some(HolderInstance {
        pid: instance.pid,
        instance: instance.instance,
        build: instance.build,
        proto: welcome.proto,
        sessions: Some(live(&welcome)),
        compatible: true,
        handed_off: false,
    });
    Ok((conn, welcome))
}

/// The newest of this build's announced sessionds that is still there. One
/// whose endpoint refuses connections or is missing is gone even if its pid
/// is alive (reused, or on its way out); one that accepts is kept however
/// slowly it answers.
async fn reachable(mut mine: Vec<Instance>) -> Option<Instance> {
    while let Some(i) = mine.pop() {
        match os::connect(&i.endpoint).await {
            Err(err) if gone(&err) => {
                warn!(pid = i.pid, %err, "an announced session holder takes no connections; not using it")
            }
            // Dropped unused: sessiond closes a connection that never says Hello.
            _ => return Some(i),
        }
    }
    None
}

/// Whether a connect error means nothing serves the endpoint, rather than
/// that it is busy.
fn gone(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

/// Start this build's sessiond and wait for it to announce itself, or keep
/// waiting for the one an earlier round started. One that has not announced
/// itself in time is kept for the next round rather than replaced.
async fn start(
    installed: PathBuf,
    home: PathBuf,
    starting: &mut Option<Starting>,
    timeout: Duration,
) -> io::Result<Instance> {
    let earlier = starting.take();
    let (s, waited) = tokio::task::spawn_blocking(move || {
        let mut s = match earlier {
            Some(s) => s,
            None => launch::spawn(&installed, &home)?,
        };
        let waited = s.wait(timeout);
        io::Result::Ok((s, waited))
    })
    .await
    .map_err(io::Error::other)??;
    match waited {
        Ok(i) => {
            info!(pid = i.pid, build = %i.build, "started the session holder");
            Ok(i)
        }
        Err(err) if err.kind() == io::ErrorKind::TimedOut => {
            warn!(
                pid = s.pid(),
                "the session holder has not announced itself yet; waiting for it"
            );
            *starting = Some(s);
            Err(err)
        }
        Err(err) => Err(err),
    }
}

impl HolderInstance {
    /// An older sessiond as announced, before it is asked anything.
    fn older(i: &Instance) -> Self {
        HolderInstance {
            pid: i.pid,
            instance: i.instance,
            build: i.build.clone(),
            proto: i.proto,
            sessions: None,
            compatible: SESSIOND_PROTOS.contains(&i.proto),
            handed_off: false,
        }
    }
}

/// Have `current` adopt the sessions of the older sessiond `i`, or, when it
/// cannot, tell `i` to drain.
#[cfg(unix)]
async fn retire(current: &Instance, i: &Instance) -> HolderInstance {
    let mut seen = HolderInstance::older(i);
    if seen.compatible && i.handoff.is_some() {
        match adopt(current, i).await {
            Ok(sessions) => {
                info!(pid = i.pid, build = %i.build, sessions, "an older session holder handed over its sessions");
                seen.sessions = Some(0);
                seen.handed_off = true;
                return seen;
            }
            Err(err) => {
                warn!(pid = i.pid, %err, "an older session holder could not hand over its sessions; draining it")
            }
        }
    }
    drain(i, seen).await
}

/// Ask `current` to take every session `from` holds. All or nothing: on an
/// error `from` still holds them all. Answers how many moved.
#[cfg(unix)]
async fn adopt(current: &Instance, from: &Instance) -> io::Result<usize> {
    let (mut conn, _) = Conn::open(&current.endpoint, PATIENCE.answer).await?;
    conn.send(&ToSessiond::Adopt(Adopt {
        req: 1,
        from: from.endpoint.clone(),
    }))
    .await?;
    let answer = async {
        loop {
            match conn.recv().await? {
                ToVornd::Adopted(a) if a.req == 1 => return Ok(a.sessions.len()),
                ToVornd::Failed(f) if f.req == 1 => return Err(io::Error::other(f.error)),
                _ => {}
            }
        }
    };
    tokio::time::timeout(ADOPT_TIMEOUT, answer)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no answer to Adopt"))?
}

/// Tell an older sessiond to take no new sessions and exit after its last
/// one. One whose protocol this vornd does not speak is only reported.
async fn drain(i: &Instance, mut seen: HolderInstance) -> HolderInstance {
    if !seen.compatible {
        warn!(
            pid = i.pid,
            proto = i.proto,
            "an older session holder speaks a protocol this vornd does not; leaving it"
        );
        return seen;
    }
    match Conn::open(&i.endpoint, OLDER_ANSWER).await {
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
    let out = vorn_spawn::tokio_command(bundled)
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
    /// Connects and says Hello. A sessiond that is slow to answer is waited
    /// for, up to `wait.give_up`.
    async fn open(endpoint: &str, wait: Wait) -> io::Result<(Conn, Welcome)> {
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
        let answer = match tokio::time::timeout(wait.warn, conn.recv()).await {
            Ok(answer) => answer,
            Err(_) => {
                warn!(
                    endpoint,
                    "the session holder is slow to answer; waiting for it"
                );
                // Cancel-safe: a frame half read stays buffered in the reader.
                tokio::time::timeout(wait.give_up.saturating_sub(wait.warn), conn.recv())
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("no Welcome after {:?}", wait.give_up),
                        )
                    })?
            }
        };
        match answer? {
            ToVornd::Welcome(w) => Ok((conn, w)),
            _ => Err(io::Error::other("answered without a Welcome")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Arc;
    use vorn_sessiond::server;

    const VERSION: &str = "0.0.0-test";

    /// [`PATIENCE`] scaled down, so a slow holder takes milliseconds to show.
    fn quick(give_up: Duration) -> Patience {
        Patience {
            start: Duration::from_millis(300),
            answer: Wait {
                warn: Duration::from_millis(100),
                give_up,
            },
        }
    }

    /// A home whose bundled sessiond is `program`, installed as vornd would.
    fn home_with(program: &[u8]) -> (tempfile::TempDir, HolderConfig, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let bundled = home.path().join("bundled-sessiond");
        std::fs::write(&bundled, program).unwrap();
        let installed = launch::install(&bundled, home.path(), VERSION).unwrap();
        let cfg = HolderConfig {
            home: home.path().to_owned(),
            bundled,
        };
        (home, cfg, installed)
    }

    fn announced(cfg: &HolderConfig, installed: &Path, instance: u128) -> Instance {
        let i = Instance {
            endpoint: server::endpoint(&cfg.home, instance),
            // Alive for as long as the test runs.
            pid: std::process::id(),
            proto: PROTO,
            build: VERSION.into(),
            instance,
            exe: Some(installed.to_owned()),
            handoff: None,
        };
        launch::announce(&cfg.home, &i).unwrap();
        i
    }

    /// A sessiond of this build, served from this process, that answers each
    /// Hello after `delay` (never, for `None`); counts the Hellos.
    fn slow_holder(
        cfg: &HolderConfig,
        installed: &Path,
        delay: Option<Duration>,
    ) -> (Instance, Arc<AtomicUsize>) {
        let i = announced(cfg, installed, 0xfeed);
        let mut listener = os::Listener::bind(&cfg.home, &i.endpoint).unwrap();
        let hellos = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hellos);
        let instance = i.instance;
        tokio::spawn(async move {
            while let Ok(mut s) = listener.accept().await {
                let counted = Arc::clone(&counted);
                tokio::spawn(async move {
                    let mut frames = FrameReader::default();
                    let mut buf = [0u8; 4096];
                    loop {
                        match frames.read::<ToSessiond>() {
                            Ok(Some(ToSessiond::Hello(_))) => break,
                            Ok(_) => {}
                            Err(_) => return,
                        }
                        match s.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => frames.push(&buf[..n]),
                        }
                    }
                    counted.fetch_add(1, SeqCst);
                    if let Some(delay) = delay {
                        tokio::time::sleep(delay).await;
                        let welcome = ToVornd::Welcome(Welcome {
                            proto: PROTO,
                            sessiond_instance: instance,
                            sessiond_build: VERSION.into(),
                            sessions: Vec::new(),
                        });
                        let _ = s.write_all(&welcome.encode()).await;
                    }
                    // Held open, as sessiond holds its vornd's connection.
                    std::future::pending::<()>().await;
                });
            }
        });
        (i, hellos)
    }

    #[tokio::test]
    async fn a_holder_that_answers_after_the_timeout_is_reused_not_duplicated() {
        // Not a program: had vornd tried to start another, it would fail at once.
        let (home, cfg, installed) = home_with(b"not a sessiond");
        let (fake, hellos) = slow_holder(&cfg, &installed, Some(Duration::from_millis(500)));
        let mut starting = None;
        let (_conn, welcome) = up(
            &cfg,
            VERSION,
            &Holder::new(),
            &mut starting,
            quick(Duration::from_secs(10)),
        )
        .await
        .expect("the slow holder is used");
        assert_eq!(welcome.sessiond_instance, fake.instance);
        assert_eq!(hellos.load(SeqCst), 1, "one Hello, waited on");
        assert!(starting.is_none());
        assert_eq!(launch::running(home.path()), vec![fake]);
    }

    #[tokio::test]
    async fn a_holder_that_does_not_answer_is_asked_again_not_replaced() {
        let (home, cfg, installed) = home_with(b"not a sessiond");
        let (fake, hellos) = slow_holder(&cfg, &installed, None);
        let mut starting = None;
        for round in 1..=2 {
            let err = up(
                &cfg,
                VERSION,
                &Holder::new(),
                &mut starting,
                quick(Duration::from_millis(300)),
            )
            .await
            .err()
            .expect("no Welcome");
            assert_eq!(err.kind(), io::ErrorKind::TimedOut, "round {round}: {err}");
            assert_eq!(hellos.load(SeqCst), round);
        }
        assert!(starting.is_none(), "nothing was started");
        assert_eq!(launch::running(home.path()), vec![fake]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_refused_holder_is_replaced_and_a_slow_start_is_waited_for() {
        // Starts, but never announces itself.
        let (_home, cfg, installed) = home_with(b"#!/bin/sh\nexec sleep 30\n");
        // Announced and alive, but nothing serves its endpoint.
        let refused = announced(&cfg, &installed, 0xdead);
        assert!(!Path::new(&refused.endpoint).exists());
        let mut starting = None;
        let err = up(
            &cfg,
            VERSION,
            &Holder::new(),
            &mut starting,
            quick(Duration::from_secs(1)),
        )
        .await
        .err()
        .expect("the new one never announces itself");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        let pid = starting
            .as_ref()
            .map(Starting::pid)
            .expect("one was started");
        let err = up(
            &cfg,
            VERSION,
            &Holder::new(),
            &mut starting,
            quick(Duration::from_secs(1)),
        )
        .await
        .err()
        .expect("still not announced");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert_eq!(
            starting.as_ref().map(Starting::pid),
            Some(pid),
            "the same one is waited for, not a second"
        );
        let _ = launch::kill(pid);
    }
}
