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
    Ack, Attach, AttachFrom, Io, Kind, Nonce, Resize, SessionRef, Sig, Signal, Spawn, SpawnAs,
    SpawnSpec, ToSessiond, ToVornd, Welcome, Write,
};
use vorn_term_proto::msg::{ResizeReason, ServerMsg};
use vorn_term_proto::{Cursor, Entry, Record};

use crate::holder::{Conn, Writer};
use crate::size::{Sizes, Who};
use crate::streams::{Action, Snap, Streams};

/// The path the session report is served at.
pub const SESSIONS_PATH: &str = "/vornd/sessions";

/// How long one write to sessiond may take before the connection is given
/// up. A local socket to a sessiond that reads takes milliseconds for the
/// biggest checkpoint.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(20);

const PING_EVERY: Duration = Duration::from_secs(15);

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
    Spawn(SpawnSpec, oneshot::Sender<Result<String, String>>),
    /// A spawn under the app's own name, in a fresh epoch.
    SpawnAs(
        String,
        u32,
        SpawnSpec,
        oneshot::Sender<Result<String, String>>,
    ),
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
}

/// What the engine hands its tap ([`Engine::tap`]), in the order the
/// sessions' actors produced it: each session's effects come before the
/// records of the batch that caused them.
#[derive(Debug, Clone)]
pub enum Tapped {
    /// Records of a session, once applied to its terminal.
    Records(String, Vec<Entry>),
    Effect(EffectId, Effect),
    /// The engine connected to a sessiond and took on what it holds: a
    /// session the consumer had that is not there now has gone with an
    /// earlier sessiond (RC §6 flow E), and only [`Engine::held`] can say.
    Held,
}

/// A session sessiond holds, as the engine learned of it: from a Welcome
/// or from its own spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub kind: Kind,
    pub pid: u32,
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
    /// Every session the current sessiond holds, by id.
    held: Mutex<HashMap<String, Held>>,
    /// Where applied records and effects also go, when someone asked.
    tap: Mutex<Option<mpsc::UnboundedSender<Tapped>>>,
    sizes: Arc<Sizes>,
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
        Arc::new(Engine {
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
            held: Mutex::new(HashMap::new()),
            tap: Mutex::new(None),
        })
    }

    /// Every session's size rule.
    pub fn sizes(&self) -> &Arc<Sizes> {
        &self.sizes
    }

    /// Every applied batch of records and every effect from now on, in
    /// order and none dropped, for one consumer (the Node link,
    /// [`crate::node_link`]). A second call replaces the first consumer.
    /// Unbounded on purpose: what the actors produce is already bounded by
    /// what sessiond sends, and a consumer that cannot keep up drops its
    /// own backlog rather than slow every session.
    pub fn tap(&self) -> mpsc::UnboundedReceiver<Tapped> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.tap.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        rx
    }

    fn tapped(&self, t: Tapped) {
        let mut tap = self.tap.lock().unwrap_or_else(|e| e.into_inner());
        if tap.as_ref().is_some_and(|tx| tx.send(t).is_err()) {
            *tap = None;
        }
    }

    fn tapping(&self) -> bool {
        self.tap.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Whether the engine is connected to a sessiond now.
    pub fn connected(&self) -> bool {
        self.current().is_some()
    }

    /// The sessions the current sessiond holds, by id.
    pub fn held(&self) -> HashMap<String, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn held_mut(&self) -> std::sync::MutexGuard<'_, HashMap<String, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a session named `name` in sessiond, in `epoch`, and runs it
    /// through the engine. Refused while a session of that name is still
    /// held: an ended one is released once its last record is applied.
    pub async fn spawn_as(&self, name: &str, epoch: u32, spec: SpawnSpec) -> Result<Held, String> {
        if self.held_mut().contains_key(name) {
            return Err(format!("session {name} is still held"));
        }
        let (tx, rx) = oneshot::channel();
        self.command(Command::SpawnAs(name.to_owned(), epoch, spec, tx))?;
        let id = rx
            .await
            .map_err(|_| "the session holder connection closed".to_owned())??;
        self.held_mut()
            .get(&id)
            .copied()
            .ok_or_else(|| format!("session {id} ended as it started"))
    }

    /// Sends `sig` to a session's program. Its exit comes back as records
    /// and an [`Effect::Exit`].
    pub fn signal(&self, session: &str, sig: Sig) -> Result<(), String> {
        self.command(Command::Signal(session.to_owned(), sig))
    }

    /// The terminal streams of the sessions this engine holds.
    pub fn streams(&self) -> &Arc<Streams> {
        &self.streams
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
        let (tx, rx) = oneshot::channel();
        self.command(Command::Spawn(spec, tx))?;
        rx.await
            .map_err(|_| "the session holder connection closed".to_owned())?
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
        let mut writing = tokio::spawn(write_queued(writer, queued, self.write_timeout));
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
        *self.held_mut() = welcome
            .sessions
            .iter()
            .map(|i| {
                let held = Held {
                    kind: i.kind,
                    pid: i.pid,
                    epoch: i.epoch,
                };
                (i.session.clone(), held)
            })
            .collect();
        for info in &welcome.sessions {
            self.streams.opened(&info.session, info.epoch);
            let open = Open::from_info(info);
            if open.pty {
                self.sizes.opened(&info.session, size_of(open.size));
            }
            pool.open(&info.session, open);
        }
        info!(sessions = welcome.sessions.len(), "recovering sessions");
        if self.tapping() {
            self.tapped(Tapped::Held);
        }
        // The pool moves into the driver so that `_clear` drops the last
        // reference, off the runtime's threads.
        let mut d = Driver {
            engine: self,
            pool,
            to_send,
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
    Message(ToSessiond),
    /// Answered once everything queued before it is written.
    Written(oneshot::Sender<()>),
}

/// Writes what is queued, in order, until the connection fails or a write
/// takes `timeout`; answers why it stopped.
async fn write_queued(
    mut w: Writer,
    mut queued: mpsc::UnboundedReceiver<Queued>,
    timeout: Duration,
) -> String {
    while let Some(q) = queued.recv().await {
        match q {
            Queued::Message(m) => match tokio::time::timeout(timeout, w.send(&m)).await {
                Ok(Ok(())) => {}
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
        // Until the next Welcome, nothing is known to be held: a sessiond
        // that died took its sessions with it.
        self.0.held_mut().clear();
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
    reply: oneshot::Sender<Result<String, String>>,
    /// The PTY size, or `None` for a piped agent.
    size: Option<(u16, u16)>,
}

struct Driver<'a> {
    engine: &'a Engine,
    pool: Arc<Pool>,
    /// The writer's queue. Sending never waits; a writer that has stopped
    /// ends the connection, which the driver learns from its task.
    to_send: mpsc::UnboundedSender<Queued>,
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
    fn send(&self, m: ToSessiond) {
        let _ = self.to_send.send(Queued::Message(m));
    }

    /// A message from sessiond, for the session it names.
    fn received(&mut self, m: ToVornd) {
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
                    let kind = if p.size.is_some() {
                        Kind::Pty
                    } else {
                        Kind::Piped
                    };
                    let held = Held {
                        kind,
                        pid: s.pid,
                        epoch: s.start.epoch,
                    };
                    self.engine.held_mut().insert(s.session.clone(), held);
                    self.engine.streams.opened(&s.session, s.start.epoch);
                    if let Some(size) = p.size {
                        self.engine.sizes.opened(&s.session, size_of(size));
                    }
                    self.pool.open(&s.session, Open::spawned(s.start, p.size));
                    let _ = p.reply.send(Ok(s.session));
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
                if self.engine.tapping() {
                    self.engine
                        .tapped(Tapped::Effect(fx.clone(), effect.clone()));
                }
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
                self.resized(id, &entries);
                if self.engine.tapping() {
                    self.engine
                        .tapped(Tapped::Records(id.to_owned(), entries.clone()));
                }
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
            self.engine.held_mut().remove(&b.session);
            self.engine.sizes.closed(&b.session);
            self.send(ToSessiond::Release(SessionRef {
                session: b.session.clone(),
            }));
        } else {
            warn!(session = %b.session, reason = ?b.reason, "session lost; the next connection takes it on again");
        }
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
            Command::Spawn(spec, reply) => {
                self.next_req += 1;
                let size = match spec.io {
                    Io::Pty { cols, rows } => Some((cols, rows)),
                    Io::Piped { .. } => None,
                };
                self.spawns.insert(self.next_req, Pending { reply, size });
                self.send(ToSessiond::Spawn(Spawn {
                    req: self.next_req,
                    spec,
                }));
            }
            Command::SpawnAs(session, epoch, spec, reply) => {
                self.next_req += 1;
                let size = match spec.io {
                    Io::Pty { cols, rows } => Some((cols, rows)),
                    Io::Piped { .. } => None,
                };
                self.spawns.insert(self.next_req, Pending { reply, size });
                self.send(ToSessiond::SpawnAs(SpawnAs {
                    req: self.next_req,
                    session,
                    epoch,
                    spec,
                }));
            }
            Command::Signal(session, signal) => {
                self.send(ToSessiond::Signal(Signal { session, signal }));
            }
            Command::Write(session, bytes) => self.write(session, bytes),
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
