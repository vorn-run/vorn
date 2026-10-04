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
//! No client sees the sessions yet. They are reported, without their
//! contents, at [`SESSIONS_PATH`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use vorn_engine::{Config, Fidelity, Input, Open, Out, Pool, Summary};
use vorn_sessiond_wire::{
    Ack, Attach, Io, Nonce, Resize, SessionRef, Spawn, SpawnSpec, ToSessiond, ToVornd, Welcome,
    Write,
};

use crate::holder::Conn;

/// The path the session report is served at.
pub const SESSIONS_PATH: &str = "/vornd/sessions";

const PING_EVERY: Duration = Duration::from_secs(15);

/// How long a clean stop waits for the last checkpoints to be sent.
const LAST_CHECKPOINTS: Duration = Duration::from_secs(5);

enum Command {
    Spawn(SpawnSpec, oneshot::Sender<Result<String, String>>),
    Write(String, Vec<u8>),
    CloseStdin(String),
    /// Cut a last checkpoint for every session and send them.
    Flush(oneshot::Sender<()>),
}

/// The connection the engine is running on, while there is one.
struct Current {
    pool: Arc<Pool>,
    commands: mpsc::UnboundedSender<Command>,
}

/// The session engine as vornd runs it, across connections.
pub struct Engine {
    cfg: Config,
    current: Mutex<Option<Current>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("cfg", &self.cfg)
            .field("connected", &self.current().is_some())
            .finish()
    }
}

impl Engine {
    pub fn new(cfg: Config) -> Arc<Engine> {
        Arc::new(Engine {
            cfg,
            current: Mutex::new(None),
        })
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<Current>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn command(&self, c: Command) -> Result<(), String> {
        self.current()
            .as_ref()
            .ok_or_else(|| "no session holder connected".to_owned())?
            .commands
            .send(c)
            .map_err(|_| "the session holder connection closed".to_owned())
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

    /// Every session as the engine has it, contents included. For tests and
    /// in-process callers only: [`Engine::report`] is what goes over HTTP.
    pub async fn sessions(&self) -> Vec<Summary> {
        let Some(pool) = self.current().as_ref().map(|c| Arc::clone(&c.pool)) else {
            return Vec::new();
        };
        tokio::task::spawn_blocking(move || pool.sessions())
            .await
            .unwrap_or_default()
    }

    /// The debug report: where each session is and how it was recovered.
    /// Never the screen, title or cwd: the endpoint answers anyone on this
    /// machine.
    pub async fn report(&self) -> Value {
        let sessions: Vec<Value> = self
            .sessions()
            .await
            .iter()
            .map(|s| {
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
            })
            .collect();
        json!({ "connected": self.current().is_some(), "sessions": sessions })
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
    pub(crate) async fn run(&self, conn: &mut Conn, welcome: Welcome) -> String {
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
        let mut d = Driver {
            pool: Arc::clone(&pool),
            spawns: HashMap::new(),
            next_req: 0,
            input_seq: 0,
        };
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        let mut nonce = 0u64;
        let why = loop {
            tokio::select! {
                msg = conn.recv() => match msg {
                    Ok(m) => d.received(m),
                    Err(e) => break e.to_string(),
                },
                Some((id, o)) = outs.recv() => {
                    if let Err(e) = d.asked(conn, &id, o).await {
                        break e.to_string();
                    }
                }
                Some(c) = commands.recv() => {
                    if let Err(e) = d.command(conn, c, &mut outs).await {
                        break e.to_string();
                    }
                }
                _ = ping.tick() => {
                    nonce += 1;
                    if let Err(e) = conn.send(&ToSessiond::Ping(Nonce { nonce })).await {
                        break e.to_string();
                    }
                }
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

struct Clear<'a>(&'a Engine);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        *self.0.current() = None;
    }
}

fn fidelity(f: Fidelity) -> &'static str {
    match f {
        Fidelity::Exact => "exact",
        Fidelity::Approximate => "approximate",
    }
}

/// A spawn sent and not answered yet.
struct Pending {
    reply: oneshot::Sender<Result<String, String>>,
    /// The PTY size, or `None` for a piped agent.
    size: Option<(u16, u16)>,
}

struct Driver {
    pool: Arc<Pool>,
    /// Spawns sent and not answered: who asked, and the PTY size.
    spawns: HashMap<u64, Pending>,
    next_req: u64,
    /// Numbers this connection's writes.
    input_seq: u64,
}

impl Driver {
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

    /// What session `id` asked for.
    async fn asked(&mut self, conn: &mut Conn, id: &str, o: Out) -> std::io::Result<()> {
        let session = id.to_owned();
        match o {
            Out::Attach(from) => {
                conn.send(&ToSessiond::Attach(Attach { session, from }))
                    .await
            }
            Out::Ack(delivered) => {
                conn.send(&ToSessiond::Ack(Ack { session, delivered }))
                    .await
            }
            Out::Checkpoint(cp) => {
                debug!(
                    session = id,
                    rseq = cp.resume.next_rseq,
                    bytes = cp.blob.len(),
                    "checkpoint"
                );
                conn.send(&ToSessiond::PutCheckpoint(cp)).await
            }
            Out::Write(bytes) => self.write(conn, session, bytes).await,
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
                    conn.send(&ToSessiond::Resize(r)).await?;
                }
                Ok(())
            }
            Out::Effect(fx, effect) => {
                debug!(
                    session = id,
                    rseq = fx.rseq,
                    index = fx.index,
                    ?effect,
                    "effect"
                );
                Ok(())
            }
            Out::Ready(f) => {
                info!(session = id, fidelity = fidelity(f), "session live");
                Ok(())
            }
            Out::Lost => {
                warn!(session = id, "session lost");
                Ok(())
            }
        }
    }

    async fn write(
        &mut self,
        conn: &mut Conn,
        session: String,
        bytes: Vec<u8>,
    ) -> std::io::Result<()> {
        self.input_seq += 1;
        let w = Write {
            session,
            input_seq: self.input_seq,
            bytes,
        };
        conn.send(&ToSessiond::Write(w)).await
    }

    async fn command(
        &mut self,
        conn: &mut Conn,
        c: Command,
        outs: &mut mpsc::UnboundedReceiver<(String, Out)>,
    ) -> std::io::Result<()> {
        match c {
            Command::Spawn(spec, reply) => {
                self.next_req += 1;
                let size = match spec.io {
                    Io::Pty { cols, rows } => Some((cols, rows)),
                    Io::Piped { .. } => None,
                };
                self.spawns.insert(self.next_req, Pending { reply, size });
                conn.send(&ToSessiond::Spawn(Spawn {
                    req: self.next_req,
                    spec,
                }))
                .await
            }
            Command::Write(session, bytes) => self.write(conn, session, bytes).await,
            Command::CloseStdin(session) => {
                conn.send(&ToSessiond::CloseStdin(SessionRef { session }))
                    .await
            }
            Command::Flush(done) => {
                // Nothing else reaches sessiond until the checkpoints have.
                let pool = Arc::clone(&self.pool);
                let _ = tokio::task::spawn_blocking(move || pool.checkpoint_all()).await;
                while let Ok((id, o)) = outs.try_recv() {
                    self.asked(conn, &id, o).await?;
                }
                let _ = done.send(());
                Ok(())
            }
        }
    }
}
