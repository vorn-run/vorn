//! vornd as sessiond's client: every session the current sessiond holds
//! runs through the session engine ([`vorn_engine`]), which is the only
//! place its output is parsed.
//!
//! The driver here is a pipe between one sessiond connection and the
//! engine's worker pool. What sessiond sends about a session goes to that
//! session's actor; what the actor asks for (an attach, an ack, a
//! checkpoint to keep, a reply to a query) goes back on the same
//! connection, since sessiond serves one vornd connection at a time. A new
//! connection, after sessiond or vornd restarted, starts a new pool and
//! recovers every session from sessiond's Welcome.
//!
//! The connection is read and written by two tasks of their own, and the
//! driver waits on neither. sessiond stops reading while its outbox is full,
//! so a vornd that stopped reading while it wrote (a big checkpoint, say)
//! would leave both ends waiting on each other. The reader always drains
//! sessiond; the writer works through its own queue, and gives the
//! connection up when one write takes longer than [`WRITE_TIMEOUT`], so a
//! stuck sessiond costs a reconnect rather than a hang.
//!
//! A session leaves the engine when its program has ended and every record
//! is applied: the driver then releases it in sessiond. A lost session
//! leaves it too, and is taken on again from the next connection's Welcome.
//! [`Engine::subscribe`] carries what happens to them, and they are
//! reported, without their contents, at [`SESSIONS_PATH`]. Bytes clients
//! read them through [`Engine::streams`] ([`crate::streams`]), which is fed
//! every record the actors apply. Grid clients ([`crate::grid`]) reach them
//! through [`Engine::grid_open`] and [`Engine::grid_input`]: their requests
//! go to the session's actor, and what it answers comes back on the
//! connection's queue. Each session's size is decided by [`Engine::sizes`]
//! ([`crate::size`]); the driver sends what it decides to sessiond, and
//! tells every client who asked for each resize when its record comes back.
//!
//! With the Native server switch on, [`Engine::decide_statuses`] has the
//! copy of the server's session records ([`crate::registry`]) decide each
//! terminal's status from what its session does: its screen's status, its
//! output and its going quiet.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use vorn_engine::{
    Brief, Config, Effect, EffectId, Fidelity, GridIn, HubOut, Input, Open, Out, Peer, Pool,
    Summary,
};
use vorn_sessiond_wire::{
    Ack, Attach, AttachFrom, Io, Nonce, Resize, SessionRef, Sig, Signal, Spawn, SpawnSpec,
    ToSessiond, ToVornd, Welcome, Write,
};
use vorn_term_proto::msg::{ResizeReason, ServerMsg};
use vorn_term_proto::{Cursor, Entry, Record};

use crate::holder::{Conn, Writer};
use crate::journal::{Journal, Kind};
use crate::names::Names;
use crate::registry::{SessionRegistry, Stamp};
use crate::size::{Sizes, Who};
use crate::streams::{Action, Snap, Streams};

/// The path the session report is served at.
pub const SESSIONS_PATH: &str = "/vornd/sessions";

/// How long one write to sessiond may take before the connection is given
/// up. A local socket to a sessiond that reads takes milliseconds for the
/// biggest checkpoint.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(20);

const PING_EVERY: Duration = Duration::from_secs(15);

/// Bytes queued for sessiond and not yet written, past which the connection
/// is given up: a sessiond that stopped reading costs a reconnect, and the
/// queue never grows past this while one write waits out
/// [`WRITE_TIMEOUT`].
pub const WRITE_QUEUE_CAP: usize = 64 << 20;

/// The default foreground and background a session's terminal answers
/// colour queries (OSC 10, 11) with: the app's terminal theme, which is what
/// xterm.js answered with before vornd answered every query.
pub const DEFAULT_COLORS: ([u8; 3], [u8; 3]) = ([0xd4, 0xd4, 0xd8], [0x14, 0x14, 0x16]);

/// How often attaches waiting on a snapshot or a fetch are checked.
const EXPIRE_EVERY: Duration = Duration::from_secs(1);

/// How long a clean stop waits for the last checkpoints to be sent.
const LAST_CHECKPOINTS: Duration = Duration::from_secs(5);

/// Sessions that left the engine, kept for the report.
const CLOSED_KEPT: usize = 64;

/// Events kept for a subscriber that falls behind.
const EVENTS: usize = 1024;

enum Command {
    /// Start a session, under a name when one is given.
    Spawn(
        SpawnSpec,
        Option<String>,
        oneshot::Sender<Result<Spawned, String>>,
    ),
    /// A signal for a session's program.
    Signal(String, Sig),
    Write(String, Vec<u8>),
    CloseStdin(String),
    /// Cut a last checkpoint for every session and send them.
    Flush(oneshot::Sender<()>),
    /// A client's resize, for sessiond to apply and record.
    Resize(String, u16, u16),
    /// Records from this cursor again, for a client continuing from it.
    Fetch(String, Cursor),
}

/// What happened to a session, for whoever listens.
#[derive(Debug, Clone)]
pub enum Event {
    Effect(EffectId, Effect),
    /// Replay reached the head: the session is live from here on.
    Ready {
        session: String,
        fidelity: Fidelity,
    },
    /// The session left the engine: its program ended, or it was lost.
    /// How it stood last.
    Closed(Arc<Summary>),
    /// The session printed: reported at most once a second while it does
    /// ([`crate::journal::ACTIVITY_EVERY`]).
    Activity(String),
    /// The engine connected to sessiond and opened every session it holds.
    Connected,
}

/// A session started through the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    /// The id it goes by: its name, or sessiond's id when it has none.
    pub id: String,
    pub pid: u32,
    /// The epoch its log starts in: a reader that wants every byte
    /// attaches from `{epoch, 0, 0}`.
    pub epoch: u32,
}

/// Messages queued for one grid connection before it drops behind. A
/// client holds two frames in flight at most, so this many means it stopped
/// reading; it is disconnected and resumes with a snapshot (TP §11).
pub const GRID_QUEUE: usize = 256;

/// The grid connections and the input they are waiting to have written.
#[derive(Default)]
struct GridConns {
    next: u64,
    conns: HashMap<u64, mpsc::Sender<ServerMsg>>,
    /// The write each grid input became, by the driver's `input_seq`, and
    /// the event it answers: its InputAck goes out when sessiond says the
    /// bytes were written.
    inputs: HashMap<u64, (Peer, u64)>,
}

/// The connection the engine is running on, while there is one.
struct Current {
    pool: Arc<Pool>,
    commands: mpsc::UnboundedSender<Command>,
}

/// The session engine as vornd runs it, across connections.
pub struct Engine {
    cfg: Config,
    write_timeout: Duration,
    current: Mutex<Option<Current>>,
    closed: Mutex<VecDeque<Brief>>,
    events: broadcast::Sender<Event>,
    grid: Mutex<GridConns>,
    streams: Arc<Streams>,
    sizes: Arc<Sizes>,
    names: Mutex<Names>,
    journal: Mutex<Journal>,
    /// The copy of the app's session records, which the app's channel feeds.
    registry: Arc<SessionRegistry>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("cfg", &self.cfg)
            .field("write_timeout", &self.write_timeout)
            .field("connected", &self.current().is_some())
            .finish()
    }
}

impl Engine {
    pub fn new(cfg: Config) -> Arc<Engine> {
        Engine::with_write_timeout(cfg, WRITE_TIMEOUT)
    }

    /// An engine that gives a connection up after a write has taken
    /// `write_timeout`.
    pub fn with_write_timeout(cfg: Config, write_timeout: Duration) -> Arc<Engine> {
        let streams = Streams::new();
        let sizes = Arc::new(Sizes::new());
        // A bytes connection that closes leaves every session's size rule.
        let left = Arc::clone(&sizes);
        streams.on_disconnect(Box::new(move |conn| {
            left.bytes_gone(conn, std::time::Instant::now())
        }));
        // Kept beside the history, in vornd's own directory.
        let names_file = cfg
            .history
            .as_ref()
            .and_then(|h| h.parent())
            .map(|d| d.join("names.json"));
        Arc::new(Engine {
            names: Mutex::new(Names::load(names_file)),
            journal: Mutex::new(Journal::default()),
            registry: SessionRegistry::new(),
            // Every session's records go on to bytes clients.
            cfg: Config {
                stream: true,
                colors: cfg.colors.or(Some(DEFAULT_COLORS)),
                ..cfg
            },
            write_timeout,
            current: Mutex::new(None),
            closed: Mutex::new(VecDeque::new()),
            events: broadcast::channel(EVENTS).0,
            grid: Mutex::new(GridConns::default()),
            streams,
            sizes,
        })
    }

    /// Every session's size rule.
    pub fn sizes(&self) -> &Arc<Sizes> {
        &self.sizes
    }

    /// The terminal streams of the sessions this engine holds.
    pub fn streams(&self) -> &Arc<Streams> {
        &self.streams
    }

    /// The app's session records, as the app's channel told them.
    pub fn registry(&self) -> &Arc<SessionRegistry> {
        &self.registry
    }

    /// Has the registry decide the server's terminals' statuses from now on
    /// ([`crate::registry::Registry::decide_statuses`]), following what
    /// every session does on a task of its own. Needs a runtime; a second
    /// call does nothing.
    pub fn decide_statuses(self: &Arc<Self>) {
        if self.registry.decides() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!("no runtime to follow the sessions on; the server keeps deciding their statuses");
            return;
        };
        self.registry.decide_statuses();
        runtime.spawn(follow_statuses(Arc::clone(self), self.subscribe()));
    }

    /// The stamp of a state told for `session` now ([`Stamp::at_head`]).
    pub fn head_stamp(&self, session: &str) -> Option<Stamp> {
        self.streams.head(session).map(|c| Stamp::at_head(&c))
    }

    /// A resize of `session`, past the size rule. sessiond applies it and
    /// records it, and the terminal and every client resize when they reach
    /// the record. Clients' resizes go through [`Engine::sizes`] instead.
    pub fn resize(&self, session: &str, cols: u16, rows: u16) -> Result<(), String> {
        self.command(Command::Resize(session.to_owned(), cols, rows))
    }

    /// Carries out what the streams asked for. A session that cannot be
    /// asked answers its client with an error at once.
    pub fn perform(&self, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        let pool = self.current().as_ref().map(|c| Arc::clone(&c.pool));
        for a in actions {
            match a {
                Action::Snapshot { session, token } => {
                    if !pool.as_ref().is_some_and(|p| p.snapshot(&session, token)) {
                        self.streams.failed(token);
                    }
                }
                Action::Output {
                    session,
                    token,
                    lines,
                } => {
                    if !pool
                        .as_ref()
                        .is_some_and(|p| p.output(&session, token, lines))
                    {
                        self.streams.failed(token);
                    }
                }
                Action::Fetch { session, from } => {
                    let _ = self.command(Command::Fetch(session, from));
                }
            }
        }
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<Current>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn closed(&self) -> std::sync::MutexGuard<'_, VecDeque<Brief>> {
        self.closed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn names(&self) -> std::sync::MutexGuard<'_, Names> {
        self.names.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What the app is told about the sessions: their states and the last
    /// notifications ([`crate::journal`]).
    pub fn journal(&self) -> std::sync::MutexGuard<'_, Journal> {
        self.journal.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn grid_conns(&self) -> std::sync::MutexGuard<'_, GridConns> {
        self.grid.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new grid connection: its id, and the queue of what sessions send
    /// it. The queue closes when the connection falls too far behind.
    pub fn grid_open(&self) -> (u64, mpsc::Receiver<ServerMsg>) {
        let (tx, rx) = mpsc::channel(GRID_QUEUE);
        let mut g = self.grid_conns();
        g.next += 1;
        let conn = g.next;
        g.conns.insert(conn, tx);
        (conn, rx)
    }

    /// A grid connection ended: its attachments on `sessions` go.
    pub fn grid_close(&self, conn: u64, sessions: impl IntoIterator<Item = String>) {
        {
            let mut g = self.grid_conns();
            g.conns.remove(&conn);
            g.inputs.retain(|_, (peer, _)| peer.conn != conn);
        }
        self.sizes.grid_gone(conn, std::time::Instant::now());
        for s in sessions {
            let _ = self.grid_input(&s, GridIn::Gone { conn });
        }
    }

    /// Whether the engine is connected to a sessiond.
    pub fn connected(&self) -> bool {
        self.current().is_some()
    }

    /// Whether the engine runs session `id` now.
    pub fn has_session(&self, id: &str) -> bool {
        self.current()
            .as_ref()
            .is_some_and(|c| c.pool.briefs().iter().any(|b| b.session == id))
    }

    /// A grid client's request for session `id`, for its actor.
    pub fn grid_input(&self, id: &str, m: GridIn) -> Result<(), String> {
        let current = self.current();
        let c = current
            .as_ref()
            .ok_or_else(|| "no session holder connected".to_owned())?;
        if c.pool.grid(id, m) {
            Ok(())
        } else {
            Err(format!("no session {id}"))
        }
    }

    /// A message for grid connection `conn`. One that cannot take it now is
    /// let go.
    fn grid_send(&self, conn: u64, msg: ServerMsg) {
        let mut g = self.grid_conns();
        if let Some(tx) = g.conns.get(&conn) {
            if tx.try_send(msg).is_err() {
                warn!(conn, "grid client fell behind; disconnecting it");
                g.conns.remove(&conn);
            }
        }
    }

    fn command(&self, c: Command) -> Result<(), String> {
        self.current()
            .as_ref()
            .ok_or_else(|| "no session holder connected".to_owned())?
            .commands
            .send(c)
            .map_err(|_| "the session holder connection closed".to_owned())
    }

    /// What happens to sessions from now on. A subscriber that falls more
    /// than a thousand events behind misses the oldest.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Starts a session in sessiond and runs it through the engine.
    /// Answers its id.
    pub async fn spawn(&self, spec: SpawnSpec) -> Result<String, String> {
        self.spawn_as(spec, None).await.map(|s| s.id)
    }

    /// Starts a session in sessiond under `name` ([`crate::names`]), or
    /// under sessiond's id with none, and runs it through the engine.
    /// Refused when another session goes by the name.
    pub async fn spawn_as(&self, spec: SpawnSpec, name: Option<String>) -> Result<Spawned, String> {
        let (tx, rx) = oneshot::channel();
        self.command(Command::Spawn(spec, name, tx))?;
        rx.await
            .map_err(|_| "the session holder connection closed".to_owned())?
    }

    /// A signal for the program of `session`.
    pub fn signal(&self, session: &str, signal: Sig) -> Result<(), String> {
        self.command(Command::Signal(session.to_owned(), signal))
    }

    /// Input for a session's program.
    pub fn write(&self, session: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.command(Command::Write(session.to_owned(), bytes))
    }

    pub fn close_stdin(&self, session: &str) -> Result<(), String> {
        self.command(Command::CloseStdin(session.to_owned()))
    }

    /// Every session in the engine, contents included. For tests and
    /// in-process callers only: [`Engine::report`] is what goes over HTTP.
    pub async fn sessions(&self) -> Vec<Summary> {
        let Some(pool) = self.current().as_ref().map(|c| Arc::clone(&c.pool)) else {
            return Vec::new();
        };
        tokio::task::spawn_blocking(move || pool.sessions())
            .await
            .unwrap_or_default()
    }

    /// The debug report: where each session is and how it was recovered,
    /// and the last few that left the engine. Never the screen, title or
    /// cwd: the endpoint answers anyone on this machine. Read from what the
    /// workers last published, so it never waits on one.
    pub fn report(&self) -> Value {
        let (connected, briefs) = match self.current().as_ref() {
            Some(c) => (true, c.pool.briefs()),
            None => (false, Vec::new()),
        };
        let closed: Vec<Value> = self.closed().iter().map(brief).collect();
        json!({
            "connected": connected,
            "sessions": briefs.iter().map(brief).collect::<Vec<_>>(),
            "closed": closed,
        })
    }

    /// [`Engine::report`] with each running session's state digest
    /// ([`Summary::digest`]) as `"digest"`, 16 hex digits: what `?digest=1`
    /// on [`SESSIONS_PATH`] answers, for tests that compare a recovered
    /// terminal with one that never died. The digest is a hash of the
    /// terminal's whole state, so it says whether two terminals agree and
    /// nothing of what either shows. It walks every session's cells on the
    /// workers, off the runtime's threads, so it waits on them as
    /// [`Engine::sessions`] does.
    pub async fn report_with_digests(&self) -> Value {
        let digests: HashMap<String, u64> = self
            .sessions()
            .await
            .into_iter()
            .filter_map(|s| Some((s.brief.session, s.digest?)))
            .collect();
        let mut report = self.report();
        if let Some(Value::Array(sessions)) = report.get_mut("sessions") {
            for s in sessions {
                let digest = s
                    .get("session")
                    .and_then(Value::as_str)
                    .and_then(|id| digests.get(id));
                if let (Some(d), Value::Object(fields)) = (digest, s) {
                    fields.insert("digest".into(), Value::String(format!("{d:016x}")));
                }
            }
        }
        report
    }

    /// Cuts a last checkpoint for every session and hands them to sessiond,
    /// as a clean stop does, waiting at most a few seconds.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.command(Command::Flush(tx)).is_ok() {
            let _ = tokio::time::timeout(LAST_CHECKPOINTS, rx).await;
        }
    }

    /// Runs every session `conn` holds until the connection ends; answers
    /// why it ended.
    pub(crate) async fn run(&self, conn: Conn, welcome: Welcome) -> String {
        let (mut reader, writer) = conn.split();
        let (in_tx, mut incoming) = mpsc::unbounded_channel();
        let mut reading: JoinHandle<String> = tokio::spawn(async move {
            loop {
                match reader.recv().await {
                    Ok(m) => {
                        if in_tx.send(m).is_err() {
                            return String::new();
                        }
                    }
                    Err(e) => return e.to_string(),
                }
            }
        });
        let (to_send, queued) = mpsc::unbounded_channel();
        let backlog = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut writing = tokio::spawn(write_queued(
            writer,
            queued,
            self.write_timeout,
            Arc::clone(&backlog),
        ));
        // The tasks end with the connection, however that ends.
        let _tasks = Abort([reading.abort_handle(), writing.abort_handle()]);

        let (out_tx, mut outs) = mpsc::unbounded_channel::<(String, Out)>();
        let sink = Arc::new(move |id: &str, o: Out| {
            let _ = out_tx.send((id.to_owned(), o));
        });
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
        let pool = match Pool::new(threads, self.cfg.clone(), sink) {
            Ok(p) => Arc::new(p),
            Err(e) => return format!("could not start the session engine: {e}"),
        };
        let (cmd_tx, mut commands) = mpsc::unbounded_channel();
        // This connection's driver numbers its writes from 0 again: input
        // waiting on the last one's numbers would be acknowledged by the
        // wrong writes, and those were never written.
        self.grid_conns().inputs.clear();
        *self.current() = Some(Current {
            pool: Arc::clone(&pool),
            commands: cmd_tx,
        });
        // However this ends, a dropped future included, the sessions go
        // with the connection.
        let _clear = Clear(self);
        // Names of sessions this sessiond no longer holds go; every other
        // session goes by its name from here on.
        self.names()
            .keep_only(welcome.sessions.iter().map(|i| i.session.as_str()));
        for info in &welcome.sessions {
            let id = self.names().public(&info.session).to_owned();
            self.streams.opened(&id, info.epoch);
            let open = Open::from_info(info);
            if open.pty {
                self.sizes.opened(&id, size_of(open.size));
            }
            let kind = if open.pty { Kind::Pty } else { Kind::Piped };
            self.journal().opened(&id, kind, info.pid);
            pool.open(&id, open);
        }
        {
            let ids: std::collections::HashSet<String> = welcome
                .sessions
                .iter()
                .map(|i| self.names().public(&i.session).to_owned())
                .collect();
            self.journal().keep_only(|id| ids.contains(id));
        }
        info!(sessions = welcome.sessions.len(), "recovering sessions");
        let _ = self.events.send(Event::Connected);
        // The pool moves into the driver so that `_clear` drops the last
        // reference, off the runtime's threads.
        let mut d = Driver {
            engine: self,
            pool,
            to_send,
            backlog,
            overflowed: false,
            spawns: HashMap::new(),
            next_req: 0,
            input_seq: 0,
            repumping: std::collections::HashSet::new(),
        };
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        let mut expire = tokio::time::interval(EXPIRE_EVERY);
        let mut nonce = 0u64;
        let why = loop {
            tokio::select! {
                m = incoming.recv() => match m {
                    Some(m) => d.received(m),
                    None => break ended(&mut reading).await,
                },
                Some((id, o)) = outs.recv() => d.asked(&id, o),
                Some(c) = commands.recv() => d.command(c, &mut outs).await,
                _ = ping.tick() => {
                    nonce += 1;
                    d.send(ToSessiond::Ping(Nonce { nonce }));
                }
                _ = expire.tick() => self.streams.expire(std::time::Instant::now()),
                () = until(self.sizes.due()) => d.resize_due(),
                () = self.sizes.woken() => d.resize_due(),
                r = &mut writing => break r.unwrap_or_else(|e| e.to_string()),
            }
            if d.overflowed {
                break format!(
                    "more than {WRITE_QUEUE_CAP} bytes queued for sessiond and not written"
                );
            }
        };
        for (_, p) in d.spawns.drain() {
            let _ = p.reply.send(Err(why.clone()));
        }
        self.streams.suspended();
        // The pool stops with the last reference, without last checkpoints:
        // the sessions are sessiond's, and the next connection recovers them.
        why
    }
}

/// Resolves at `at`, or never.
async fn until(at: Option<std::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        None => std::future::pending().await,
    }
}

fn size_of((cols, rows): (u16, u16)) -> vorn_size::Size {
    vorn_size::Size::new(cols, rows)
}

fn wire_reason(r: vorn_size::Reason) -> ResizeReason {
    match r {
        vorn_size::Reason::Input => ResizeReason::Input,
        vorn_size::Reason::Returned => ResizeReason::Returned,
        vorn_size::Reason::Explicit => ResizeReason::Explicit,
        vorn_size::Reason::Locked => ResizeReason::Locked,
        vorn_size::Reason::Launch => ResizeReason::Launch,
    }
}

/// Why a task that ended did.
async fn ended(task: &mut JoinHandle<String>) -> String {
    task.await.unwrap_or_else(|e| e.to_string())
}

/// What the writer is given.
enum Queued {
    /// A message and the bytes it counts against the backlog.
    Message(ToSessiond, usize),
    /// Answered once everything queued before it is written.
    Written(oneshot::Sender<()>),
}

/// Writes what is queued, in order, until the connection fails or a write
/// takes `timeout`; answers why it stopped.
async fn write_queued(
    mut w: Writer,
    mut queued: mpsc::UnboundedReceiver<Queued>,
    timeout: Duration,
    backlog: Arc<std::sync::atomic::AtomicUsize>,
) -> String {
    while let Some(q) = queued.recv().await {
        match q {
            Queued::Message(m, size) => match tokio::time::timeout(timeout, w.send(&m)).await {
                Ok(Ok(())) => {
                    backlog.fetch_sub(size, std::sync::atomic::Ordering::AcqRel);
                }
                Ok(Err(e)) => return e.to_string(),
                Err(_) => return format!("a write to sessiond timed out after {timeout:?}"),
            },
            Queued::Written(done) => {
                let _ = done.send(());
            }
        }
    }
    "the engine stopped".to_owned()
}

struct Abort<const N: usize>([tokio::task::AbortHandle; N]);

impl<const N: usize> Drop for Abort<N> {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

struct Clear<'a>(&'a Engine);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        let Some(c) = self.0.current().take() else {
            return;
        };
        // Dropping the pool joins its workers, each finishing the job in
        // hand: not on a runtime thread.
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn_blocking(move || drop(c));
            }
            Err(_) => drop(c),
        }
    }
}

fn fidelity(f: Fidelity) -> &'static str {
    match f {
        Fidelity::Exact => "exact",
        Fidelity::Approximate => "approximate",
    }
}

fn brief(s: &Brief) -> Value {
    json!({
        "session": s.session,
        "state": s.state.as_str(),
        "base": s.base.map(|b| b.as_str()),
        "fidelity": fidelity(s.fidelity),
        "reason": s.reason,
        "rejected": s.rejected,
        "cursor": s.cursor.map(|c| json!({
            "epoch": c.epoch,
            "nextRseq": c.next_rseq,
            "nextOffset": c.next_offset,
        })),
        "cols": s.cols,
        "rows": s.rows,
        "checkpoints": s.checkpoints,
        "uncut": s.uncut,
        "exited": s.exited.map(|(code, signal)| json!({ "code": code, "signal": signal })),
    })
}

/// A spawn sent and not answered yet.
struct Pending {
    reply: oneshot::Sender<Result<Spawned, String>>,
    /// The PTY size, or `None` for a piped agent.
    size: Option<(u16, u16)>,
    /// The name the session is to go by.
    name: Option<String>,
}

/// The session a message to sessiond names.
fn outbound_session(m: &mut ToSessiond) -> Option<&mut String> {
    match m {
        ToSessiond::Attach(a) => Some(&mut a.session),
        ToSessiond::Write(w) => Some(&mut w.session),
        ToSessiond::CloseStdin(r) | ToSessiond::Release(r) => Some(&mut r.session),
        ToSessiond::Resize(r) => Some(&mut r.session),
        ToSessiond::Signal(s) => Some(&mut s.session),
        ToSessiond::Ack(a) => Some(&mut a.session),
        ToSessiond::PutCheckpoint(cp) => Some(&mut cp.session),
        ToSessiond::Hello(_)
        | ToSessiond::Spawn(_)
        | ToSessiond::Ping(_)
        | ToSessiond::Drain(_) => None,
    }
}

/// The session a message from sessiond names. A spawn's answer is named
/// where the spawn is matched to its request.
fn inbound_session(m: &mut ToVornd) -> Option<&mut String> {
    match m {
        ToVornd::CheckpointIs(cp) => Some(&mut cp.session),
        ToVornd::Refused(r) => Some(&mut r.session),
        ToVornd::Entries(e) => Some(&mut e.session),
        ToVornd::InputDone(d) => Some(&mut d.session),
        ToVornd::Welcome(_) | ToVornd::Spawned(_) | ToVornd::Failed(_) | ToVornd::Pong(_) => None,
    }
}

/// A rough size of a message to sessiond, for the writer's backlog.
fn queued_size(m: &ToSessiond) -> usize {
    match m {
        ToSessiond::Write(w) => w.bytes.len() + 64,
        ToSessiond::PutCheckpoint(cp) => cp.blob.len() + 128,
        _ => 64,
    }
}

struct Driver<'a> {
    engine: &'a Engine,
    pool: Arc<Pool>,
    /// The writer's queue. Sending never waits; a writer that has stopped
    /// ends the connection, which the driver learns from its task.
    to_send: mpsc::UnboundedSender<Queued>,
    /// Bytes queued for the writer and not yet written.
    backlog: Arc<std::sync::atomic::AtomicUsize>,
    /// The backlog passed [`WRITE_QUEUE_CAP`]: the connection is given up.
    overflowed: bool,
    /// Spawns sent and not answered: who asked, and the PTY size.
    spawns: HashMap<u64, Pending>,
    next_req: u64,
    /// Numbers this connection's writes.
    input_seq: u64,
    /// Sessions re-attached from vornd's own cursor after a refused fetch,
    /// until sessiond answers.
    repumping: std::collections::HashSet<String>,
}

impl Driver<'_> {
    /// Queues `m` for sessiond, under sessiond's id for the session it
    /// names.
    fn send(&mut self, mut m: ToSessiond) {
        if let Some(session) = outbound_session(&mut m) {
            let held = self.engine.names().held(session).to_owned();
            *session = held;
        }
        let size = queued_size(&m);
        let queued = self
            .backlog
            .fetch_add(size, std::sync::atomic::Ordering::AcqRel)
            + size;
        if queued > WRITE_QUEUE_CAP {
            self.overflowed = true;
        }
        let _ = self.to_send.send(Queued::Message(m, size));
    }

    /// A message from sessiond, for the session it names, which goes by its
    /// name from here on.
    fn received(&mut self, mut m: ToVornd) {
        if let Some(session) = inbound_session(&mut m) {
            let public = self.engine.names().public(session).to_owned();
            *session = public;
        }
        match m {
            ToVornd::Entries(e) => {
                self.repumping.remove(&e.session);
                self.pool.input(&e.session, Input::Entries(e.entries))
            }
            ToVornd::CheckpointIs(cp) => {
                let id = cp.session.clone();
                self.pool.input(&id, Input::Checkpoint(cp));
            }
            // A refusal of a client's fetch is the streams', not the actor's:
            // fetches are made only for live sessions, which ask for nothing.
            ToVornd::Refused(r) if self.engine.streams.fetching(&r.session) => {
                let actions = self.engine.streams.fetch_refused(&r.session, r.why);
                self.engine.perform(actions);
                self.repump(r.session);
            }
            // The cursor a repump asked from is gone too: the newest
            // checkpoint is always held, and the actor skips what it has.
            ToVornd::Refused(r) if self.repumping.remove(&r.session) => {
                self.send(ToSessiond::Attach(Attach {
                    session: r.session,
                    from: AttachFrom::NewestCheckpoint,
                }));
            }
            ToVornd::Refused(r) => self.pool.input(&r.session, Input::Refused(r.why)),
            ToVornd::Spawned(s) => {
                if let Some(p) = self.spawns.remove(&s.req) {
                    let id = match p.name {
                        Some(name) => {
                            self.engine.names().name(&s.session, &name);
                            name
                        }
                        None => s.session,
                    };
                    self.engine.streams.opened(&id, s.start.epoch);
                    if let Some(size) = p.size {
                        self.engine.sizes.opened(&id, size_of(size));
                    }
                    let kind = if p.size.is_some() {
                        Kind::Pty
                    } else {
                        Kind::Piped
                    };
                    self.engine.journal().opened(&id, kind, s.pid);
                    self.pool.open(&id, Open::spawned(s.start, p.size));
                    let _ = p.reply.send(Ok(Spawned {
                        id,
                        pid: s.pid,
                        epoch: s.start.epoch,
                    }));
                }
            }
            ToVornd::Failed(f) => {
                if let Some(p) = self.spawns.remove(&f.req) {
                    let _ = p.reply.send(Err(f.error));
                }
            }
            ToVornd::InputDone(done) => {
                let acked = self.engine.grid_conns().inputs.remove(&done.input_seq);
                if let Some((peer, input_seq)) = acked {
                    self.engine.grid_send(
                        peer.conn,
                        ServerMsg::InputAck {
                            sid: peer.sid,
                            input_seq,
                        },
                    );
                }
            }
            ToVornd::Pong(_) | ToVornd::Welcome(_) => {}
        }
    }

    /// What session `id` asked for, in the order it asked. An effect is
    /// delivered here before any checkpoint after it is queued for sessiond,
    /// which is what lets recovery from that checkpoint skip it.
    fn asked(&mut self, id: &str, o: Out) {
        let session = id.to_owned();
        match o {
            Out::Attach(from) => self.send(ToSessiond::Attach(Attach { session, from })),
            Out::Ack(delivered) => self.send(ToSessiond::Ack(Ack { session, delivered })),
            Out::Checkpoint(cp) => {
                debug!(
                    session = id,
                    rseq = cp.resume.next_rseq,
                    bytes = cp.blob.len(),
                    "checkpoint"
                );
                self.send(ToSessiond::PutCheckpoint(cp));
            }
            Out::Write(bytes) => self.write(session, bytes),
            Out::Nudge { cols, rows } => {
                for rows in [rows.saturating_sub(1).max(1), rows] {
                    self.next_req += 1;
                    let r = Resize {
                        session: session.clone(),
                        req: self.next_req,
                        cols,
                        rows,
                        px_w: 0,
                        px_h: 0,
                    };
                    self.send(ToSessiond::Resize(r));
                }
            }
            Out::Effect(fx, effect) => {
                debug!(
                    session = id,
                    rseq = fx.rseq,
                    index = fx.index,
                    ?effect,
                    "effect"
                );
                if matches!(effect, Effect::Bell) {
                    self.engine.streams.bell(id);
                }
                self.engine.journal().record(&fx, &effect);
                let _ = self.engine.events.send(Event::Effect(fx, effect));
            }
            Out::Ready(f) => {
                info!(session = id, fidelity = fidelity(f), "session live");
                let _ = self.engine.events.send(Event::Ready {
                    session,
                    fidelity: f,
                });
            }
            Out::Lost => warn!(session = id, "session lost"),
            Out::Closed(summary) => self.closed(summary),
            Out::Grid(HubOut::Send { conn, msg }) => self.engine.grid_send(conn, msg),
            Out::Grid(HubOut::Write { bytes, ack }) => {
                self.write(session, bytes);
                if let Some(ack) = ack {
                    self.engine.grid_conns().inputs.insert(self.input_seq, ack);
                }
            }
            Out::Applied(entries) => {
                let printed = entries.iter().any(|e| matches!(e.rec, Record::Data { .. }));
                if printed
                    && self
                        .engine
                        .journal()
                        .activity(id, std::time::Instant::now())
                {
                    let _ = self.engine.events.send(Event::Activity(session.clone()));
                }
                self.resized(id, &entries);
                self.engine.streams.applied(id, entries)
            }
            Out::Live(at) => {
                let actions = self.engine.streams.live(id, at);
                self.engine.perform(actions);
            }
            Out::Snapshot(token, snap) => {
                let snap = snap.as_deref().map(|s| Snap {
                    resume: s.resume,
                    cols: s.cols,
                    rows: s.rows,
                    vt: &s.vt,
                    title: &s.title,
                    cwd: &s.cwd,
                });
                self.engine.streams.snapshot_ready(token, snap);
            }
            Out::Output(token, lines) => self.engine.streams.output_ready(token, lines.as_deref()),
        }
    }

    /// The resizes among records the actor applied: who asked for each,
    /// noted for bytes clients' `terminal:resized` before the records go
    /// out, and sent to the session's grid clients as `Resized`.
    fn resized(&mut self, id: &str, entries: &[Entry]) {
        for e in entries {
            let Record::Resize { cols, rows, .. } = e.rec else {
                continue;
            };
            let decided = self.engine.sizes.applied(id, size_of((cols, rows)));
            let owner = decided.and_then(|d| d.owner);
            let reason = decided.map(|d| d.reason);
            self.engine.streams.resized_by(
                id,
                e.hdr.rseq,
                owner.map(Who::name),
                reason.map(vorn_size::Reason::as_str),
            );
            for peer in self.engine.sizes.grid_peers(id) {
                let msg = ServerMsg::Resized {
                    sid: peer.sid,
                    cols,
                    rows,
                    rseq: e.hdr.rseq,
                    rev: None,
                    owner: (owner == Some(Who::Grid(peer))).then_some(peer.sid),
                    reason: reason.map(wire_reason),
                };
                self.engine.grid_send(peer.conn, msg);
            }
        }
    }

    /// The resizes the size rule has ready, to sessiond.
    fn resize_due(&mut self) {
        for (session, d) in self.engine.sizes.poll(std::time::Instant::now()) {
            debug!(session, ?d, "resize");
            self.next_req += 1;
            self.send(ToSessiond::Resize(Resize {
                session,
                req: self.next_req,
                cols: d.size.cols,
                rows: d.size.rows,
                px_w: 0,
                px_h: 0,
            }));
        }
    }

    /// Session `id` left the engine. One whose program ended is released in
    /// sessiond: its exit has been delivered, and nothing more will come.
    /// One that was lost stays in sessiond, for the next connection to take
    /// on again.
    fn closed(&mut self, summary: Box<Summary>) {
        let b = &summary.brief;
        if b.exited.is_some() {
            info!(session = %b.session, "session ended; releasing it");
            self.engine.sizes.closed(&b.session);
            self.send(ToSessiond::Release(SessionRef {
                session: b.session.clone(),
            }));
            // After the release, which went out under sessiond's id.
            let held = self.engine.names().held(&b.session).to_owned();
            self.engine.names().forget(&held);
        } else {
            warn!(session = %b.session, reason = ?b.reason, "session lost; the next connection takes it on again");
        }
        self.engine.journal().closed(&b.session, b.exited.is_some());
        {
            let mut closed = self.engine.closed();
            if closed.len() == CLOSED_KEPT {
                closed.pop_front();
            }
            closed.push_back(b.clone());
        }
        self.engine
            .streams
            .closed(&b.session, b.exited, &summary.screen);
        let _ = self.engine.events.send(Event::Closed(Arc::from(summary)));
    }

    /// Starts the session's records flowing again after a refused fetch.
    /// sessiond ends a session's stream to vornd when it takes an attach,
    /// before it knows whether it can serve it; a refused one leaves none.
    /// A served fetch needs nothing: sessiond carries on live after it.
    fn repump(&mut self, session: String) {
        let from = match self.engine.streams.head(&session) {
            Some(c) => {
                self.repumping.insert(session.clone());
                AttachFrom::Cursor(c)
            }
            None => AttachFrom::NewestCheckpoint,
        };
        self.send(ToSessiond::Attach(Attach { session, from }));
    }

    /// Whether a session may be started under `name`: no session the
    /// engine runs, nor one waiting to be started, goes by it already.
    fn name_free(&self, name: &str) -> Result<(), String> {
        let running = |id: &str| self.pool.briefs().iter().any(|b| b.session == id);
        if running(name)
            || self
                .spawns
                .values()
                .any(|p| p.name.as_deref() == Some(name))
        {
            return Err(crate::names::Refused::Taken.to_string());
        }
        self.engine
            .names()
            .check(name, &running)
            .map_err(|e| e.to_string())
    }

    fn write(&mut self, session: String, bytes: Vec<u8>) {
        self.input_seq += 1;
        self.send(ToSessiond::Write(Write {
            session,
            input_seq: self.input_seq,
            bytes,
        }));
    }

    async fn command(&mut self, c: Command, outs: &mut mpsc::UnboundedReceiver<(String, Out)>) {
        match c {
            Command::Spawn(spec, name, reply) => {
                if let Some(name) = &name {
                    if let Err(e) = self.name_free(name) {
                        let _ = reply.send(Err(e));
                        return;
                    }
                }
                self.next_req += 1;
                let size = match spec.io {
                    Io::Pty { cols, rows } => Some((cols, rows)),
                    Io::Piped { .. } => None,
                };
                self.spawns
                    .insert(self.next_req, Pending { reply, size, name });
                self.send(ToSessiond::Spawn(Spawn {
                    req: self.next_req,
                    spec,
                }));
            }
            Command::Write(session, bytes) => self.write(session, bytes),
            Command::Signal(session, signal) => {
                self.send(ToSessiond::Signal(Signal { session, signal }));
            }
            Command::CloseStdin(session) => {
                self.send(ToSessiond::CloseStdin(SessionRef { session }));
            }
            Command::Resize(session, cols, rows) => {
                self.next_req += 1;
                self.send(ToSessiond::Resize(Resize {
                    session,
                    req: self.next_req,
                    cols,
                    rows,
                    px_w: 0,
                    px_h: 0,
                }));
            }
            Command::Fetch(session, from) => {
                // sessiond sends the records from `from` again, then carries
                // on live; the actor skips what it has. The ack keeps
                // sessiond's window open past the records sent again.
                let delivered = self.engine.streams.head(&session);
                self.send(ToSessiond::Attach(Attach {
                    session: session.clone(),
                    from: AttachFrom::Cursor(from),
                }));
                if let Some(delivered) = delivered {
                    self.send(ToSessiond::Ack(Ack { session, delivered }));
                }
            }
            Command::Flush(done) => {
                // The reader keeps draining sessiond meanwhile; what it
                // reads waits here until the checkpoints are queued.
                let pool = Arc::clone(&self.pool);
                let _ = tokio::task::spawn_blocking(move || pool.checkpoint_all()).await;
                while let Ok((id, o)) = outs.try_recv() {
                    self.asked(&id, o);
                }
                let _ = self.to_send.send(Queued::Written(done));
            }
        }
    }
}

impl From<&EffectId> for Stamp {
    fn from(id: &EffectId) -> Stamp {
        Stamp {
            epoch: u64::from(id.epoch),
            rseq: id.rseq,
            index: u64::from(id.index),
        }
    }
}

/// Tells the registry what each session does, for the statuses it decides,
/// and turns terminals idle as their timers run out. Runs as long as the
/// engine does.
async fn follow_statuses(engine: Arc<Engine>, mut events: broadcast::Receiver<Event>) {
    let registry = Arc::clone(engine.registry());
    loop {
        let wake = registry.next_idle();
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(Event::Effect(fx, Effect::Status(code))) => {
                    registry.screen_status(&fx.session, code, Stamp::from(&fx));
                }
                Ok(Event::Activity(session)) => {
                    let head = engine.head_stamp(&session);
                    registry.activity(&session, head, tokio::time::Instant::now());
                }
                Ok(Event::Closed(summary)) => registry.session_closed(&summary.brief.session),
                Ok(_) => {}
                // Fell behind: each session's latest screen status again,
                // which changes nothing the registry already took.
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(n, "the statuses fell behind the sessions");
                    let held = engine.journal().held();
                    for h in held {
                        if let Some(s) = h.status {
                            registry.screen_status(&h.session, s.value, Stamp::from(&s.id));
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            () = sleep_until(wake), if wake.is_some() => {
                registry.tick(tokio::time::Instant::now(), |id| engine.head_stamp(id));
            }
        }
    }
}

/// Sleeps until `at`; never resolves for `None`.
async fn sleep_until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::registry::{AgentStatus, HookStatus};

    fn agent(id: &str) -> Value {
        json!({
            "id": id, "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 1, "pid": 3,
        })
    }

    fn status_of(engine: &Engine, id: &str) -> Option<AgentStatus> {
        engine.registry().read(|r| {
            r.terminals()
                .into_iter()
                .find(|t| t.id == id)
                .map(|t| t.status)
        })?
    }

    /// Lets the follower take what was sent, on the paused clock.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_terminal_that_goes_quiet_turns_idle_on_the_paused_clock() {
        let engine = Engine::new(Config::default());
        engine
            .registry()
            .feed(
                1,
                &json!({ "op": "snapshot", "terminals": [agent("a")], "headless": [] }),
            )
            .unwrap();
        engine.decide_statuses();
        assert!(engine.registry().decides());

        let _ = engine.events.send(Event::Activity("a".into()));
        settle().await;
        tokio::time::advance(Duration::from_millis(4_900)).await;
        settle().await;
        assert_eq!(status_of(&engine, "a"), Some(AgentStatus::Running));
        tokio::time::advance(Duration::from_millis(200)).await;
        settle().await;
        assert_eq!(status_of(&engine, "a"), Some(AgentStatus::Idle));

        // Its screen says waiting: it is, at the effect's stamp.
        let fx = EffectId {
            session: "a".into(),
            epoch: 1,
            rseq: 7,
            index: 0,
        };
        let _ = engine.events.send(Event::Effect(fx, Effect::Status(2)));
        settle().await;
        assert_eq!(status_of(&engine, "a"), Some(AgentStatus::Waiting));

        // Promoted to hooks, its timer runs 30s.
        let hook = HookStatus {
            id: "a".into(),
            status: Some(AgentStatus::Running),
            promote: true,
        };
        engine
            .registry()
            .hook_status(&hook, None, tokio::time::Instant::now())
            .unwrap();
        let _ = engine.events.send(Event::Activity("a".into()));
        settle().await;
        tokio::time::advance(Duration::from_secs(29)).await;
        settle().await;
        assert_eq!(status_of(&engine, "a"), Some(AgentStatus::Running));
        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert_eq!(status_of(&engine, "a"), Some(AgentStatus::Idle));
    }
}
