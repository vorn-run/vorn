//! sessiond's endpoint (RC §5): one vornd connection at a time over a
//! user-only local socket, serving sessions to it as records.
//!
//! A second Hello replaces the first connection, which covers a vornd that
//! hung rather than died. The replaced connection closes at once, its pumps
//! with it, without waiting for the hung peer to send anything.
//!
//! Each attached session has a pump that sends new records as they are
//! appended, up to 4 MiB past what vornd acked; past that the session keeps
//! reading into its log and the pump waits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch, Notify};
use tokio::task::JoinHandle;
use vorn_term_proto::{Cursor, Entry, Record};

use crate::log::SpoolPool;
use crate::session::Session;
use crate::wire::*;

#[cfg(unix)]
mod handoff;
#[cfg(unix)]
pub use handoff::{Fault, Step};

/// How far the pump may run ahead of vornd's acks.
pub const WINDOW_BYTES: u64 = 4 << 20;
/// Data per Entries frame, well under the frame cap.
const BATCH_BYTES: u64 = 1 << 20;

pub struct Config {
    /// `$VORN_HOME`: the socket goes in `run/`, spools in `spool/`.
    pub home: PathBuf,
    pub instance: u128,
    pub build: String,
    /// Exit after this long with no sessions and no vornd.
    pub idle_exit: Duration,
    pub spool_cap: u64,
}

pub struct Sessiond {
    cfg: Config,
    pool: SpoolPool,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next_id: AtomicU64,
    /// Raised by every Hello; a connection whose generation is not current
    /// closes. A watch, so a replaced connection hears of it while it waits
    /// on a peer that never sends or never reads.
    generation: watch::Sender<u64>,
    /// The current connection's outbox, for replies that come from session threads.
    outbox: Mutex<Option<mpsc::Sender<ToVornd>>>,
    idle_since: Mutex<Option<Instant>>,
    /// Set by Drain: no new sessions, and exit once the last one is released.
    draining: AtomicBool,
    /// Set by [`Sessiond::shut_down`], so [`serve`] ends even if it was
    /// between waits when `stop` was notified.
    stopping: AtomicBool,
    pub stop: Notify,
    /// Set while the sessions are being handed to a newer sessiond: no new
    /// sessions, and none released, until it is done or called off.
    #[cfg(unix)]
    handing: AtomicBool,
    /// Set once a newer sessiond took the sessions: this one only reaps.
    #[cfg(unix)]
    handed: AtomicBool,
    /// Sessions handed off whose programs this process still has to reap.
    #[cfg(unix)]
    reaping: Mutex<Vec<Arc<Session>>>,
    #[cfg(unix)]
    adopting: AtomicBool,
    #[cfg(unix)]
    fault: Mutex<Option<Fault>>,
}

impl Sessiond {
    pub fn new(cfg: Config) -> Arc<Self> {
        let pool = SpoolPool::new(cfg.spool_cap);
        Arc::new(Sessiond {
            cfg,
            pool,
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            generation: watch::Sender::new(0),
            outbox: Mutex::new(None),
            idle_since: Mutex::new(Some(Instant::now())),
            draining: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            stop: Notify::new(),
            #[cfg(unix)]
            handing: AtomicBool::new(false),
            #[cfg(unix)]
            handed: AtomicBool::new(false),
            #[cfg(unix)]
            reaping: Mutex::new(Vec::new()),
            #[cfg(unix)]
            adopting: AtomicBool::new(false),
            #[cfg(unix)]
            fault: Mutex::new(None),
        })
    }

    /// Make the handoff fail or hang at one step, for tests.
    #[cfg(unix)]
    pub fn inject(&self, fault: Fault) {
        *self.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some(fault);
    }

    /// Whether a newer sessiond took this one's sessions.
    #[cfg(unix)]
    pub fn handed_off(&self) -> bool {
        self.handed.load(Ordering::SeqCst)
    }

    /// Whether this one takes new sessions.
    fn closed_to_new(&self) -> bool {
        #[cfg(unix)]
        if self.handing.load(Ordering::SeqCst) || self.handed.load(Ordering::SeqCst) {
            return true;
        }
        self.draining.load(Ordering::SeqCst)
    }

    /// Ends [`serve`], which then removes the endpoint and the announcement:
    /// an orderly exit, as on SIGTERM, leaves nothing behind in `run/`.
    pub fn shut_down(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
    }

    /// Where vornd connects: a Unix socket path or a named pipe name.
    pub fn endpoint(&self) -> String {
        endpoint(&self.cfg.home, self.cfg.instance)
    }

    fn spool_dir(&self) -> PathBuf {
        self.cfg.home.join("spool")
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn session(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions().get(id).cloned()
    }

    /// Whether session `id` is still held: until it is released.
    pub fn holds(&self, id: &str) -> bool {
        self.sessions().contains_key(id)
    }

    fn send_async(&self, msg: ToVornd) {
        if let Some(tx) = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.try_send(msg);
        }
    }

    fn infos(&self) -> Vec<SessionInfo> {
        self.sessions()
            .values()
            .map(|s| {
                s.with_log(|l| {
                    l.settle_gap();
                    let (cols, rows) = l.size();
                    SessionInfo {
                        session: s.id.clone(),
                        kind: s.kind,
                        pid: s.pid,
                        epoch: l.epoch(),
                        oldest: l.oldest(),
                        head: l.head(),
                        newest_cp: l.newest_cp(),
                        retain_from: l.retain_from(),
                        sent: l.sent(),
                        cols,
                        rows,
                        exited: l.exited(),
                        spooled_bytes: l.spooled_bytes(),
                    }
                })
            })
            .collect()
    }

    /// Serve one connection until it closes or a newer one replaces it.
    pub async fn serve_conn<S>(self: Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        self.serve_read(stream, &[]).await;
    }

    /// Serve a connection whose first bytes, `read`, were read already.
    async fn serve_read<S>(self: Arc<Self>, stream: S, read: &[u8])
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        *self.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let (mut rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<ToVornd>(256);
        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if wr.write_all(&msg.encode()).await.is_err() {
                    break;
                }
            }
        });
        let mut conn = Conn {
            d: Arc::clone(&self),
            tx,
            generation: None,
            pumps: HashMap::new(),
        };
        let mut frames = FrameReader::default();
        frames.push(read);
        let mut buf = vec![0u8; 64 << 10];
        let mut generation = self.generation.subscribe();
        'read: loop {
            let n = tokio::select! {
                r = rd.read(&mut buf) => match r { Ok(0) | Err(_) => break, Ok(n) => n },
                _ = self.stop.notified() => break,
                _ = replaced(&mut generation, conn.generation) => break,
            };
            frames.push(&buf[..n]);
            loop {
                match frames.read::<ToSessiond>() {
                    Ok(Some(msg)) => {
                        // Handling may wait on a peer that stopped reading;
                        // a newer Hello ends that wait too.
                        let mine = conn.generation;
                        let open = tokio::select! {
                            open = conn.handle(msg) => open,
                            _ = replaced(&mut generation, mine) => false,
                        };
                        if !open {
                            break 'read;
                        }
                    }
                    Ok(None) => break,
                    // A frame this build cannot read closes the connection.
                    Err(_) => break 'read,
                }
            }
            if conn.replaced() {
                break;
            }
        }
        conn.close();
        // The current vornd left: nobody to reply to, and the idle clock starts.
        let current = conn.generation.is_some() && !conn.replaced();
        let anyone = self
            .outbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if current || !anyone {
            *self.outbox.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *self.idle_since.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        }
        drop(conn);
        writer.abort();
    }

    /// Whether sessiond has nothing to hold and nobody to serve, for long
    /// enough to exit.
    pub fn idle_for(&self, d: Duration) -> bool {
        if !self.sessions().is_empty() {
            return false;
        }
        self.draining.load(Ordering::SeqCst)
            || self
                .idle_since
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|t| t.elapsed() >= d)
    }

    pub fn idle_exit(&self) -> Duration {
        self.cfg.idle_exit
    }
}

struct Pump {
    acked: Arc<AtomicU64>,
    acked_note: Arc<Notify>,
    task: JoinHandle<()>,
}

struct Conn {
    d: Arc<Sessiond>,
    tx: mpsc::Sender<ToVornd>,
    generation: Option<u64>,
    pumps: HashMap<String, Pump>,
}

impl Conn {
    fn replaced(&self) -> bool {
        self.generation
            .is_some_and(|g| g != *self.d.generation.borrow())
    }

    fn close(&mut self) {
        for (_, p) in self.pumps.drain() {
            p.task.abort();
        }
    }

    async fn send(&self, msg: ToVornd) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Handle one message; false closes the connection.
    async fn handle(&mut self, msg: ToSessiond) -> bool {
        if self.generation.is_none() {
            let ToSessiond::Hello(h) = msg else {
                return false;
            };
            if h.proto_min > PROTO || h.proto_max < PROTO {
                return false;
            }
            #[cfg(unix)]
            if self.d.handed_off() {
                return false;
            }
            let mut g = 0;
            self.d.generation.send_modify(|n| {
                *n += 1;
                g = *n;
            });
            self.generation = Some(g);
            *self.d.outbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.tx.clone());
            let welcome = Welcome {
                proto: PROTO,
                sessiond_instance: self.d.cfg.instance,
                sessiond_build: self.d.cfg.build.clone(),
                sessions: self.d.infos(),
            };
            return self.send(ToVornd::Welcome(welcome)).await;
        }
        if self.replaced() {
            return false;
        }
        match msg {
            ToSessiond::Hello(_) => return false,
            ToSessiond::Attach(a) => return self.attach(a).await,
            ToSessiond::Spawn(sp) if self.d.closed_to_new() => {
                let reply = ToVornd::Failed(Failed {
                    req: sp.req,
                    error: "draining: start new sessions on the newer sessiond".into(),
                });
                return self.send(reply).await;
            }
            ToSessiond::Handoff(_) => return false,
            ToSessiond::Adopt(a) => {
                let reply = self.adopt(a).await;
                return self.send(reply).await;
            }
            ToSessiond::Drain(_) => self.d.draining.store(true, Ordering::SeqCst),
            ToSessiond::Spawn(sp) => {
                let reply = self.spawn(sp);
                return self.send(reply).await;
            }
            ToSessiond::Write(w) => {
                if let Some(s) = self.d.session(&w.session) {
                    s.write(w.input_seq, w.bytes);
                }
            }
            ToSessiond::CloseStdin(r) => {
                if let Some(s) = self.d.session(&r.session) {
                    s.close_stdin();
                }
            }
            ToSessiond::Resize(r) => {
                if let Some(s) = self.d.session(&r.session) {
                    s.resize(r.req, r.cols, r.rows, r.px_w, r.px_h);
                }
            }
            ToSessiond::Signal(sig) => {
                if let Some(s) = self.d.session(&sig.session) {
                    s.signal(sig.signal);
                }
            }
            ToSessiond::Ack(a) => {
                if let Some(s) = self.d.session(&a.session) {
                    s.with_log(|l| l.ack(a.delivered));
                }
                if let Some(p) = self.pumps.get(&a.session) {
                    p.acked.fetch_max(a.delivered.next_offset, Ordering::SeqCst);
                    p.acked_note.notify_waiters();
                }
            }
            ToSessiond::PutCheckpoint(cp) => {
                if let Some(s) = self.d.session(&cp.session) {
                    s.put_checkpoint(cp);
                }
            }
            // A session handed over mid-release goes on to be released there.
            #[cfg(unix)]
            ToSessiond::Release(_) if self.d.handing.load(Ordering::SeqCst) => {}
            ToSessiond::Release(r) => {
                let mut map = self.d.sessions();
                if map
                    .get(&r.session)
                    .is_some_and(|s| s.with_log(|l| l.exited().is_some()))
                {
                    map.remove(&r.session);
                    drop(map);
                    if let Some(p) = self.pumps.remove(&r.session) {
                        p.task.abort();
                    }
                }
            }
            ToSessiond::Ping(n) => return self.send(ToVornd::Pong(n)).await,
        }
        true
    }

    /// Reports each write the kernel took to whichever vornd is connected.
    fn on_written(&self) -> impl Fn(&str, crate::session::Written) + Send + Clone + 'static {
        let d = Arc::downgrade(&self.d);
        move |session: &str, w: crate::session::Written| {
            if let Some(d) = d.upgrade() {
                d.send_async(ToVornd::InputDone(InputDone {
                    session: session.to_owned(),
                    input_seq: w.input_seq,
                    written: w.written,
                }));
            }
        }
    }

    /// Take every session of the older sessiond at `a.from`, or none.
    async fn adopt(&self, a: Adopt) -> ToVornd {
        #[cfg(unix)]
        {
            let failed = |error: String| ToVornd::Failed(Failed { req: a.req, error });
            if self.d.adopting.swap(true, Ordering::SeqCst) {
                return failed("already adopting".into());
            }
            let d = Arc::clone(&self.d);
            let on_written = self.on_written();
            let from = a.from.clone();
            let res = tokio::task::spawn_blocking(move || d.adopt(&from, on_written)).await;
            self.d.adopting.store(false, Ordering::SeqCst);
            match res {
                Ok(Ok(sessions)) => ToVornd::Adopted(Adopted {
                    req: a.req,
                    sessions,
                }),
                Ok(Err(e)) => failed(e.to_string()),
                Err(e) => failed(e.to_string()),
            }
        }
        #[cfg(windows)]
        ToVornd::Failed(Failed {
            req: a.req,
            error: "sessions cannot move between processes here".into(),
        })
    }

    fn spawn(&self, sp: Spawn) -> ToVornd {
        let n = self.d.next_id.fetch_add(1, Ordering::SeqCst);
        let id = format!("{:08x}-{n}", self.d.cfg.instance as u32);
        let on_written = self.on_written();
        if let Err(e) = std::fs::create_dir_all(self.d.spool_dir()) {
            return ToVornd::Failed(Failed {
                req: sp.req,
                error: e.to_string(),
            });
        }
        match Session::spawn(
            id.clone(),
            &sp.spec,
            &self.d.spool_dir(),
            self.d.pool.clone(),
            on_written,
        ) {
            Ok(s) => {
                let pid = s.pid;
                let mut map = self.d.sessions();
                // A handoff that began meanwhile would not take it along.
                if self.d.closed_to_new() {
                    drop(map);
                    return ToVornd::Failed(Failed {
                        req: sp.req,
                        error: "draining: start new sessions on the newer sessiond".into(),
                    });
                }
                map.insert(id.clone(), s);
                ToVornd::Spawned(Spawned {
                    req: sp.req,
                    session: id,
                    pid,
                    start: Cursor::start(0),
                })
            }
            Err(e) => ToVornd::Failed(Failed {
                req: sp.req,
                error: e.to_string(),
            }),
        }
    }

    async fn attach(&mut self, a: Attach) -> bool {
        let Some(s) = self.d.session(&a.session) else {
            return self
                .send(ToVornd::Refused(Refused {
                    session: a.session,
                    why: AttachRefusal::NoSuchSession,
                }))
                .await;
        };
        if let Some(old) = self.pumps.remove(&a.session) {
            old.task.abort();
        }
        let start = match a.from {
            AttachFrom::Cursor(c) => c,
            _ => Cursor::start(0),
        };
        let res = s.with_log(|l| l.attach(a.from));
        let (cp, entries) = match res {
            Ok(v) => v,
            Err(why) => {
                return self
                    .send(ToVornd::Refused(Refused {
                        session: a.session,
                        why,
                    }))
                    .await
            }
        };
        let from = cp.as_ref().map_or(start, |c| c.resume);
        if let Some(cp) = cp {
            if !self.send(ToVornd::CheckpointIs(cp)).await {
                return false;
            }
        }
        let mut next = from;
        for batch in batches(entries) {
            next = batch.last().expect("non-empty").after();
            if !self.send(entries_msg(&s.id, batch)).await || self.replaced() {
                return false;
            }
            s.with_log(|l| l.mark_sent(next));
        }
        let Some(mine) = self.generation else {
            return false;
        };
        let acked = Arc::new(AtomicU64::new(from.next_offset));
        let acked_note = Arc::new(Notify::new());
        let task = tokio::spawn(pump(
            Arc::clone(&s),
            next,
            Arc::clone(&acked),
            Arc::clone(&acked_note),
            self.tx.clone(),
            (self.d.generation.subscribe(), mine),
        ));
        self.pumps.insert(
            a.session,
            Pump {
                acked,
                acked_note,
                task,
            },
        );
        true
    }
}

fn entries_msg(session: &str, entries: Vec<Entry>) -> ToVornd {
    ToVornd::Entries(Entries {
        session: session.to_owned(),
        entries,
    })
}

/// Split a run of records into frames of about [`BATCH_BYTES`].
fn batches(entries: Vec<Entry>) -> Vec<Vec<Entry>> {
    let mut out: Vec<Vec<Entry>> = Vec::new();
    let mut bytes = 0;
    for e in entries {
        if out.is_empty() || bytes + e.rec.len() > BATCH_BYTES {
            out.push(Vec::new());
            bytes = 0;
        }
        bytes += e.rec.len();
        out.last_mut().expect("pushed").push(e);
    }
    out
}

/// Resolves once a newer Hello replaced the connection that said Hello as
/// `mine`; never for a connection that has not said Hello yet.
async fn replaced(generation: &mut watch::Receiver<u64>, mine: Option<u64>) {
    match mine {
        // The sender lives as long as sessiond, so this ends only on a change.
        Some(g) => {
            let _ = generation.wait_for(|&now| now != g).await;
        }
        None => std::future::pending().await,
    }
}

/// Send a session's new records as they arrive, within the window, until
/// the connection that owns this pump is replaced.
async fn pump(
    s: Arc<Session>,
    mut next: Cursor,
    acked: Arc<AtomicU64>,
    acked_note: Arc<Notify>,
    tx: mpsc::Sender<ToVornd>,
    (mut generation, mine): (watch::Receiver<u64>, u64),
) {
    loop {
        let changed = s.changed.notified();
        let ack = acked_note.notified();
        tokio::pin!(changed, ack);
        changed.as_mut().enable();
        ack.as_mut().enable();
        let in_flight = next
            .next_offset
            .saturating_sub(acked.load(Ordering::SeqCst));
        let batch = if in_flight >= WINDOW_BYTES {
            Vec::new()
        } else {
            let room = (WINDOW_BYTES - in_flight).min(BATCH_BYTES);
            match s.with_log(|l| l.read_batch(next, room)) {
                Ok(b) => b,
                // The records this connection needs are gone; vornd must
                // attach again, and closing the stream tells it so.
                Err(_) => return,
            }
        };
        if batch.is_empty() {
            tokio::select! {
                _ = &mut changed => {}
                _ = &mut ack => {}
                // A missed wakeup costs at most this.
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                _ = replaced(&mut generation, Some(mine)) => return,
            }
            continue;
        }
        let exit = matches!(batch.last().map(|e| &e.rec), Some(Record::Exit { .. }));
        next = batch.last().expect("non-empty").after();
        // A peer that stopped reading backs the outbox up; a newer Hello
        // must still end this pump, and nothing it queues after that counts
        // as sent.
        tokio::select! {
            sent = tx.send(entries_msg(&s.id, batch)) => if sent.is_err() { return },
            _ = replaced(&mut generation, Some(mine)) => return,
        }
        if *generation.borrow() != mine {
            return;
        }
        s.with_log(|l| l.mark_sent(next));
        if exit {
            return;
        }
    }
}

/// The endpoint for an instance under `home`.
pub fn endpoint(home: &Path, instance: u128) -> String {
    #[cfg(unix)]
    {
        // Folded to 64 bits: macOS caps a socket path at 104 bytes, and a
        // temp-dir home is long already.
        let short = (instance as u64) ^ ((instance >> 64) as u64);
        home.join("run")
            .join(format!("sessiond-{PROTO}-{short:x}.sock"))
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(windows)]
    {
        let _ = home;
        format!(
            r"\\.\pipe\vorn-sessiond-{}-{instance:x}",
            crate::os::user_sid().unwrap_or_else(|_| "user".into())
        )
    }
}

/// Sweep what dead owners left in `run/` ([`crate::rundir`]), then bind the
/// endpoint and announce it there. Connections wait until [`serve`] runs.
pub fn bind(d: &Sessiond) -> std::io::Result<crate::os::Listener> {
    crate::rundir::sweep(&d.cfg.home);
    let listener = crate::os::Listener::bind(&d.cfg.home, &d.endpoint())?;
    crate::launch::announce(
        &d.cfg.home,
        &crate::launch::Instance {
            endpoint: d.endpoint(),
            pid: std::process::id(),
            proto: PROTO,
            build: d.cfg.build.clone(),
            instance: d.cfg.instance,
            exe: std::env::current_exe().ok(),
            handoff: cfg!(unix).then_some(HANDOFF),
        },
    )?;
    Ok(listener)
}

/// Serve connections until `stop` is notified or sessiond has been idle for
/// its `idle_exit`.
pub async fn serve(d: Arc<Sessiond>, mut listener: crate::os::Listener) -> std::io::Result<()> {
    let idle = d.idle_exit();
    loop {
        tokio::select! {
            conn = listener.accept() => {
                let stream = conn?;
                #[cfg(unix)]
                tokio::spawn(first_frame(Arc::clone(&d), stream));
                #[cfg(windows)]
                tokio::spawn(Arc::clone(&d).serve_conn(stream));
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if d.idle_for(idle) || d.stopping.load(Ordering::SeqCst) {
                    break;
                }
                #[cfg(unix)]
                if d.handed_off() {
                    break;
                }
            }
            _ = d.stop.notified() => break,
        }
    }
    listener.close();
    crate::launch::withdraw(&d.cfg.home, d.cfg.instance);
    // Connections still open end with the endpoint.
    d.stop.notify_waiters();
    #[cfg(unix)]
    reap_handed(&d).await;
    Ok(())
}

/// After a handoff, stay only until every program this process started is
/// reaped: nobody else can reap them and write down how they ended.
#[cfg(unix)]
async fn reap_handed(d: &Sessiond) {
    loop {
        let reaping = d
            .reaping
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|s| s.reaping());
        if !reaping || d.stopping.load(Ordering::SeqCst) {
            return;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            _ = d.stop.notified() => {}
        }
    }
}

/// Serve a connection by its first frame: a newer sessiond's handoff, or
/// vornd.
#[cfg(unix)]
async fn first_frame(d: Arc<Sessiond>, mut stream: crate::os::Stream) {
    let mut head = [0u8; 5];
    if stream.read_exact(&mut head).await.is_err() {
        return;
    }
    if head[4] == HANDOFF_FRAME {
        handoff::serve(d, stream, head).await;
    } else {
        d.serve_read(stream, &head).await;
    }
}
