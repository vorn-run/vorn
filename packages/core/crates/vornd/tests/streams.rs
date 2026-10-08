//! Terminal streams to bytes clients, through vornd's engine and a sessiond
//! that serves seeded record logs from vorn-recovery, so what is retained,
//! where a gap falls and when vornd dies are the test's to choose.
//!
//! The bytes client here stands in for xterm.js: a Ghostty terminal fed the
//! attach answer's VT and then every Bytes frame and `terminal:resized`, in
//! order, keeping the cursor the frames give it. It is compared with a
//! reference terminal fed the log's records directly.

#![cfg(all(feature = "engine", unix))]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;
use vorn_engine::{Cadence, Config};
use vorn_recovery::gen::{Generator, Profile};
use vorn_recovery::{LogBuilder, Size};
use vorn_screen::Emulator;
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{
    AttachFrom, AttachRefusal, Checkpoint, Entries, FrameReader, Kind, Message as _, Refused,
    SessionInfo, ToSessiond, ToVornd, Welcome, PROTO,
};
use vorn_sessiond_wire::{Io, SpawnSpec};
use vorn_term_proto::bytes::BytesFrame;
use vorn_term_proto::{Cursor, Entry, GapReason, Record, RecordHeader, Stream};
use vornd::engine::Engine;
use vornd::holder::{self, Holder};
use vornd::streams::{ClientConn, QUEUE_CAP};

const PATIENCE: Duration = Duration::from_secs(20);
const SESSION: &str = "s1";
const SCROLLBACK: usize = 4 << 20;

// ---------------------------------------------------------------------------
// A sessiond holding one session, its log in the test's hands.

struct Held {
    epoch: u32,
    size: (u16, u16),
    log: Vec<Entry>,
    /// The first rseq still retained.
    oldest: u64,
    newest: Option<Checkpoint>,
    fallback: Option<Checkpoint>,
    /// Bytes vornd wrote to the program, in order.
    writes: Vec<Vec<u8>>,
}

impl Held {
    fn head(&self) -> Cursor {
        self.log
            .last()
            .map_or(Cursor::start(self.epoch), Entry::after)
    }

    fn retained(&self) -> Cursor {
        match self.oldest {
            0 => Cursor::start(self.epoch),
            n => self.log[n as usize - 1].after(),
        }
    }

    fn info(&self) -> SessionInfo {
        SessionInfo {
            session: SESSION.into(),
            kind: Kind::Pty,
            pid: 1,
            epoch: self.epoch,
            oldest: self.retained(),
            head: self.head(),
            newest_cp: self.newest.as_ref().map(|c| c.resume),
            retain_from: self.retained(),
            sent: self.head(),
            cols: self.size.0,
            rows: self.size.1,
            exited: None,
            spooled_bytes: 0,
        }
    }

    fn from(&self, c: Cursor) -> Result<Vec<Entry>, AttachRefusal> {
        if c.epoch != self.epoch {
            return Err(AttachRefusal::WrongEpoch);
        }
        if c.next_rseq < self.oldest || c.next_rseq > self.head().next_rseq {
            return Err(AttachRefusal::NotRetained);
        }
        Ok(self.log[c.next_rseq as usize..].to_vec())
    }

    fn push(&mut self, rec: Record) {
        let at = self.head();
        self.log.push(Entry {
            hdr: RecordHeader {
                epoch: at.epoch,
                rseq: at.next_rseq,
                start_offset: at.next_offset,
            },
            at_ns: 0,
            rec,
        });
    }
}

#[derive(Clone)]
struct Fake {
    path: PathBuf,
    held: Arc<Mutex<Held>>,
    grew: Arc<Notify>,
}

impl Fake {
    /// A sessiond holding `entries` (rseq 0 on) of a session spawned at
    /// `size`.
    async fn start(dir: &std::path::Path, size: (u16, u16), entries: Vec<Entry>) -> Fake {
        let path = dir.join("sessiond.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let fake = Fake {
            path,
            held: Arc::new(Mutex::new(Held {
                epoch: entries.first().map_or(0, |e| e.hdr.epoch),
                size,
                log: entries,
                oldest: 0,
                newest: None,
                fallback: None,
                writes: Vec::new(),
            })),
            grew: Arc::new(Notify::new()),
        };
        let f = fake.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = listener.accept().await {
                tokio::spawn(f.clone().serve(s));
            }
        });
        fake
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held.lock().unwrap()
    }

    /// Appends records as the program's output, and wakes the pump.
    fn push(&self, recs: impl IntoIterator<Item = Record>) {
        let mut h = self.held();
        for r in recs {
            h.push(r);
        }
        drop(h);
        self.grew.notify_waiters();
    }

    fn data(&self, bytes: &[u8]) {
        self.push([Record::Data {
            stream: Stream::Pty,
            bytes: bytes.to_vec(),
        }]);
    }

    fn head(&self) -> Cursor {
        self.held().head()
    }

    /// Drops every record sessiond may: those before the older of the two
    /// checkpoints it keeps (RC §4, retain_from).
    fn trim(&self) {
        let mut h = self.held();
        h.oldest = h.fallback.as_ref().map_or(0, |cp| cp.resume.next_rseq);
    }

    async fn serve(self, mut s: UnixStream) {
        let mut frames = FrameReader::default();
        let mut buf = vec![0u8; 64 << 10];
        // The next record to send once attached.
        let mut pump: Option<Cursor> = None;
        loop {
            let grew = self.grew.notified();
            let mut sends = Vec::new();
            while let Ok(Some(m)) = frames.read::<ToSessiond>() {
                sends.extend(self.answer(m, &mut pump));
            }
            if let Some(at) = pump {
                let h = self.held();
                if at.next_rseq < h.head().next_rseq && at.epoch == h.epoch {
                    let entries = h.log[at.next_rseq as usize..].to_vec();
                    pump = Some(h.head());
                    sends.push(entries_msg(entries));
                }
            }
            for m in sends {
                if s.write_all(&m.encode()).await.is_err() {
                    return;
                }
            }
            tokio::select! {
                r = s.read(&mut buf) => match r {
                    Ok(0) | Err(_) => return,
                    Ok(n) => frames.push(&buf[..n]),
                },
                () = grew => {}
            }
        }
    }

    fn answer(&self, m: ToSessiond, pump: &mut Option<Cursor>) -> Vec<ToVornd> {
        let mut h = self.held();
        match m {
            ToSessiond::Hello(_) => vec![ToVornd::Welcome(Welcome {
                proto: PROTO,
                sessiond_instance: 1,
                sessiond_build: "fake".into(),
                sessions: vec![h.info()],
            })],
            ToSessiond::Attach(a) => {
                let (cp, from) = match a.from {
                    AttachFrom::NewestCheckpoint => match &h.newest {
                        Some(cp) => (Some(cp.clone()), cp.resume),
                        None => return vec![refused(AttachRefusal::NoSuchCheckpoint)],
                    },
                    AttachFrom::FallbackCheckpoint => match &h.fallback {
                        Some(cp) => (Some(cp.clone()), cp.resume),
                        None => return vec![refused(AttachRefusal::NoSuchCheckpoint)],
                    },
                    AttachFrom::SessionStart => (None, Cursor::start(h.epoch)),
                    AttachFrom::Cursor(c) => (None, c),
                };
                match h.from(from) {
                    Err(why) => vec![refused(why)],
                    Ok(entries) => {
                        *pump = Some(h.head());
                        let mut out: Vec<ToVornd> =
                            cp.map(ToVornd::CheckpointIs).into_iter().collect();
                        if !entries.is_empty() {
                            out.push(entries_msg(entries));
                        }
                        out
                    }
                }
            }
            ToSessiond::PutCheckpoint(cp) => {
                h.fallback = h.newest.replace(cp);
                Vec::new()
            }
            ToSessiond::Write(w) => {
                h.writes.push(w.bytes);
                Vec::new()
            }
            ToSessiond::Resize(r) => {
                h.size = (r.cols, r.rows);
                h.push(Record::Resize {
                    cols: r.cols,
                    rows: r.rows,
                    px_w: 0,
                    px_h: 0,
                    req: Some(r.req),
                });
                drop(h);
                self.grew.notify_waiters();
                Vec::new()
            }
            ToSessiond::Ping(n) => vec![ToVornd::Pong(n)],
            _ => Vec::new(),
        }
    }
}

fn refused(why: AttachRefusal) -> ToVornd {
    ToVornd::Refused(Refused {
        session: SESSION.into(),
        why,
    })
}

fn entries_msg(entries: Vec<Entry>) -> ToVornd {
    ToVornd::Entries(Entries {
        session: SESSION.into(),
        entries,
    })
}

// ---------------------------------------------------------------------------
// vornd, killed and started again.

struct Vornd {
    engine: Arc<Engine>,
    task: tokio::task::JoinHandle<()>,
}

impl Vornd {
    fn start(fake: &Fake, cadence: Cadence) -> Vornd {
        Vornd::start_at(fake.path.to_string_lossy().into_owned(), cadence)
    }

    /// A vornd connected to the sessiond at `endpoint`.
    fn start_at(endpoint: String, cadence: Cadence) -> Vornd {
        let engine = Engine::new(Config {
            scrollback: SCROLLBACK,
            cadence,
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        Vornd { engine, task }
    }

    /// No last word to sessiond, and every client connection goes with it.
    async fn kill(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    /// Cuts a checkpoint at sessiond's head and waits until sessiond holds
    /// it.
    async fn checkpoint(&self, fake: &Fake) {
        self.caught_up(fake).await;
        self.engine.flush().await;
        let head = fake.head();
        let t = Instant::now();
        while fake.held().newest.as_ref().map(|c| c.resume) != Some(head) {
            assert!(
                t.elapsed() < PATIENCE,
                "no checkpoint at {head:?}: {}",
                self.engine.report()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Cuts two checkpoints at sessiond's head, so the ring may be trimmed
    /// up to it: the newest and the fallback both lie past everything there.
    async fn checkpoint_twice(&self, fake: &Fake) {
        for _ in 0..2 {
            fake.data(b"\r\n");
            self.checkpoint(fake).await;
        }
        let h = fake.held();
        let head = h.head();
        assert_eq!(
            h.fallback.as_ref().map(|c| c.resume.next_rseq),
            Some(head.next_rseq - 1)
        );
    }

    /// Waits until the session is live at sessiond's head.
    async fn caught_up(&self, fake: &Fake) {
        let t = Instant::now();
        loop {
            let head = fake.head();
            let at = self
                .engine
                .report()
                .get("sessions")
                .and_then(|s| s.get(0))
                .map(|s| (s["state"].clone(), s["cursor"]["nextRseq"].as_u64()));
            if let Some((state, Some(rseq))) = &at {
                if state == "live" && *rseq == head.next_rseq {
                    return;
                }
            }
            assert!(
                t.elapsed() < PATIENCE,
                "never caught up: {at:?} vs {head:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn no_cadence() -> Cadence {
    Cadence {
        bytes: u64::MAX,
        quiet: Duration::from_secs(3600),
        quiet_bytes: u64::MAX,
    }
}

// ---------------------------------------------------------------------------
// The bytes client.

/// How an attach was answered.
#[derive(Debug, Clone, PartialEq)]
struct Answer {
    continued: bool,
    cursor: Cursor,
    resync: Option<String>,
}

struct Client {
    conn: ClientConn,
    engine: Arc<Engine>,
    term: Option<Emulator>,
    cursor: Option<Cursor>,
    /// Every data byte received since the client last started from a
    /// snapshot, or since it first attached when it never needed one.
    bytes: Vec<u8>,
    /// Each frame's resume cursor and the client's size once it applied it.
    frames: Vec<(Cursor, (u16, u16))>,
    /// Resizes received, by rseq.
    resizes: Vec<(u64, u16, u16)>,
    /// Every message, as the stream's own record of what came.
    log: Vec<String>,
    resyncs: Vec<String>,
    exits: Vec<i64>,
    next_rpc: u64,
    /// The session it attaches.
    session: String,
}

impl Client {
    fn new(v: &Vornd) -> Client {
        Client {
            conn: v.engine.streams().connect(),
            engine: Arc::clone(&v.engine),
            term: None,
            cursor: None,
            bytes: Vec::new(),
            frames: Vec::new(),
            resizes: Vec::new(),
            log: Vec::new(),
            resyncs: Vec::new(),
            exits: Vec::new(),
            next_rpc: 0,
            session: SESSION.into(),
        }
    }

    /// The same client after a reconnect: a new connection, its screen and
    /// cursor kept.
    fn reconnect(mut self, v: &Vornd) -> Client {
        let fresh = Client::new(v);
        self.conn = fresh.conn;
        self.engine = fresh.engine;
        self
    }

    /// Sends a JSON-RPC call as a client would, through the router.
    fn call(&mut self, method: &str, params: Value) -> u64 {
        self.next_rpc += 1;
        let text =
            json!({ "jsonrpc": "2.0", "id": self.next_rpc, "method": method, "params": params })
                .to_string();
        let handled = vornd::terminal::handle(
            &self.engine,
            self.conn.id(),
            &self.conn.forwarder(),
            &text,
            false,
        );
        assert!(handled, "{method} was not answered by vornd");
        self.next_rpc
    }

    /// Attaches, resuming from `cursor` when given, and waits for the
    /// answer.
    async fn attach(&mut self, cursor: Option<Cursor>) -> Answer {
        let mut params = json!({ "id": self.session });
        if let Some(c) = cursor {
            params["cursor"] =
                json!({ "epoch": c.epoch, "nextRseq": c.next_rseq, "nextOffset": c.next_offset });
        }
        let rpc = self.call("terminal:attach", params);
        let result = self.answer(rpc).await;
        let cursor = cursor_of(&result["cursor"]);
        let continued = result["continued"].as_bool().unwrap();
        if !continued {
            let (cols, rows) = (
                result["cols"].as_u64().unwrap() as u32,
                result["rows"].as_u64().unwrap() as u32,
            );
            let mut em = Emulator::with_scrollback(cols, rows, SCROLLBACK).unwrap();
            em.feed(result["data"].as_str().unwrap().as_bytes(), &mut Vec::new());
            self.term = Some(em);
            self.bytes.clear();
        }
        self.cursor = Some(cursor);
        Answer {
            continued,
            cursor,
            resync: result["resync"].as_str().map(str::to_owned),
        }
    }

    /// Reads until the answer to `rpc`, applying what comes before it.
    async fn answer(&mut self, rpc: u64) -> Value {
        let t = Instant::now();
        loop {
            let left = PATIENCE.saturating_sub(t.elapsed());
            let o = tokio::time::timeout(left, self.conn.next())
                .await
                .expect("no answer in time")
                .expect("connection open");
            self.conn.written(o.size());
            if let Message::Text(t) = &o.msg {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["id"] == json!(rpc) {
                    assert!(v.get("error").is_none(), "{v}");
                    return v["result"].clone();
                }
            }
            self.apply(o.msg);
        }
    }

    fn apply(&mut self, msg: Message) {
        match msg {
            Message::Binary(b) => {
                let f = BytesFrame::decode(&b).expect("a v2 frame");
                let at = self.cursor.expect("bytes after an attach");
                assert_eq!(
                    (f.first_rseq, f.start_offset),
                    (at.next_rseq, at.next_offset),
                    "a frame starts at the client's cursor"
                );
                self.term
                    .as_mut()
                    .expect("a terminal")
                    .feed(f.data, &mut Vec::new());
                self.bytes.extend_from_slice(f.data);
                self.cursor = Some(f.resume());
                let em = self.term.as_ref().unwrap();
                self.frames.push((f.resume(), (em.cols(), em.rows())));
                self.log.push(format!(
                    "bytes {}..={} @{} {}",
                    f.first_rseq,
                    f.last_rseq,
                    f.start_offset,
                    f.data.len()
                ));
            }
            Message::Text(t) => {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                let p = &v["params"];
                match v["method"].as_str() {
                    Some("terminal:resized") => {
                        let rseq = p["rseq"].as_u64().unwrap();
                        let (cols, rows) =
                            (p["cols"].as_u64().unwrap(), p["rows"].as_u64().unwrap());
                        let at = self.cursor.as_mut().unwrap();
                        assert_eq!(rseq, at.next_rseq, "a resize at its place");
                        at.next_rseq += 1;
                        self.term
                            .as_mut()
                            .unwrap()
                            .resize(cols as u32, rows as u32, &mut Vec::new())
                            .unwrap();
                        self.resizes.push((rseq, cols as u16, rows as u16));
                        self.log.push(format!("resized {rseq} {cols}x{rows}"));
                    }
                    Some("terminal:resync") => {
                        self.resyncs.push(p["reason"].as_str().unwrap().to_owned())
                    }
                    Some("terminal:exit") => self.exits.push(p["exitCode"].as_i64().unwrap()),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Reads until the client's cursor reaches `head`.
    async fn follow_to(&mut self, head: Cursor) {
        let t = Instant::now();
        while self.cursor != Some(head) {
            let left = PATIENCE.saturating_sub(t.elapsed());
            let Ok(Some(o)) = tokio::time::timeout(left, self.conn.next()).await else {
                panic!(
                    "stopped at {:?}, wanted {head:?}: {:?}",
                    self.cursor, self.resyncs
                );
            };
            self.conn.written(o.size());
            self.apply(o.msg);
        }
    }

    fn text(&self) -> String {
        text(self.term.as_ref().unwrap())
    }
}

fn cursor_of(v: &Value) -> Cursor {
    Cursor {
        epoch: v["epoch"].as_u64().unwrap() as u32,
        next_rseq: v["nextRseq"].as_u64().unwrap(),
        next_offset: v["nextOffset"].as_u64().unwrap(),
    }
}

/// A terminal's text, scrollback included, through the accepted
/// differences below.
fn text(em: &Emulator) -> String {
    let opts = FormatterOptions::new().with_format(Format::Plain);
    let b = Formatter::new(em.terminal(), opts)
        .and_then(|mut f| f.format_alloc(None))
        .unwrap();
    blank_cells_as_spaces(&String::from_utf8_lossy(&b))
}

/// Accepted difference: a cell erased under a background colour is a blank
/// cell with that colour in the session's terminal, and the snapshot draws it
/// as a space in that colour, which the client then holds as written text.
/// Both look the same; the plain text differs only by trailing spaces.
fn blank_cells_as_spaces(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Asserts two terminals' texts are the same, naming the first line that
/// differs.
#[track_caller]
fn same_text(got: &str, want: &str, what: &str) {
    if got == want {
        return;
    }
    let (g, w): (Vec<_>, Vec<_>) = (got.lines().collect(), want.lines().collect());
    let n = (0..g.len().max(w.len()))
        .find(|&i| g.get(i) != w.get(i))
        .unwrap_or(0);
    let around = |v: &[&str]| v[n.saturating_sub(2)..(n + 3).min(v.len())].join("\n");
    panic!(
        "{what}: line {n} of {} vs {} differs\n--- client\n{}\n--- reference\n{}",
        g.len(),
        w.len(),
        around(&g),
        around(&w)
    );
}

/// A terminal fed the records before `upto` directly: what every client
/// must show at that cursor.
fn reference(size: (u16, u16), log: &[Entry], upto: Cursor) -> Emulator {
    let mut em =
        Emulator::with_scrollback(u32::from(size.0), u32::from(size.1), SCROLLBACK).unwrap();
    for e in log.iter().filter(|e| upto.includes(&e.hdr)) {
        match &e.rec {
            Record::Data { bytes, .. } => em.feed(bytes, &mut Vec::new()),
            &Record::Resize { cols, rows, .. } => {
                em.resize(u32::from(cols), u32::from(rows), &mut Vec::new())
                    .unwrap();
            }
            _ => {}
        }
    }
    em
}

/// The data bytes of the records from `from` up to `upto`.
fn bytes_between(log: &[Entry], from: Cursor, upto: Cursor) -> Vec<u8> {
    log.iter()
        .filter(|e| !from.includes(&e.hdr) && upto.includes(&e.hdr))
        .flat_map(|e| match &e.rec {
            Record::Data { bytes, .. } => bytes.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn lines(n: std::ops::Range<u32>) -> Vec<u8> {
    n.flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect()
}

fn data(bytes: &[u8]) -> Record {
    Record::Data {
        stream: Stream::Pty,
        bytes: bytes.to_vec(),
    }
}

fn resize(cols: u16, rows: u16) -> Record {
    Record::Resize {
        cols,
        rows,
        px_w: 0,
        px_h: 0,
        req: None,
    }
}

// ---------------------------------------------------------------------------

/// RC-T10 and TP-T24's bytes half: a client with cursor {R, N} survives a
/// vornd restart and receives exactly records R.. and bytes N.. with no
/// snapshot, from the new vornd's tail (R after the newest checkpoint) or
/// from sessiond's ring (R before it), resizes in their original order.
/// Killed again with its cursor trimmed from the ring, it gets NotRetained
/// and a snapshot, and converges to the reference.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_raw_client_continues_across_vornd_restarts() {
    for behind_checkpoint in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        // Content a checkpoint can carry, so vornd cuts them as it goes.
        let first = Generator::log(
            10,
            Profile::round_trip()
                .bytes(32 << 10)
                .size(Size::new(80, 24)),
        );
        let fake = Fake::start(dir.path(), (80, 24), first.entries.clone()).await;
        let v = Vornd::start(&fake, no_cadence());
        v.caught_up(&fake).await;
        let mut c = Client::new(&v);
        let a = c.attach(None).await;
        assert!(!a.continued);
        fake.push([
            data(&lines(0..50)),
            resize(100, 30),
            resize(90, 20),
            data(&lines(50..60)),
        ]);
        c.follow_to(fake.head()).await;
        let at = c.cursor.unwrap();

        if behind_checkpoint {
            // vornd moves on and checkpoints past the client before it dies.
            fake.push([data(&lines(60..70)), resize(70, 20)]);
            v.checkpoint(&fake).await;
            let cp = fake.held().newest.as_ref().unwrap().resume;
            assert!(cp.next_rseq > at.next_rseq, "{cp:?} {at:?}");
        }
        v.kill().await;
        // Output while no vornd is there.
        fake.push([data(&lines(70..80)), resize(120, 40), data(&lines(80..90))]);

        let v = Vornd::start(&fake, no_cadence());
        v.caught_up(&fake).await;
        let mut c = c.reconnect(&v);
        let a = c.attach(Some(at)).await;
        assert_eq!(
            a,
            Answer {
                continued: true,
                cursor: at,
                resync: None
            },
            "behind the checkpoint: {behind_checkpoint}"
        );
        let mark = c.bytes.len();
        let head = fake.head();
        c.follow_to(head).await;
        let log = fake.held().log.clone();
        assert_eq!(
            c.bytes[mark..],
            bytes_between(&log, at, head)[..],
            "exactly bytes N.."
        );
        let want_resizes: Vec<_> = log
            .iter()
            .filter(|e| !at.includes(&e.hdr))
            .filter_map(|e| match e.rec {
                Record::Resize { cols, rows, .. } => Some((e.hdr.rseq, cols, rows)),
                _ => None,
            })
            .collect();
        let got: Vec<_> = c
            .resizes
            .iter()
            .copied()
            .filter(|(r, _, _)| *r >= at.next_rseq)
            .collect();
        assert_eq!(got, want_resizes, "resizes in their original order");
        same_text(&c.text(), &text(&reference((80, 24), &log, head)), "client");

        // Again, with the client's cursor trimmed away.
        let at = c.cursor.unwrap();
        fake.data(&lines(90..95));
        v.checkpoint_twice(&fake).await;
        fake.trim();
        v.kill().await;
        fake.data(&lines(95..99));
        let v = Vornd::start(&fake, no_cadence());
        v.caught_up(&fake).await;
        let mut c = c.reconnect(&v);
        let a = c.attach(Some(at)).await;
        assert!(!a.continued);
        assert_eq!(a.resync.as_deref(), Some("notRetained"));
        assert_eq!(
            a.cursor,
            fake.head(),
            "the snapshot is cut at the actor's cursor"
        );
        let log = fake.held().log.clone();
        same_text(
            &c.text(),
            &text(&reference((80, 24), &log, fake.head())),
            "client",
        );
        v.kill().await;
    }
}

/// RC-T17's client half and TP-T23: record rseq 7 at offset 100 holding 20
/// bytes; a snapshot cut right after it says {8, 120} and the client's next
/// byte is 120. Over the whole stream, each byte arrives once. Repeated with
/// data records of 0, 1 and 64 KiB, and with the cut directly before a
/// resize, between two resizes at one offset, and after them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_snapshot_cut_after_a_record_continues_at_its_last_byte() {
    for len in [20usize, 0, 1, 64 << 10] {
        for extra in 0..3usize {
            let dir = tempfile::tempdir().unwrap();
            let mut b = LogBuilder::new(Size::new(80, 24));
            // Records 0..=6 hold 100 bytes; record 7 holds `len`.
            b.data(vec![b'a'; 40]);
            for _ in 0..5 {
                b.data(vec![b'b'; 12]);
            }
            b.data(vec![b'\n'; 0]);
            b.data(vec![b'c'; len]);
            // Then up to two resizes at the same offset.
            for i in 0..extra {
                b.resize(Size::new(70 + i as u16, 20));
            }
            let built = b.build();
            assert_eq!(built.entries[7].hdr.start_offset, 100);
            let fake = Fake::start(dir.path(), (80, 24), built.entries.clone()).await;
            let v = Vornd::start(&fake, no_cadence());
            v.caught_up(&fake).await;
            let mut c = Client::new(&v);
            let a = c.attach(None).await;
            assert_eq!(
                a.cursor,
                Cursor {
                    epoch: 0,
                    next_rseq: 8 + extra as u64,
                    next_offset: 100 + len as u64,
                }
            );
            let snap_at = a.cursor;
            fake.push([
                resize(60, 20),
                data(b"after the cut"),
                resize(50, 10),
                data(b"!"),
            ]);
            c.follow_to(fake.head()).await;
            let log = fake.held().log.clone();
            assert_eq!(c.bytes, bytes_between(&log, snap_at, fake.head()));
            assert_eq!(&c.bytes[..3], b"aft", "byte {} is next", 100 + len);
            same_text(
                &c.text(),
                &text(&reference((80, 24), &log, fake.head())),
                "client",
            );
            v.kill().await;
        }
    }
}

/// RC-T21, the raw-client rows: vornd alive or restarted, times a cursor
/// that is retained, trimmed, of the wrong epoch, or spanning a gap. Each
/// cell continues or gets a snapshot exactly as flow G says.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_reconnect_matrix_for_raw_clients() {
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Case {
        Retained,
        Trimmed,
        WrongEpoch,
        Gap,
    }
    for restarted in [false, true] {
        for case in [Case::Retained, Case::Trimmed, Case::WrongEpoch, Case::Gap] {
            let dir = tempfile::tempdir().unwrap();
            let log = Generator::log(21, Profile::shell().bytes(16 << 10));
            let fake = Fake::start(dir.path(), (log.size.cols, log.size.rows), log.entries).await;
            let mut v = Vornd::start(&fake, no_cadence());
            v.caught_up(&fake).await;
            let mut c = Client::new(&v);
            c.attach(None).await;
            c.follow_to(fake.head()).await;
            let mut at = c.cursor.unwrap();
            fake.data(&lines(0..5));
            match case {
                Case::Retained => {}
                // Past what vornd keeps too: more than its tail holds.
                Case::Trimmed => {
                    for _ in 0..80 {
                        fake.data(&lines(0..8000)[..64 << 10]);
                    }
                    v.checkpoint_twice(&fake).await;
                    fake.trim();
                }
                Case::WrongEpoch => at.epoch += 1,
                Case::Gap => fake.push([Record::Gap {
                    lost_bytes: 77,
                    reason: GapReason::SpoolFull,
                }]),
            }
            fake.data(&lines(5..9));
            if restarted {
                v.kill().await;
                v = Vornd::start(&fake, no_cadence());
            }
            v.caught_up(&fake).await;
            let mut c = c.reconnect(&v);
            let a = c.attach(Some(at)).await;
            let expect = match case {
                Case::Retained => None,
                Case::Trimmed => Some("notRetained"),
                Case::WrongEpoch => Some("wrongEpoch"),
                Case::Gap => Some("gap"),
            };
            assert_eq!(
                a.continued,
                expect.is_none(),
                "{case:?} restarted {restarted}"
            );
            assert_eq!(
                a.resync.as_deref(),
                expect,
                "{case:?} restarted {restarted}"
            );
            c.follow_to(fake.head()).await;
            if case != Case::Gap {
                let log = fake.held().log.clone();
                let size = fake.held().size;
                same_text(
                    &c.text(),
                    &text(&reference(size, &log, fake.head())),
                    "{case:?}",
                );
            }
            v.kill().await;
        }
    }
}

/// TP-T24: a client with no cursor (a version 1 client has only an offset,
/// which is never a resume token) always gets a snapshot, and converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_without_a_cursor_always_gets_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let log = Generator::log(24, Profile::mixed().bytes(64 << 10));
    let size = (log.size.cols, log.size.rows);
    let fake = Fake::start(dir.path(), size, log.entries).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    for _ in 0..2 {
        let mut c = Client::new(&v);
        let a = c.attach(None).await;
        assert!(!a.continued);
        assert_eq!(a.resync, None);
        assert_eq!(a.cursor, fake.head());
    }
    v.kill().await;
}

/// TP-T7: a bytes client that stops reading. Its queue overflows past the
/// cap, the bytes queued for it are dropped and it is told to resync, the
/// session keeps applying at its own pace, and a client that reads keeps
/// getting every byte. The slow one resyncs to a snapshot and converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_bytes_client_resyncs_and_slows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::start(dir.path(), (100, 30), Vec::new()).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut slow = Client::new(&v);
    let mut fast = Client::new(&v);
    slow.attach(None).await;
    fast.attach(None).await;
    // 8 MiB in 64 KiB records, read as it comes by the fast client and
    // never by the slow one.
    let record = lines(0..8000);
    let record = &record[..64 << 10];
    let t = Instant::now();
    for _ in 0..128 {
        fake.data(record);
        fast.follow_to(fake.head()).await;
    }
    v.caught_up(&fake).await;
    assert!(
        t.elapsed() < Duration::from_secs(10),
        "the session kept its pace"
    );
    assert!(fast.resyncs.is_empty(), "{:?}", fast.resyncs);
    assert!(
        slow.conn.queued() <= QUEUE_CAP + (64 << 10) + 4096,
        "vornd holds at most the cap for it: {}",
        slow.conn.queued()
    );
    // What it reads now: whatever fit before the cap, then the resync.
    let got = slow.conn.drain_now();
    let resync = got
        .iter()
        .any(|m| matches!(m, Message::Text(t) if t.as_str().contains("\"reason\":\"overflow\"")));
    assert!(resync, "told to resync");
    let a = slow.attach(None).await;
    assert!(!a.continued);
    slow.follow_to(fake.head()).await;
    same_text(&slow.text(), &fast.text(), "slow client");
    v.kill().await;
}

/// TP-T8 for bytes clients: 100 resizes during marker output. Every frame
/// the client applies finds it at the size of the last resize record before
/// the frame's resume cursor, and a client replaying the same records from
/// sessiond's ring receives the identical stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resizes_reach_bytes_clients_in_record_order() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::start(dir.path(), (80, 24), Vec::new()).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut live = Client::new(&v);
    let start = live.attach(None).await.cursor;
    for i in 0..100u32 {
        let (cols, rows) = (40 + (i * 7 % 80) as u16, 10 + (i * 3 % 30) as u16);
        fake.push([
            data(format!("marker {i} ").repeat(5).as_bytes()),
            resize(cols, rows),
        ]);
    }
    fake.data(b"done");
    let head = fake.head();
    live.follow_to(head).await;
    let log = fake.held().log.clone();
    for (resume, size) in &live.frames {
        let last = log
            .iter()
            .rev()
            .filter(|e| resume.includes(&e.hdr))
            .find_map(|e| match e.rec {
                Record::Resize { cols, rows, .. } => Some((cols, rows)),
                _ => None,
            })
            .unwrap_or((80, 24));
        assert_eq!(*size, last, "frame ending at {resume:?}");
    }
    assert_eq!(live.resizes.len(), 100);
    // The same records again, from the ring.
    let mut replay = Client::new(&v);
    replay.term = Some(Emulator::with_scrollback(80, 24, SCROLLBACK).unwrap());
    replay.cursor = Some(start);
    let rpc = replay.call(
        "terminal:attach",
        json!({ "id": SESSION, "cursor": { "epoch": 0, "nextRseq": 0, "nextOffset": 0 } }),
    );
    // The answer comes once sessiond has sent the records again.
    let answer = replay.answer(rpc).await;
    assert_eq!(answer["continued"], true);
    replay.follow_to(head).await;
    assert_eq!(replay.log, live.log, "identical frames");
    same_text(&replay.text(), &live.text(), "replay");
    v.kill().await;
}

/// TP-T12, vornd's half: three clients attached, and a program asking DA1,
/// DSR 6, DECRQM and OSC 11. vornd's actor answers each once, live; the
/// clients answer none (their half is the renderer's query handlers).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_query_gets_exactly_one_reply() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::start(dir.path(), (80, 24), Vec::new()).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut clients = vec![Client::new(&v), Client::new(&v), Client::new(&v)];
    for c in &mut clients {
        c.attach(None).await;
    }
    let queries: [&[u8]; 4] = [b"\x1b[c", b"\x1b[6n", b"\x1b[?2004$p", b"\x1b]11;?\x1b\\"];
    for q in queries {
        let before = fake.held().writes.len();
        fake.data(q);
        v.caught_up(&fake).await;
        for c in &mut clients {
            c.follow_to(fake.head()).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let writes = fake.held().writes[before..].to_vec();
        assert_eq!(
            writes.len(),
            1,
            "{:?}: {writes:?}",
            String::from_utf8_lossy(q)
        );
    }
    v.kill().await;
}

/// TP-T19, the bytes part: a client attaching while a full-screen program
/// has the alternate screen sees that screen, and the primary screen's
/// history is not in its snapshot. That is the known gap of TP §14, asserted
/// here until a primary-screen formatter closes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_alternate_screen_gap_is_known() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = LogBuilder::new(Size::new(80, 24));
    b.data(lines(0..40));
    b.data(b"\x1b[?1049h\x1b[2J\x1b[Hfull screen program".to_vec());
    let fake = Fake::start(dir.path(), (80, 24), b.build().entries).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut c = Client::new(&v);
    c.attach(None).await;
    assert!(c.text().contains("full screen program"), "{}", c.text());
    fake.data(b"\x1b[?1049l");
    c.follow_to(fake.head()).await;
    let log = fake.held().log.clone();
    let reference = text(&reference((80, 24), &log, fake.head()));
    assert!(reference.contains("line 39"), "{reference}");
    // The known gap: the client never received the primary screen.
    assert!(!c.text().contains("line 39"), "{}", c.text());
    v.kill().await;
}

/// The other calls for a held session: write reaches the program, resize
/// becomes a record every client sees, readOutput and readScrollback answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn write_resize_and_reads_are_answered_by_vornd() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::start(dir.path(), (80, 24), Vec::new()).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut c = Client::new(&v);
    c.attach(None).await;
    c.call("terminal:write", json!({ "id": SESSION, "data": "ls\r" }));
    c.call(
        "terminal:resize",
        json!({ "id": SESSION, "cols": 100, "rows": 40 }),
    );
    fake.data(b"hello world\r\nsecond line\r\n");
    let t = Instant::now();
    while fake.held().log.len() < 2 || c.resizes.is_empty() {
        assert!(t.elapsed() < PATIENCE);
        c.follow_to(fake.head()).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(fake.held().writes.iter().any(|w| w == b"ls\r"));
    assert_eq!((c.resizes[0].1, c.resizes[0].2), (100, 40));
    let rpc = c.call("terminal:readOutput", json!({ "id": SESSION, "lines": 5 }));
    let out = c.answer(rpc).await;
    assert!(out.to_string().contains("second line"), "{out}");
    let rpc = c.call("terminal:readScrollback", json!({ "id": SESSION }));
    let sb = c.answer(rpc).await;
    assert!(sb["data"].as_str().unwrap().contains("hello world"), "{sb}");
    // A session nothing holds has no screen and is not live.
    let rpc = c.call("terminal:attach", json!({ "id": "nodes" }));
    assert_eq!(
        c.answer(rpc).await,
        json!({ "data": "", "seq": 0, "live": false })
    );
    // Writing to it is still the server's.
    let text = json!({ "jsonrpc": "2.0", "id": 99, "method": "terminal:write", "params": { "id": "nodes", "data": "x" } })
        .to_string();
    assert!(!vornd::terminal::handle(
        &v.engine,
        c.conn.id(),
        &c.conn.forwarder(),
        &text,
        false
    ));
    v.kill().await;
}

/// A client that dropped while megabytes were printed resumes from vornd's
/// tail without a snapshot and without overflowing its own catch-up: the
/// tail is sent as the connection drains, not all at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_far_behind_catches_up_from_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::start(dir.path(), (100, 30), Vec::new()).await;
    let v = Vornd::start(&fake, no_cadence());
    v.caught_up(&fake).await;
    let mut c = Client::new(&v);
    c.attach(None).await;
    fake.data(b"start\r\n");
    c.follow_to(fake.head()).await;
    let at = c.cursor.unwrap();
    // Gone while 3 MiB arrive: more than a connection may queue, less than
    // the tail holds.
    let record = lines(0..8000);
    for _ in 0..48 {
        fake.data(&record[..64 << 10]);
    }
    v.caught_up(&fake).await;
    let mut c = c.reconnect(&v);
    let a = c.attach(Some(at)).await;
    assert!(a.continued, "{a:?}");
    assert!(
        c.conn.queued() <= QUEUE_CAP + (320 << 10),
        "queued {} at once",
        c.conn.queued()
    );
    let mark = c.bytes.len();
    c.follow_to(fake.head()).await;
    assert!(c.resyncs.is_empty(), "{:?}", c.resyncs);
    let log = fake.held().log.clone();
    assert_eq!(c.bytes[mark..], bytes_between(&log, at, fake.head())[..]);
    v.kill().await;
}

/// A client attaching while the session replays to its exit is answered:
/// with the live screen and then the exit, or with how it ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attach_racing_the_exit_is_answered() {
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let mut b = LogBuilder::new(Size::new(80, 24));
        for _ in 0..64 {
            b.data(lines(0..2000));
        }
        b.push(Record::Exit {
            code: Some(5),
            signal: None,
        });
        let fake = Fake::start(dir.path(), (80, 24), b.build().entries).await;
        let v = Vornd::start(&fake, no_cadence());
        let t = Instant::now();
        while !v.engine.streams().holds(SESSION) {
            assert!(t.elapsed() < PATIENCE);
            tokio::task::yield_now().await;
        }
        let mut c = Client::new(&v);
        let rpc = c.call("terminal:attach", json!({ "id": SESSION }));
        let answer = c.answer(rpc).await;
        if answer["live"] == false {
            assert_eq!(answer["exitCode"], 5, "{answer}");
        } else {
            c.cursor = Some(cursor_of(&answer["cursor"]));
            c.term = Some(Emulator::with_scrollback(80, 24, SCROLLBACK).unwrap());
            let t = Instant::now();
            while c.exits.is_empty() {
                assert!(t.elapsed() < PATIENCE, "no exit after {answer}");
                if let Ok(Some(o)) =
                    tokio::time::timeout(Duration::from_millis(100), c.conn.next()).await
                {
                    c.conn.written(o.size());
                    c.apply(o.msg);
                }
            }
            assert_eq!(c.exits, [5]);
        }
        v.kill().await;
    }
}

/// Against the real sessiond, which ends a session's stream to vornd when it
/// takes an attach: a client whose cursor sessiond has trimmed gets
/// NotRetained and a snapshot, and the session's output keeps reaching
/// vornd and every client afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_fetch_leaves_the_session_streaming() {
    let home = tempfile::tempdir().unwrap();
    let d = Sessiond::new(server::Config {
        home: home.path().to_path_buf(),
        instance: 0xf7c4,
        build: "test".into(),
        idle_exit: Duration::from_secs(600),
        spool_cap: 64 << 20,
    });
    let listener = server::bind(&d).unwrap();
    let _serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
    // Checkpoints every few KiB, so sessiond trims behind the client fast.
    let cadence = Cadence {
        bytes: 2 << 10,
        ..no_cadence()
    };
    let v = Vornd::start_at(d.endpoint(), cadence);
    let spec = SpawnSpec {
        argv: [
            "sh",
            "-c",
            "while read l; do i=0; while [ $i -lt 200 ]; do echo \"$l $i ................................\"; i=$((i+1)); done; done",
        ]
        .map(String::from)
        .to_vec(),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: Vec::new(),
        io: Io::Pty { cols: 80, rows: 24 },
        ring_bytes: None,
    };
    let t = Instant::now();
    let id = loop {
        match v.engine.spawn(spec.clone()).await {
            Ok(id) => break id,
            Err(_) if t.elapsed() < PATIENCE => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(e) => panic!("spawn: {e}"),
        }
    };
    let mut c = Client::new(&v);
    c.session = id.clone();
    c.attach(None).await;
    let shows = |c: &Client, s: &str| c.term.as_ref().is_some_and(|_| c.text().contains(s));
    async fn until(c: &mut Client, what: &str) {
        let t = Instant::now();
        while !c.text().contains(what) {
            assert!(t.elapsed() < PATIENCE, "never saw {what}: {}", c.text());
            if let Ok(Some(o)) =
                tokio::time::timeout(Duration::from_millis(100), c.conn.next()).await
            {
                c.conn.written(o.size());
                c.apply(o.msg);
            }
        }
    }
    c.call("terminal:write", json!({ "id": id, "data": "a\r" }));
    until(&mut c, "a 199").await;
    let old = c.cursor.unwrap();
    for word in ["b", "c", "d"] {
        c.call(
            "terminal:write",
            json!({ "id": id, "data": format!("{word}\r") }),
        );
        until(&mut c, &format!("{word} 199")).await;
    }
    assert!(!shows(&c, "e 199"));
    v.kill().await;

    let v = Vornd::start_at(d.endpoint(), cadence);
    let t = Instant::now();
    while !v.engine.report()["sessions"]
        .as_array()
        .is_some_and(|s| s.iter().any(|s| s["state"] == "live"))
    {
        assert!(t.elapsed() < PATIENCE, "{}", v.engine.report());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut c = c.reconnect(&v);
    let a = c.attach(Some(old)).await;
    assert_eq!(a.resync.as_deref(), Some("notRetained"), "{a:?}");
    // Still streaming: new output reaches vornd and the client.
    c.call("terminal:write", json!({ "id": id, "data": "e\r" }));
    until(&mut c, "e 199").await;
    v.kill().await;
}

/// The same resume while vornd is still applying the output: the hub holds
/// only part of it when the client attaches, and the rest arrives as one
/// batch larger than a connection may queue. It follows from the tail as the
/// connection drains, never an overflow, however the two interleave.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resume_during_a_burst_never_overflows() {
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::start(dir.path(), (100, 30), Vec::new()).await;
        let v = Vornd::start(&fake, no_cadence());
        v.caught_up(&fake).await;
        let mut c = Client::new(&v);
        c.attach(None).await;
        fake.data(b"start\r\n");
        c.follow_to(fake.head()).await;
        let at = c.cursor.unwrap();
        let record = lines(0..8000);
        for _ in 0..48 {
            fake.data(&record[..64 << 10]);
        }
        let mut c = c.reconnect(&v);
        let a = c.attach(Some(at)).await;
        assert!(a.continued, "{a:?}");
        let mark = c.bytes.len();
        c.follow_to(fake.head()).await;
        assert!(c.resyncs.is_empty(), "{:?}", c.resyncs);
        let log = fake.held().log.clone();
        assert_eq!(c.bytes[mark..], bytes_between(&log, at, fake.head())[..]);
        v.kill().await;
    }
}
