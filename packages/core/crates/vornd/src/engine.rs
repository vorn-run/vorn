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
//! No client sees the sessions yet; [`Engine::subscribe`] carries what
//! happens to them, and they are reported, without their contents, at
//! [`SESSIONS_PATH`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use vorn_engine::{Brief, Config, Effect, EffectId, Fidelity, Input, Open, Out, Pool, Summary};
use vorn_sessiond_wire::{
    Ack, Attach, Io, Nonce, Resize, SessionRef, Spawn, SpawnSpec, ToSessiond, ToVornd, Welcome,
    Write,
};

use crate::holder::{Conn, Writer};

/// The path the session report is served at.
pub const SESSIONS_PATH: &str = "/vornd/sessions";

/// How long one write to sessiond may take before the connection is given
/// up. A local socket to a sessiond that reads takes milliseconds for the
/// biggest checkpoint.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(20);

const PING_EVERY: Duration = Duration::from_secs(15);

/// How long a clean stop waits for the last checkpoints to be sent.
const LAST_CHECKPOINTS: Duration = Duration::from_secs(5);

/// Sessions that left the engine, kept for the report.
const CLOSED_KEPT: usize = 64;

/// Events kept for a subscriber that falls behind.
const EVENTS: usize = 1024;

enum Command {
    Spawn(SpawnSpec, oneshot::Sender<Result<String, String>>),
    Write(String, Vec<u8>),
    CloseStdin(String),
    /// Cut a last checkpoint for every session and send them.
    Flush(oneshot::Sender<()>),
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
        Arc::new(Engine {
            cfg,
            write_timeout,
            current: Mutex::new(None),
            closed: Mutex::new(VecDeque::new()),
            events: broadcast::channel(EVENTS).0,
        })
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<Current>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn closed(&self) -> std::sync::MutexGuard<'_, VecDeque<Brief>> {
        self.closed.lock().unwrap_or_else(|e| e.into_inner())
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
        *self.current() = Some(Current {
            pool: Arc::clone(&pool),
            commands: cmd_tx,
        });
        // However this ends, a dropped future included, the sessions go
        // with the connection.
        let _clear = Clear(self);
        for info in &welcome.sessions {
            pool.open(&info.session, Open::from_info(info));
        }
        info!(sessions = welcome.sessions.len(), "recovering sessions");
        // The pool moves into the driver so that `_clear` drops the last
        // reference, off the runtime's threads.
        let mut d = Driver {
            engine: self,
            pool,
            to_send,
            spawns: HashMap::new(),
            next_req: 0,
            input_seq: 0,
        };
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
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
                r = &mut writing => break r.unwrap_or_else(|e| e.to_string()),
            }
        };
        for (_, p) in d.spawns.drain() {
            let _ = p.reply.send(Err(why.clone()));
        }
        // The pool stops with the last reference, without last checkpoints:
        // the sessions are sessiond's, and the next connection recovers them.
        why
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
}

impl Driver<'_> {
    fn send(&self, m: ToSessiond) {
        let _ = self.to_send.send(Queued::Message(m));
    }

    /// A message from sessiond, for the session it names.
    fn received(&mut self, m: ToVornd) {
        match m {
            ToVornd::Entries(e) => self.pool.input(&e.session, Input::Entries(e.entries)),
            ToVornd::CheckpointIs(cp) => {
                let id = cp.session.clone();
                self.pool.input(&id, Input::Checkpoint(cp));
            }
            ToVornd::Refused(r) => self.pool.input(&r.session, Input::Refused(r.why)),
            ToVornd::Spawned(s) => {
                if let Some(p) = self.spawns.remove(&s.req) {
                    self.pool.open(&s.session, Open::spawned(s.start, p.size));
                    let _ = p.reply.send(Ok(s.session));
                }
            }
            ToVornd::Failed(f) => {
                if let Some(p) = self.spawns.remove(&f.req) {
                    let _ = p.reply.send(Err(f.error));
                }
            }
            ToVornd::InputDone(_) | ToVornd::Pong(_) | ToVornd::Welcome(_) => {}
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
        let _ = self.engine.events.send(Event::Closed(Arc::from(summary)));
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
            Command::Write(session, bytes) => self.write(session, bytes),
            Command::CloseStdin(session) => {
                self.send(ToSessiond::CloseStdin(SessionRef { session }));
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
