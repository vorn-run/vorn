//! Terminal streams to bytes clients, for the sessions vornd holds.
//!
//! A bytes client (the desktop renderer, the web client, the phone) parses
//! a session itself with xterm.js. vornd sends it the session's records
//! exactly, as they were applied to the session's terminal: data as
//! version 2 Bytes frames that name their records, resizes in-band as
//! `terminal:resized` at their place in the stream, and the exit last
//! (Terminal State Protocol §7, §14). Each client knows its [`Cursor`], and
//! one rule decides how it continues after any reconnect, a vornd restart
//! included (TP §12, Session Recovery Contract §6 flow G):
//!
//! - it continues without a snapshot when it resumes with a cursor of the
//!   session's epoch and every record from it on is retained: in this
//!   hub's tail ([`TAIL_BYTES`]) or in sessiond's ring, fetched with
//!   `Attach{Cursor}`;
//! - otherwise it gets a VtSnapshot cut in the session actor, stamped with
//!   the actor's cursor, and the reason it could not continue. An attach
//!   with no cursor (a version 1 client has only an offset, which is never a
//!   resume token) always gets a snapshot.
//!
//! Every client connection has one ordered outbox, which the connection's
//! writer drains. Bytes for an attachment are never held back for a slow
//! client: when the connection already has [`QUEUE_CAP`] queued and not
//! written (the socket's buffered amount, as in WP4), the attachment's
//! queued bytes are dropped and it is told to resync, and the session goes
//! on at its own pace (TP §11).
//!
//! The hub holds no terminal and parses nothing. It learns what the actor
//! applied, in the actor's order, from the engine driver ([`Streams::applied`],
//! [`Streams::snapshot_ready`]), so a snapshot's cursor and the records
//! after it always line up. What it needs done elsewhere (a snapshot cut,
//! records fetched from sessiond) it returns as [`Action`]s.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::Message;
use vorn_sessiond_wire::AttachRefusal;
use vorn_term_proto::bytes::{pack, Lost, Piece, MAX_FLUSH};
use vorn_term_proto::{Cursor, Entry};

/// Raw records kept per session after the actor applied them, so a client
/// that reconnects continues from them without a snapshot (TP §12: 4 MB to
/// start).
pub const TAIL_BYTES: usize = 4 << 20;

/// Bytes queued for one connection and not yet written, past which an
/// attachment's bytes are dropped and it resyncs (TP §11, WP4's 1 MiB).
pub const QUEUE_CAP: usize = 1 << 20;

/// Queued bytes past which the forwarder waits before queueing more of the
/// server's frames, so a client that stops reading slows the server's
/// socket rather than growing vornd.
pub const FORWARD_PAUSE: usize = 8 << 20;

/// How long a snapshot or a fetch from sessiond may take before the client
/// is answered with an error rather than kept waiting.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// Ended sessions remembered, so an attach after the exit still reports it.
const ENDED_KEPT: usize = 64;

/// One message for a connection's writer.
#[derive(Debug)]
pub struct Outgoing {
    pub msg: Message,
    size: usize,
    /// Bytes for an attachment: dropped unwritten once the attachment's
    /// generation moves on.
    stale: Option<(Arc<AtomicU64>, u64)>,
}

impl Outgoing {
    /// Whether the message is still wanted.
    pub fn current(&self) -> bool {
        self.stale
            .as_ref()
            .is_none_or(|(gen, g)| gen.load(Ordering::Acquire) == *g)
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

/// The sending half of one connection's outbox.
#[derive(Debug, Clone)]
struct Outbox {
    tx: mpsc::UnboundedSender<Outgoing>,
    queued: Arc<AtomicUsize>,
}

impl Outbox {
    fn push(&self, msg: Message, stale: Option<(Arc<AtomicU64>, u64)>) {
        let size = match &msg {
            Message::Text(t) => t.len(),
            Message::Binary(b) => b.len(),
            _ => 0,
        };
        self.queued.fetch_add(size, Ordering::AcqRel);
        if self.tx.send(Outgoing { msg, size, stale }).is_err() {
            self.queued.fetch_sub(size, Ordering::AcqRel);
        }
    }

    fn text(&self, v: &Value) {
        self.push(Message::text(v.to_string()), None);
    }

    fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

/// One client connection: what its writer drains, in order. Dropping it
/// detaches every session it was attached to.
#[derive(Debug)]
pub struct ClientConn {
    id: u64,
    rx: mpsc::UnboundedReceiver<Outgoing>,
    queued: Arc<AtomicUsize>,
    drained: Arc<Notify>,
    streams: Arc<Streams>,
    sender: Outbox,
}

impl ClientConn {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The next message still wanted, skipping bytes an overflow dropped.
    /// Call [`ClientConn::written`] with its size once it is on the wire.
    pub async fn next(&mut self) -> Option<Outgoing> {
        loop {
            let o = self.rx.recv().await?;
            if o.current() {
                return Some(o);
            }
            self.written(o.size);
        }
    }

    /// `n` bytes taken from the outbox are written.
    pub fn written(&self, n: usize) {
        self.queued.fetch_sub(n, Ordering::AcqRel);
        self.drained.notify_waiters();
    }

    /// A handle that queues the server's own frames behind native ones.
    pub fn forwarder(&self) -> Forwarder {
        Forwarder {
            outbox: self.sender.clone(),
            drained: Arc::clone(&self.drained),
        }
    }

    /// Bytes queued and not written.
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }

    /// Takes everything queued now, for tests that do not run a writer.
    pub fn drain_now(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok(o) = self.rx.try_recv() {
            self.written(o.size);
            if o.current() {
                out.push(o.msg);
            }
        }
        out
    }
}

impl Drop for ClientConn {
    fn drop(&mut self) {
        self.streams.disconnect(self.id);
    }
}

/// Queues frames from the server for one connection, in order with what
/// vornd sends it.
#[derive(Debug, Clone)]
pub struct Forwarder {
    outbox: Outbox,
    drained: Arc<Notify>,
}

impl Forwarder {
    /// Queues `msg`, first waiting while the connection is far behind.
    pub async fn send(&self, msg: Message) {
        loop {
            let drained = self.drained.notified();
            if self.outbox.queued() <= FORWARD_PAUSE || self.outbox.tx.is_closed() {
                break;
            }
            drained.await;
        }
        self.outbox.push(msg, None);
    }

    /// Queues a JSON-RPC message without waiting: answers to the client's
    /// own calls.
    pub fn send_now(&self, v: &Value) {
        self.outbox.text(v);
    }
}

/// What the hub needs done by whoever drives the session engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Ask the session's actor for a VtSnapshot under this token.
    Snapshot { session: String, token: u64 },
    /// Ask the session's actor for its last `lines` lines under this token.
    Output {
        session: String,
        token: u64,
        lines: u32,
    },
    /// Ask sessiond for the session's records from `from`.
    Fetch { session: String, from: Cursor },
}

/// A VtSnapshot as the session actor cut it (TP §7): the hub's view of
/// it, so the hub does not depend on the engine.
#[derive(Debug, Clone, Copy)]
pub struct Snap<'a> {
    pub resume: Cursor,
    pub cols: u16,
    pub rows: u16,
    pub vt: &'a [u8],
    pub title: &'a str,
    pub cwd: &'a str,
}

/// An attach a client asked for and that is not answered yet.
#[derive(Debug, Clone)]
struct Asked {
    conn: u64,
    rpc: Value,
    cursor: Option<Cursor>,
    at: Instant,
}

/// Why a client asked for a snapshot: to attach, or only to read it.
#[derive(Debug, Clone)]
enum Wants {
    Attach { resync: Option<&'static str> },
    Scrollback,
    Output,
}

#[derive(Debug, Clone)]
struct Waiting {
    session: String,
    conn: u64,
    rpc: Value,
    wants: Wants,
    at: Instant,
}

#[derive(Debug)]
enum Mode {
    /// Receives every record from `next` on as it is applied.
    Following,
    /// Waits for sessiond to send the records from `next` again; answers
    /// the attach once they come.
    Fetching { rpc: Value, at: Instant },
}

#[derive(Debug)]
struct Attachment {
    next: Cursor,
    gen: Arc<AtomicU64>,
    mode: Mode,
}

/// One session, as the hub keeps it.
#[derive(Debug)]
struct Stream {
    epoch: u32,
    /// Replay reached the head: clients may attach.
    live: bool,
    /// After the last record the actor applied, once known.
    head: Option<Cursor>,
    tail: VecDeque<Entry>,
    tail_bytes: usize,
    attached: HashMap<u64, Attachment>,
    /// Attaches that came before the session was live.
    early: Vec<Asked>,
    /// A fetch from sessiond in flight, from this cursor.
    fetch: Option<Cursor>,
}

impl Stream {
    fn new(epoch: u32) -> Stream {
        Stream {
            epoch,
            live: false,
            head: None,
            tail: VecDeque::new(),
            tail_bytes: 0,
            attached: HashMap::new(),
            early: Vec::new(),
            fetch: None,
        }
    }

    /// Adds what extends the tail and drops the oldest past the cap.
    fn extend(&mut self, entries: Vec<Entry>) {
        for e in entries {
            match self.head {
                Some(h) if h.includes(&e.hdr) => continue,
                Some(h) if h.is_followed_by(&e.hdr) => {}
                // Not contiguous: what is held cannot be continued through.
                _ => {
                    self.tail.clear();
                    self.tail_bytes = 0;
                }
            }
            self.tail_bytes += e.rec.len() as usize;
            self.head = Some(e.after());
            self.tail.push_back(e);
        }
        while self.tail_bytes > TAIL_BYTES && self.tail.len() > 1 {
            if let Some(e) = self.tail.pop_front() {
                self.tail_bytes -= e.rec.len() as usize;
            }
        }
    }

    /// Whether the tail holds every record from `c` on.
    fn covers(&self, c: &Cursor) -> bool {
        let Some(head) = self.head else {
            return false;
        };
        if c.epoch != head.epoch {
            return false;
        }
        if *c == head {
            return true;
        }
        self.tail
            .iter()
            .any(|e| e.hdr.rseq == c.next_rseq && e.hdr.start_offset == c.next_offset)
    }
}

/// Why a session ended, for attaches that come after.
#[derive(Debug, Clone)]
struct Ended {
    session: String,
    exit_code: i64,
    screen: String,
}

#[derive(Debug, Default)]
struct Inner {
    conns: HashMap<u64, Outbox>,
    sessions: HashMap<String, Stream>,
    ended: VecDeque<Ended>,
    waiting: HashMap<u64, Waiting>,
    next_conn: u64,
    next_token: u64,
}

/// Every session's stream and every client connection.
#[derive(Debug, Default)]
pub struct Streams {
    inner: Mutex<Inner>,
}

/// A JSON-RPC notification.
fn note(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

/// A JSON-RPC answer to `rpc`.
pub fn answer(rpc: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": rpc, "result": result })
}

/// A JSON-RPC error answer to `rpc`.
pub fn refuse(rpc: &Value, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": rpc, "error": { "code": -32000, "message": message } })
}

pub fn cursor_json(c: &Cursor) -> Value {
    json!({ "epoch": c.epoch, "nextRseq": c.next_rseq, "nextOffset": c.next_offset })
}

/// The `seq` a client compares its held chunks with: a v2 chunk's `seq` is
/// its last record, so everything at or below this is in the snapshot.
fn seq_of(c: &Cursor) -> i64 {
    i64::try_from(c.next_rseq).map_or(i64::MAX, |n| n - 1)
}

/// The exit code a client is told: the code, or 128 plus the signal.
pub fn exit_code(code: Option<i32>, signal: Option<i32>) -> i64 {
    match (code, signal) {
        (Some(c), _) => i64::from(c),
        (None, Some(s)) => 128 + i64::from(s),
        (None, None) => 0,
    }
}

impl Streams {
    pub fn new() -> Arc<Streams> {
        Arc::new(Streams::default())
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new client connection.
    pub fn connect(self: &Arc<Self>) -> ClientConn {
        let (tx, rx) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let sender = Outbox {
            tx,
            queued: Arc::clone(&queued),
        };
        let mut inner = self.inner();
        inner.next_conn += 1;
        let id = inner.next_conn;
        inner.conns.insert(id, sender.clone());
        ClientConn {
            id,
            rx,
            queued,
            drained: Arc::new(Notify::new()),
            streams: Arc::clone(self),
            sender,
        }
    }

    fn disconnect(&self, conn: u64) {
        let mut inner = self.inner();
        inner.conns.remove(&conn);
        for s in inner.sessions.values_mut() {
            if let Some(a) = s.attached.remove(&conn) {
                a.gen.fetch_add(1, Ordering::AcqRel);
            }
            s.early.retain(|a| a.conn != conn);
        }
        inner.waiting.retain(|_, w| w.conn != conn);
    }

    /// Whether vornd answers terminal calls for `session` itself.
    pub fn holds(&self, session: &str) -> bool {
        let inner = self.inner();
        inner.sessions.contains_key(session) || inner.ended.iter().any(|e| e.session == session)
    }

    /// A session the engine took on, recovering or newly spawned.
    pub fn opened(&self, session: &str, epoch: u32) {
        let mut inner = self.inner();
        let s = inner
            .sessions
            .entry(session.to_owned())
            .or_insert_with(|| Stream::new(epoch));
        s.live = false;
        if s.epoch != epoch {
            *s = Stream {
                attached: std::mem::take(&mut s.attached),
                ..Stream::new(epoch)
            };
        }
    }

    /// The engine's connection to sessiond ended: every session waits for
    /// the next one to recover it.
    pub fn suspended(&self) {
        let mut inner = self.inner();
        for s in inner.sessions.values_mut() {
            s.live = false;
            s.fetch = None;
        }
    }

    /// Replay reached the head at `at`: answers attaches that waited.
    pub fn live(&self, session: &str, at: Cursor) -> Vec<Action> {
        let mut inner = self.inner();
        let early = {
            let Some(s) = inner.sessions.get_mut(session) else {
                return Vec::new();
            };
            s.live = true;
            match s.head {
                Some(h) if h == at => {}
                _ => {
                    s.tail.clear();
                    s.tail_bytes = 0;
                    s.head = Some(at);
                }
            }
            std::mem::take(&mut s.early)
        };
        let mut actions = Vec::new();
        for a in early {
            inner.attach(session, a, &mut actions);
        }
        actions
    }

    /// A client asks to attach `session`, resuming from `cursor` if it has
    /// one. The answer goes to its connection.
    pub fn attach(
        &self,
        conn: u64,
        session: &str,
        rpc: Value,
        cursor: Option<Cursor>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let asked = Asked {
            conn,
            rpc,
            cursor,
            at: Instant::now(),
        };
        self.inner().attach(session, asked, &mut actions);
        actions
    }

    /// A client asks for the screen of `session` as VT, without attaching.
    pub fn read_scrollback(&self, conn: u64, session: &str, rpc: Value) -> Vec<Action> {
        let mut inner = self.inner();
        if !inner.sessions.get(session).is_some_and(|s| s.live) {
            let screen = inner
                .ended
                .iter()
                .find(|e| e.session == session)
                .map(|e| e.screen.clone());
            if let Some(out) = inner.conns.get(&conn) {
                match screen {
                    Some(data) => out.text(&answer(&rpc, json!({ "data": data }))),
                    None => out.text(&refuse(&rpc, "the session is not ready")),
                }
            }
            return Vec::new();
        }
        let token = inner.token();
        inner.waiting.insert(
            token,
            Waiting {
                session: session.to_owned(),
                conn,
                rpc,
                wants: Wants::Scrollback,
                at: Instant::now(),
            },
        );
        vec![Action::Snapshot {
            session: session.to_owned(),
            token,
        }]
    }

    /// A client asks for the last `lines` lines of `session`'s output, as
    /// text.
    pub fn read_output(&self, conn: u64, session: &str, rpc: Value, lines: u32) -> Vec<Action> {
        let mut inner = self.inner();
        if !inner.sessions.contains_key(session) {
            if let Some(out) = inner.conns.get(&conn) {
                out.text(&answer(&rpc, json!([])));
            }
            return Vec::new();
        }
        let token = inner.token();
        inner.waiting.insert(
            token,
            Waiting {
                session: session.to_owned(),
                conn,
                rpc,
                wants: Wants::Output,
                at: Instant::now(),
            },
        );
        vec![Action::Output {
            session: session.to_owned(),
            token,
            lines,
        }]
    }

    /// The actor answered output request `token`.
    pub fn output_ready(&self, token: u64, lines: Option<&[String]>) {
        let mut inner = self.inner();
        let Some(w) = inner.waiting.remove(&token) else {
            return;
        };
        if let Some(out) = inner.conns.get(&w.conn) {
            out.text(&answer(&w.rpc, json!(lines.unwrap_or_default())));
        }
    }

    /// The engine could not ask for `token`: the session is gone.
    pub fn failed(&self, token: u64) {
        let mut inner = self.inner();
        let Some(w) = inner.waiting.remove(&token) else {
            return;
        };
        if let Some(out) = inner.conns.get(&w.conn) {
            out.text(&refuse(&w.rpc, "the session is not running"));
        }
    }

    /// After the last record the hub saw applied to `session`.
    pub fn head(&self, session: &str) -> Option<Cursor> {
        self.inner().sessions.get(session).and_then(|s| s.head)
    }

    /// The actor cut snapshot `token`, in order with what it applied.
    pub fn snapshot_ready(&self, token: u64, snap: Option<Snap<'_>>) {
        let mut inner = self.inner();
        let Some(w) = inner.waiting.remove(&token) else {
            return;
        };
        let Some(out) = inner.conns.get(&w.conn).cloned() else {
            return;
        };
        let Some(snap) = snap else {
            out.text(&refuse(&w.rpc, "the session has no terminal to show"));
            return;
        };
        let data = String::from_utf8_lossy(snap.vt);
        match w.wants {
            Wants::Output => out.text(&refuse(&w.rpc, "not an output request")),
            Wants::Scrollback => out.text(&answer(&w.rpc, json!({ "data": data }))),
            Wants::Attach { resync } => {
                let Some(s) = inner.sessions.get_mut(&w.session) else {
                    out.text(&refuse(&w.rpc, "the session ended"));
                    return;
                };
                let mut result = json!({
                    "data": data,
                    "seq": seq_of(&snap.resume),
                    "live": true,
                    "cursor": cursor_json(&snap.resume),
                    "continued": false,
                    "cols": snap.cols,
                    "rows": snap.rows,
                    "title": snap.title,
                    "cwd": snap.cwd,
                    "replies": "vornd",
                });
                if let Some(why) = resync {
                    result["resync"] = json!(why);
                }
                out.text(&answer(&w.rpc, result));
                let replaced = s.attached.insert(
                    w.conn,
                    Attachment {
                        next: snap.resume,
                        gen: fresh_gen(),
                        mode: Mode::Following,
                    },
                );
                if let Some(old) = replaced {
                    old.gen.fetch_add(1, Ordering::AcqRel);
                }
                // Records applied after the cut and before this answer.
                let mut packer = Packer::new(&w.session, s.tail.make_contiguous());
                deliver(&mut s.attached, w.conn, &out, &mut packer);
            }
        }
    }

    /// The records of one batch the actor applied, in its order: added to
    /// the tail and sent to every attachment past its cursor. One that
    /// cannot go on is told to resync.
    pub fn applied(&self, session: &str, entries: Vec<Entry>) {
        let mut inner = self.inner();
        let Inner {
            conns, sessions, ..
        } = &mut *inner;
        let Some(s) = sessions.get_mut(session) else {
            return;
        };
        let mut packer = Packer::new(session, &entries);
        let ids: Vec<u64> = s.attached.keys().copied().collect();
        for conn in ids {
            if let Some(out) = conns.get(&conn) {
                deliver(&mut s.attached, conn, out, &mut packer);
            }
        }
        s.extend(entries);
        let fetching = s
            .attached
            .values()
            .any(|a| matches!(a.mode, Mode::Fetching { .. }));
        if !fetching {
            s.fetch = None;
        }
    }

    /// sessiond refused a fetch for `session`. False when none was in
    /// flight, and the refusal is the actor's.
    pub fn fetch_refused(&self, session: &str, why: AttachRefusal) -> Vec<Action> {
        let mut inner = self.inner();
        let refused = {
            let Some(s) = inner.sessions.get_mut(session) else {
                return Vec::new();
            };
            if s.fetch.take().is_none() {
                return Vec::new();
            }
            let mut refused = Vec::new();
            s.attached.retain(|&conn, a| match &a.mode {
                Mode::Fetching { rpc, .. } => {
                    refused.push((conn, rpc.clone()));
                    false
                }
                Mode::Following => true,
            });
            refused
        };
        let why = match why {
            AttachRefusal::WrongEpoch => Lost::WrongEpoch,
            _ => Lost::NotRetained,
        };
        let mut actions = Vec::new();
        for (conn, rpc) in refused {
            inner.snapshot_for(session, conn, rpc, Some(why.as_str()), &mut actions);
        }
        actions
    }

    /// Whether a fetch for `session` is in flight: a refusal sessiond sends
    /// for it is the hub's, not the actor's.
    pub fn fetching(&self, session: &str) -> bool {
        self.inner()
            .sessions
            .get(session)
            .is_some_and(|s| s.fetch.is_some())
    }

    /// The bell rang in `session`: every connection hears it.
    pub fn bell(&self, session: &str) {
        let v = note("terminal:bell", json!({ "id": session }));
        for out in self.inner().conns.values() {
            out.text(&v);
        }
    }

    /// The session left the engine. Attached clients had its exit in the
    /// stream; every other connection is told now.
    pub fn closed(&self, session: &str, exited: Option<(Option<i32>, Option<i32>)>, screen: &str) {
        let mut inner = self.inner();
        let Some((code, signal)) = exited else {
            // Lost, not ended: the next connection to sessiond takes it on
            // again, and its clients reattach once it is live.
            let Inner {
                conns, sessions, ..
            } = &mut *inner;
            let Some(s) = sessions.get_mut(session) else {
                return;
            };
            s.live = false;
            let v = note(
                "terminal:resync",
                json!({ "id": session, "reason": "restarted" }),
            );
            for (conn, a) in s.attached.drain() {
                a.gen.fetch_add(1, Ordering::AcqRel);
                if let Some(out) = conns.get(&conn) {
                    out.text(&v);
                }
            }
            return;
        };
        let Some(s) = inner.sessions.remove(session) else {
            return;
        };
        let exit_code = exit_code(code, signal);
        let v = note(
            "terminal:exit",
            json!({ "id": session, "exitCode": exit_code }),
        );
        for (conn, out) in &inner.conns {
            if !s.attached.contains_key(conn) {
                out.text(&v);
            }
        }
        if inner.ended.len() == ENDED_KEPT {
            inner.ended.pop_front();
        }
        inner.ended.push_back(Ended {
            session: session.to_owned(),
            exit_code,
            screen: screen.replace('\n', "\r\n"),
        });
    }

    /// Answers with an error the attaches and reads that have waited longer
    /// than [`ANSWER_TIMEOUT`] for a snapshot or a fetch.
    pub fn expire(&self, now: Instant) {
        let mut inner = self.inner();
        let old: Vec<u64> = inner
            .waiting
            .iter()
            .filter(|(_, w)| now.duration_since(w.at) >= ANSWER_TIMEOUT)
            .map(|(&t, _)| t)
            .collect();
        for t in old {
            if let Some(w) = inner.waiting.remove(&t) {
                if let Some(out) = inner.conns.get(&w.conn) {
                    out.text(&refuse(&w.rpc, "the session did not answer in time"));
                }
            }
        }
        let Inner {
            conns, sessions, ..
        } = &mut *inner;
        for s in sessions.values_mut() {
            s.attached.retain(|conn, a| match &a.mode {
                Mode::Fetching { rpc, at } if now.duration_since(*at) >= ANSWER_TIMEOUT => {
                    if let Some(out) = conns.get(conn) {
                        out.text(&refuse(rpc, "the session holder did not answer in time"));
                    }
                    false
                }
                _ => true,
            });
            s.early.retain(|a| {
                let keep = now.duration_since(a.at) < ANSWER_TIMEOUT;
                if !keep {
                    if let Some(out) = conns.get(&a.conn) {
                        out.text(&refuse(&a.rpc, "the session is not ready"));
                    }
                }
                keep
            });
        }
    }
}

impl Inner {
    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// One attach, under the lock.
    fn attach(&mut self, session: &str, a: Asked, actions: &mut Vec<Action>) {
        let Some(out) = self.conns.get(&a.conn).cloned() else {
            return;
        };
        let Some(s) = self.sessions.get_mut(session) else {
            match self.ended.iter().find(|e| e.session == session) {
                Some(e) => out.text(&answer(
                    &a.rpc,
                    json!({
                        "data": e.screen,
                        "seq": 0,
                        "live": false,
                        "continued": false,
                        "exitCode": e.exit_code,
                        "replies": "vornd",
                    }),
                )),
                None => out.text(&refuse(&a.rpc, "no such session")),
            }
            return;
        };
        // A second attach on one connection replaces the first.
        if let Some(old) = s.attached.remove(&a.conn) {
            old.gen.fetch_add(1, Ordering::AcqRel);
            if let Mode::Fetching { rpc, .. } = old.mode {
                out.text(&refuse(&rpc, "replaced by a later attach"));
            }
        }
        if !s.live {
            s.early.push(a);
            return;
        }
        let Some(c) = a.cursor else {
            return self.snapshot_for(session, a.conn, a.rpc, None, actions);
        };
        if c.epoch != s.epoch {
            return self.snapshot_for(
                session,
                a.conn,
                a.rpc,
                Some(Lost::WrongEpoch.as_str()),
                actions,
            );
        }
        if s.covers(&c) {
            // Packed first: a gap in what is held means a snapshot after all.
            let packed = prepare(session, c, s.tail.make_contiguous());
            if let Some(why) = packed.lost {
                let why = Some(why.as_str());
                return self.snapshot_for(session, a.conn, a.rpc, why, actions);
            }
            out.text(&answer(&a.rpc, continued(&c)));
            let gen = fresh_gen();
            let queued = queue(&out, &gen, &packed.msgs);
            s.attached.insert(
                a.conn,
                Attachment {
                    next: packed.to,
                    gen,
                    mode: Mode::Following,
                },
            );
            if !queued {
                overflowed(session, &mut s.attached, a.conn, &out);
            }
            return;
        }
        let ahead = s.head.is_some_and(|h| c.next_rseq > h.next_rseq);
        if ahead {
            let why = Some(Lost::NotRetained.as_str());
            return self.snapshot_for(session, a.conn, a.rpc, why, actions);
        }
        // Older than what is held: sessiond's ring may still have it.
        let gen = fresh_gen();
        s.attached.insert(
            a.conn,
            Attachment {
                next: c,
                gen,
                mode: Mode::Fetching {
                    rpc: a.rpc,
                    at: a.at,
                },
            },
        );
        if s.fetch.is_none_or(|f| c.next_rseq < f.next_rseq) {
            s.fetch = Some(c);
            actions.push(Action::Fetch {
                session: session.to_owned(),
                from: c,
            });
        }
    }

    fn snapshot_for(
        &mut self,
        session: &str,
        conn: u64,
        rpc: Value,
        resync: Option<&'static str>,
        actions: &mut Vec<Action>,
    ) {
        let token = self.token();
        self.waiting.insert(
            token,
            Waiting {
                session: session.to_owned(),
                conn,
                rpc,
                wants: Wants::Attach { resync },
                at: Instant::now(),
            },
        );
        actions.push(Action::Snapshot {
            session: session.to_owned(),
            token,
        });
    }
}

/// The answer to an attach that continues from the client's cursor.
fn continued(c: &Cursor) -> Value {
    json!({
        "data": "",
        "seq": seq_of(c),
        "live": true,
        "cursor": cursor_json(c),
        "continued": true,
        "replies": "vornd",
    })
}

/// A new attachment's generation counter. Each attachment has its own, so
/// bytes a replaced one had queued are dropped with it.
fn fresh_gen() -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(0))
}

/// What a run of records becomes for a client from one cursor: packed
/// once, and shared by every attachment that stands at that cursor, which
/// with the desktop, a phone and a browser following one session is all of
/// them. The frames are refcounted, so sharing them copies nothing.
struct Packed {
    from: Cursor,
    to: Cursor,
    msgs: Vec<Ready>,
    /// Why the stream stops after `msgs`, if it does.
    lost: Option<Lost>,
}

/// A message ready for an attachment's outbox.
enum Ready {
    /// Bytes or a resize: dropped with the rest when the client falls behind.
    Stream(Message),
    /// The exit, which every client is told even after an overflow.
    Exit(Message),
}

fn prepare(session: &str, from: Cursor, entries: &[Entry]) -> Packed {
    let mut to = from;
    let mut pieces = Vec::new();
    // Only an id that does not fit a frame fails, and sessiond's ids are
    // UUIDs; such a session's clients would get no bytes, never wrong ones.
    let _ = pack(session, &mut to, entries, MAX_FLUSH, &mut pieces);
    let mut lost = None;
    let msgs = pieces
        .into_iter()
        .filter_map(|p| match p {
            Piece::Frame(f) => Some(Ready::Stream(Message::binary(f))),
            Piece::Resized { rseq, cols, rows } => Some(Ready::Stream(Message::text(
                note(
                    "terminal:resized",
                    json!({ "id": session, "cols": cols, "rows": rows, "rseq": rseq }),
                )
                .to_string(),
            ))),
            Piece::Exit { code, signal, .. } => Some(Ready::Exit(Message::text(
                note(
                    "terminal:exit",
                    json!({ "id": session, "exitCode": exit_code(code, signal) }),
                )
                .to_string(),
            ))),
            Piece::Lost(why) => {
                lost = Some(why);
                None
            }
        })
        .collect();
    Packed {
        from,
        to,
        msgs,
        lost,
    }
}

/// Packs one batch for every attachment, once per cursor they stand at.
struct Packer<'a> {
    session: &'a str,
    entries: &'a [Entry],
    done: Vec<Packed>,
}

impl<'a> Packer<'a> {
    fn new(session: &'a str, entries: &'a [Entry]) -> Packer<'a> {
        Packer {
            session,
            entries,
            done: Vec::new(),
        }
    }

    fn from(&mut self, at: Cursor) -> &Packed {
        let i = match self.done.iter().position(|p| p.from == at) {
            Some(i) => i,
            None => {
                self.done.push(prepare(self.session, at, self.entries));
                self.done.len() - 1
            }
        };
        &self.done[i]
    }
}

/// Sends `conn` what the packer's records hold past its cursor. One that
/// cannot go on is told to resync, and its attachment is gone.
fn deliver(
    attached: &mut HashMap<u64, Attachment>,
    conn: u64,
    out: &Outbox,
    packer: &mut Packer<'_>,
) {
    let session = packer.session;
    let Some(a) = attached.get_mut(&conn) else {
        return;
    };
    let fetched;
    let packed = match &a.mode {
        Mode::Following => packer.from(a.next),
        // Waiting for sessiond to send the records from its cursor again:
        // they start exactly there, and the attach is answered then.
        Mode::Fetching { rpc, .. } => {
            let Some(start) = packer
                .entries
                .iter()
                .position(|e| a.next.is_followed_by(&e.hdr))
            else {
                return;
            };
            out.text(&answer(rpc, continued(&a.next)));
            a.mode = Mode::Following;
            fetched = prepare(session, a.next, &packer.entries[start..]);
            &fetched
        }
    };
    a.next = packed.to;
    if !queue(out, &a.gen, &packed.msgs) {
        return overflowed(session, attached, conn, out);
    }
    if let Some(why) = packed.lost {
        attached.remove(&conn);
        out.text(&note(
            "terminal:resync",
            json!({ "id": session, "reason": why.as_str() }),
        ));
    }
}

/// Queues an attachment's messages. False when the connection was already
/// over [`QUEUE_CAP`]: nothing more of its stream was queued.
fn queue(out: &Outbox, gen: &Arc<AtomicU64>, msgs: &[Ready]) -> bool {
    let g = gen.load(Ordering::Acquire);
    for m in msgs {
        match m {
            Ready::Stream(msg) => {
                if out.queued() > QUEUE_CAP {
                    return false;
                }
                out.push(msg.clone(), Some((Arc::clone(gen), g)));
            }
            Ready::Exit(msg) => out.push(msg.clone(), None),
        }
    }
    true
}

/// The connection fell [`QUEUE_CAP`] behind: what it had queued for this
/// session is dropped, and it is told to resync, which gives it a snapshot.
fn overflowed(session: &str, attached: &mut HashMap<u64, Attachment>, conn: u64, out: &Outbox) {
    if let Some(a) = attached.remove(&conn) {
        a.gen.fetch_add(1, Ordering::AcqRel);
    }
    out.text(&note(
        "terminal:resync",
        json!({ "id": session, "reason": "overflow" }),
    ));
}
