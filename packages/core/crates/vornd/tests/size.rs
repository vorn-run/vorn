//! The size rule end to end, on a real sessiond: a desktop grid client on
//! the local socket and a phone as a bytes client on the WebSocket's router,
//! both on one session. Looking never resizes it, typing resizes it once,
//! and locking the phone gives the desktop its size back (TP §10, T9 to
//! T9e). Every resize is a record: both clients see it at its place, with
//! who asked for it and why, and both draw the session's whole grid.

#![cfg(all(feature = "engine", unix))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_tungstenite::tungstenite::Message;
use vorn_engine::{Config, State};
use vorn_grid_client::{Client, Got};
use vorn_screen::Emulator;
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{Io, SpawnSpec};
use vorn_term_proto::bytes::BytesFrame;
use vorn_term_proto::msg::{
    caps, Attach, AttachMode, ClientKind, ClientMsg, Hello, InputEvent, ResizeReason, ServerMsg,
    Size, PROTO_MAJOR,
};
use vorn_term_proto::Cursor;
use vornd::engine::Engine;
use vornd::holder::{self, Holder};
use vornd::streams::ClientConn;

const PATIENCE: Duration = Duration::from_secs(30);
/// Longer than the rule's settle time, so a resize it would make is made.
const SETTLED: Duration = Duration::from_millis(600);

const LAUNCH: (u16, u16) = (100, 30);
const DESKTOP: (u16, u16) = (120, 40);
const PHONE: (u16, u16) = (50, 30);

struct Rig {
    _d: Arc<Sessiond>,
    _home: tempfile::TempDir,
    engine: Arc<Engine>,
    task: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0x512e,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        tokio::spawn(server::serve(Arc::clone(&d), listener));
        let engine = Engine::new(Config {
            scrollback: 1 << 20,
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = d.endpoint();
        let task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        Rig {
            _d: d,
            _home: home,
            engine,
            task,
        }
    }

    /// A session as a workflow would launch it: `cat`, at [`LAUNCH`].
    async fn spawn(&self) -> String {
        let t = Instant::now();
        let id = loop {
            let spec = SpawnSpec {
                argv: vec!["cat".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io: Io::Pty {
                    cols: LAUNCH.0,
                    rows: LAUNCH.1,
                },
                ring_bytes: None,
            };
            match self.engine.spawn(spec).await {
                Ok(id) => break id,
                Err(_) if t.elapsed() < PATIENCE => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("spawn: {e}"),
            }
        };
        loop {
            let live = self
                .engine
                .sessions()
                .await
                .iter()
                .any(|s| s.brief.session == id && s.brief.state == State::Live);
            if live {
                return id;
            }
            assert!(t.elapsed() < PATIENCE, "{id} never went live");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The session's size and screen as the engine has them.
    async fn engine_view(&self, id: &str) -> ((u16, u16), String) {
        let s = self
            .engine
            .sessions()
            .await
            .into_iter()
            .find(|s| s.brief.session == id)
            .expect("the session");
        ((s.brief.cols, s.brief.rows), s.screen)
    }

    /// A grid client on the local socket: the desktop.
    fn desktop(&self) -> Desktop {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        let engine = Arc::clone(&self.engine);
        tokio::spawn(async move { vornd::grid::serve_conn(theirs, engine, "test", 1).await });
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(Hello {
            proto_major: PROTO_MAJOR,
            proto_minor: 0,
            caps: caps::ALL,
            client: ClientKind::Native,
            build: "test".into(),
        }));
        Desktop {
            client,
            io: ours,
            got: Vec::new(),
            sid: 0,
        }
    }

    /// A bytes client on a connection without the desktop's token: the
    /// phone.
    fn phone(&self) -> Phone {
        Phone {
            conn: self.engine.streams().connect(),
            engine: Arc::clone(&self.engine),
            term: None,
            cursor: None,
            resized: Vec::new(),
            next_rpc: 0,
            name: String::new(),
        }
    }
}

/// A `Resized` as the desktop got it: size, owner, reason.
type Named = ((u16, u16), Option<u32>, Option<ResizeReason>);

struct Desktop {
    client: Client,
    io: DuplexStream,
    got: Vec<Got>,
    sid: u32,
}

impl Desktop {
    /// Reads what has come within `wait`, acknowledging every frame.
    async fn read_for(&mut self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let out = self.client.take_out();
            if !out.is_empty() {
                self.io.write_all(&out).await.unwrap();
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            let n = match tokio::time::timeout(left, self.io.read(&mut buf)).await {
                Err(_) => return,
                Ok(Ok(0)) | Ok(Err(_)) => panic!("the grid connection closed"),
                Ok(Ok(n)) => n,
            };
            for g in self.client.receive(&buf[..n]).unwrap() {
                if let Got::Frame { sid, rev, .. } = g {
                    self.client.ack(sid, rev);
                }
                assert!(!matches!(g, Got::Refused { .. }), "{g:?}");
                self.got.push(g);
            }
        }
    }

    async fn until(&mut self, what: &str, done: impl Fn(&Desktop) -> bool) {
        let t = Instant::now();
        while !done(self) {
            assert!(t.elapsed() < PATIENCE, "{what}: {:?}", self.got);
            self.read_for(Duration::from_millis(50)).await;
        }
    }

    async fn attach(&mut self, session: &str) {
        self.client.attach(Attach {
            session: session.to_owned(),
            mode: AttachMode::Grid,
            view: Size {
                cols: DESKTOP.0,
                rows: DESKTOP.1,
                px_w: 0,
                px_h: 0,
            },
            visible: true,
            resume: None,
            history_tail: 0,
        });
        self.until("attached", |d| {
            d.got.iter().any(|g| matches!(g, Got::Attached(_)))
        })
        .await;
        self.sid = self
            .got
            .iter()
            .find_map(|g| match g {
                Got::Attached(a) => Some(a.sid),
                _ => None,
            })
            .unwrap();
        let sid = self.sid;
        self.until("the snapshot", |d| {
            d.client.pane(sid).and_then(|p| p.mirror()).is_some()
        })
        .await;
    }

    fn send(&mut self, m: ClientMsg) {
        self.client.send(&m);
    }

    fn type_text(&mut self, text: &str) {
        let input_seq = self.client.input_seq();
        self.send(ClientMsg::Input {
            sid: self.sid,
            input_seq,
            event: InputEvent::Text {
                utf8: text.to_owned(),
            },
        });
    }

    /// Every `Resized` received: size, owner, reason.
    fn resized(&self) -> Vec<Named> {
        self.got
            .iter()
            .filter_map(|g| match g {
                Got::Message(ServerMsg::Resized {
                    cols,
                    rows,
                    owner,
                    reason,
                    ..
                }) => Some(((*cols, *rows), *owner, *reason)),
                _ => None,
            })
            .collect()
    }

    /// The grid the desktop draws: its size and text.
    fn grid(&self) -> ((u16, u16), String) {
        let m = self.client.pane(self.sid).unwrap().mirror().unwrap();
        (
            (m.term().cols, m.term().rows),
            trimmed(&m.text().join("\n")),
        )
    }
}

struct Phone {
    conn: ClientConn,
    engine: Arc<Engine>,
    term: Option<Emulator>,
    cursor: Option<Cursor>,
    resized: Vec<Value>,
    next_rpc: u64,
    /// Its name as vornd told it.
    name: String,
}

impl Phone {
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

    async fn attach(&mut self, session: &str) {
        let rpc = self.call("terminal:attach", json!({ "id": session }));
        let t = Instant::now();
        let result = loop {
            let o = tokio::time::timeout(PATIENCE.saturating_sub(t.elapsed()), self.conn.next())
                .await
                .expect("an answer")
                .expect("open");
            self.conn.written(o.size());
            if let Message::Text(t) = &o.msg {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["id"] == json!(rpc) {
                    break v["result"].clone();
                }
            }
        };
        let (cols, rows) = (
            result["cols"].as_u64().unwrap() as u32,
            result["rows"].as_u64().unwrap() as u32,
        );
        let mut em = Emulator::with_scrollback(cols, rows, 1 << 20).unwrap();
        em.feed(result["data"].as_str().unwrap().as_bytes(), &mut Vec::new());
        self.term = Some(em);
        let c = &result["cursor"];
        self.cursor = Some(Cursor {
            epoch: c["epoch"].as_u64().unwrap() as u32,
            next_rseq: c["nextRseq"].as_u64().unwrap(),
            next_offset: c["nextOffset"].as_u64().unwrap(),
        });
        self.name = result["client"].as_str().unwrap().to_owned();
    }

    /// Applies what comes within `wait`.
    async fn read_for(&mut self, wait: Duration) {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let Ok(Some(o)) = tokio::time::timeout(left, self.conn.next()).await else {
                return;
            };
            self.conn.written(o.size());
            match o.msg {
                Message::Binary(b) => {
                    let f = BytesFrame::decode(&b).unwrap();
                    self.term.as_mut().unwrap().feed(f.data, &mut Vec::new());
                    self.cursor = Some(f.resume());
                }
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    if v["method"] == "terminal:resized" {
                        let p = &v["params"];
                        let at = self.cursor.as_mut().unwrap();
                        assert_eq!(p["rseq"].as_u64(), Some(at.next_rseq), "at its place");
                        at.next_rseq += 1;
                        self.term
                            .as_mut()
                            .unwrap()
                            .resize(
                                p["cols"].as_u64().unwrap() as u32,
                                p["rows"].as_u64().unwrap() as u32,
                                &mut Vec::new(),
                            )
                            .unwrap();
                        self.resized.push(p.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// The grid the phone draws: its size and text.
    fn grid(&self) -> ((u16, u16), String) {
        let em = self.term.as_ref().unwrap();
        ((em.cols(), em.rows()), trimmed(&plain(em)))
    }
}

fn plain(em: &Emulator) -> String {
    let opts = FormatterOptions::new().with_format(Format::Plain);
    let b = Formatter::new(em.terminal(), opts)
        .and_then(|mut f| f.format_alloc(None))
        .unwrap();
    String::from_utf8_lossy(&b).into_owned()
}

/// Lines with trailing blanks and trailing blank lines dropped.
fn trimmed(s: &str) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim_end).collect();
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    lines[..end].join("\n")
}

/// TP-T9e end to end: both clients draw the session's exact grid, and the
/// text the engine has.
async fn both_draw_the_whole_grid(rig: &Rig, id: &str, desk: &mut Desktop, phone: &mut Phone) {
    let (size, screen) = rig.engine_view(id).await;
    let screen = trimmed(&screen);
    let t = Instant::now();
    loop {
        desk.read_for(Duration::from_millis(50)).await;
        phone.read_for(Duration::from_millis(50)).await;
        if desk.grid() == (size, screen.clone()) && phone.grid() == (size, screen.clone()) {
            return;
        }
        assert!(
            t.elapsed() < PATIENCE,
            "the engine has {size:?}:\n{screen}\n--- the desktop {:?}\n--- the phone {:?}",
            desk.grid(),
            phone.grid()
        );
    }
}

/// Reads both clients for `wait`.
async fn settle(desk: &mut Desktop, phone: &mut Phone, wait: Duration) {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        desk.read_for(Duration::from_millis(50)).await;
        phone.read_for(Duration::from_millis(50)).await;
    }
}

/// TP-T9 to T9e with both kinds of client: a quiet agent keeps its launch
/// size while a desktop and a phone attach and look; the desktop typing
/// resizes it once, to the desktop; the phone typing after three quiet
/// seconds takes it, once; the phone locked gives it back to the desktop
/// after the grace period, once. Both clients see each resize at its record
/// with its owner and reason, and both always draw the whole grid.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn looking_never_resizes_typing_resizes_once_and_locking_returns_it() {
    let rig = Rig::start().await;
    let id = rig.spawn().await;
    let mut desk = rig.desktop();
    desk.attach(&id).await;
    let mut phone = rig.phone();
    phone.attach(&id).await;
    phone.call(
        "terminal:viewport",
        json!({ "id": id, "cols": PHONE.0, "rows": PHONE.1 }),
    );
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;

    // Looking: the phone focuses, scrolls and backgrounds; the desktop
    // hides and shows its pane. Nobody types.
    for _ in 0..10 {
        for state in ["watching", "active", "away"] {
            phone.call("terminal:presence", json!({ "id": id, "state": state }));
        }
        phone.call(
            "terminal:write",
            json!({ "id": id, "data": "\u{1b}[I\u{1b}[<64;5;5M" }),
        );
        let sid = desk.sid;
        desk.send(ClientMsg::SetVisible {
            sid,
            visible: false,
        });
        desk.send(ClientMsg::SetVisible { sid, visible: true });
        desk.send(ClientMsg::Viewport {
            sid,
            size: Size {
                cols: DESKTOP.0 + 5,
                rows: DESKTOP.1,
                px_w: 0,
                px_h: 0,
            },
        });
        settle(&mut desk, &mut phone, Duration::from_millis(50)).await;
    }
    phone.call(
        "terminal:presence",
        json!({ "id": id, "state": "watching" }),
    );
    desk.send(ClientMsg::Viewport {
        sid: desk.sid,
        size: Size {
            cols: DESKTOP.0,
            rows: DESKTOP.1,
            px_w: 0,
            px_h: 0,
        },
    });
    settle(&mut desk, &mut phone, SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, LAUNCH, "looking resized");
    assert!(desk.resized().is_empty() && phone.resized.is_empty());
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;

    // The desktop types: one resize, to the desktop.
    for word in ["hello", "size"] {
        desk.type_text(&format!("{word}\r"));
        settle(&mut desk, &mut phone, Duration::from_millis(100)).await;
    }
    settle(&mut desk, &mut phone, SETTLED).await;
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;
    assert_eq!(rig.engine_view(&id).await.0, DESKTOP);
    let sid = desk.sid;
    assert_eq!(
        desk.resized(),
        [(DESKTOP, Some(sid), Some(ResizeReason::Input))]
    );
    assert_eq!(phone.resized.len(), 1);
    assert!(phone.resized[0]["owner"]
        .as_str()
        .is_some_and(|o| o.starts_with("grid:") && o.ends_with(&format!(":{sid}"))));
    assert_eq!(phone.resized[0]["reason"], "input");

    // The phone types while the desktop is in use: nothing. After three
    // quiet seconds: one resize, to the phone.
    phone.call("terminal:write", json!({ "id": id, "data": "early\r" }));
    settle(&mut desk, &mut phone, SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, DESKTOP);
    settle(&mut desk, &mut phone, Duration::from_millis(3_000)).await;
    phone.call("terminal:write", json!({ "id": id, "data": "phone\r" }));
    settle(&mut desk, &mut phone, SETTLED).await;
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;
    assert_eq!(rig.engine_view(&id).await.0, PHONE);
    assert_eq!(phone.resized.len(), 2);
    assert_eq!(phone.resized[1]["owner"], json!(phone.name));
    assert_eq!(phone.resized[1]["reason"], "input");
    assert_eq!(desk.resized()[1], (PHONE, None, Some(ResizeReason::Input)));

    // The phone is locked; the desktop is on screen. After the grace
    // period the size comes home, in one resize.
    phone.call("terminal:presence", json!({ "id": id, "state": "away" }));
    settle(&mut desk, &mut phone, Duration::from_millis(9_000)).await;
    assert_eq!(
        rig.engine_view(&id).await.0,
        PHONE,
        "not before the grace period"
    );
    settle(
        &mut desk,
        &mut phone,
        Duration::from_millis(1_000) + SETTLED,
    )
    .await;
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;
    assert_eq!(rig.engine_view(&id).await.0, DESKTOP);
    assert_eq!(
        desk.resized()[2..],
        [(DESKTOP, Some(sid), Some(ResizeReason::Returned))]
    );
    assert_eq!(phone.resized.len(), 3);
    assert_eq!(phone.resized[2]["reason"], "returned");

    rig.task.abort();
}

/// "Fit to this device" and the lock, from the phone: the size is the
/// phone's at once, and stays while the desktop types, until the phone's
/// connection goes; then it returns to the desktop.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lock_holds_against_typing_until_its_client_leaves() {
    let rig = Rig::start().await;
    let id = rig.spawn().await;
    let mut desk = rig.desktop();
    desk.attach(&id).await;
    let mut phone = rig.phone();
    phone.attach(&id).await;
    phone.call(
        "terminal:viewport",
        json!({ "id": id, "cols": PHONE.0, "rows": PHONE.1 }),
    );
    phone.call("terminal:lockSize", json!({ "id": id, "locked": true }));
    settle(&mut desk, &mut phone, SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, PHONE);
    assert_eq!(phone.resized[0]["reason"], "locked");
    desk.type_text("x");
    desk.send(ClientMsg::TakeSize { sid: desk.sid });
    settle(&mut desk, &mut phone, SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, PHONE, "the lock held");
    both_draw_the_whole_grid(&rig, &id, &mut desk, &mut phone).await;
    // The phone's connection goes; the size returns after the grace period.
    drop(phone);
    let t = Instant::now();
    while rig.engine_view(&id).await.0 != DESKTOP {
        assert!(t.elapsed() < PATIENCE, "the size never came home");
        desk.read_for(Duration::from_millis(100)).await;
    }
    assert!(t.elapsed() >= Duration::from_secs(9), "{:?}", t.elapsed());
    // The engine can change size a moment before the desktop reads the resize.
    let home = (DESKTOP, Some(desk.sid), Some(ResizeReason::Returned));
    desk.until("the resize home", |d| d.resized().last() == Some(&home))
        .await;
    rig.task.abort();
}

impl Phone {
    /// Reads until the answer to `rpc`, applying what comes before it.
    async fn answer(&mut self, rpc: u64) -> Value {
        let t = Instant::now();
        loop {
            let o = tokio::time::timeout(PATIENCE.saturating_sub(t.elapsed()), self.conn.next())
                .await
                .expect("an answer")
                .expect("open");
            self.conn.written(o.size());
            if let Message::Text(t) = &o.msg {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["id"] == json!(rpc) {
                    return v;
                }
            }
        }
    }
}

/// The desktop's windows share one connection, and each pane is its own
/// client: one window hiding its pane leaves the size with the pane still on
/// screen. And a lock another client holds is refused, in the answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn panes_on_one_connection_are_separate_and_locks_are_not_taken_over() {
    let rig = Rig::start().await;
    let id = rig.spawn().await;
    let mut desk = rig.phone();
    rig.engine.sizes().desktop(desk.conn.id());
    desk.attach(&id).await;
    let mut phone = rig.phone();
    phone.attach(&id).await;
    for (pane, (cols, rows)) in [(1, DESKTOP), (2, (80, 20))] {
        desk.call(
            "terminal:viewport",
            json!({ "id": id, "pane": pane, "cols": cols, "rows": rows }),
        );
        desk.call(
            "terminal:presence",
            json!({ "id": id, "pane": pane, "state": "watching" }),
        );
    }
    phone.call(
        "terminal:viewport",
        json!({ "id": id, "cols": PHONE.0, "rows": PHONE.1 }),
    );
    desk.call(
        "terminal:write",
        json!({ "id": id, "pane": 1, "data": "x" }),
    );
    tokio::time::sleep(SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, DESKTOP);
    // The second window hides its pane, long past the grace period.
    desk.call(
        "terminal:presence",
        json!({ "id": id, "pane": 2, "state": "away" }),
    );
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(
        rig.engine_view(&id).await.0,
        DESKTOP,
        "the size left a pane on screen"
    );
    desk.read_for(Duration::from_millis(100)).await;
    assert_eq!(desk.resized.len(), 1);
    assert_eq!(desk.resized[0]["owner"], json!(format!("{}:1", desk.name)));

    // The phone locks; the desktop's lock is refused and says so.
    let rpc = phone.call("terminal:lockSize", json!({ "id": id, "locked": true }));
    assert!(phone.answer(rpc).await.get("error").is_none());
    let rpc = desk.call(
        "terminal:lockSize",
        json!({ "id": id, "pane": 1, "locked": true }),
    );
    let refused = desk.answer(rpc).await;
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("locked by another client")),
        "{refused}"
    );
    tokio::time::sleep(SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, PHONE);
    rig.task.abort();
}

/// A new terminal fits the pane that opened it on first draw: the desktop
/// connection asked for the session, so its pane's viewport sizes the PTY
/// without anyone typing; a phone looking does not take it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_terminal_fits_its_pane_on_first_draw() {
    let rig = Rig::start().await;
    let id = rig.spawn().await;
    let mut desk = rig.phone();
    rig.engine.sizes().desktop(desk.conn.id());
    rig.engine.sizes().opened_by(&id, desk.conn.id());
    let mut phone = rig.phone();
    phone.attach(&id).await;
    phone.call(
        "terminal:viewport",
        json!({ "id": id, "cols": PHONE.0, "rows": PHONE.1 }),
    );
    desk.attach(&id).await;
    desk.call(
        "terminal:viewport",
        json!({ "id": id, "pane": 1, "cols": DESKTOP.0, "rows": DESKTOP.1 }),
    );
    tokio::time::sleep(SETTLED).await;
    assert_eq!(rig.engine_view(&id).await.0, DESKTOP, "the pane's size");
    desk.read_for(Duration::from_millis(100)).await;
    assert_eq!(desk.resized.len(), 1);
    assert_eq!(desk.resized[0]["reason"], "launch");
    assert_eq!(desk.resized[0]["owner"], json!(format!("{}:1", desk.name)));
    assert_eq!(desk.grid().0, DESKTOP);

    // Another connection asking for a session already shown takes nothing.
    let other = rig.spawn().await;
    desk.attach(&other).await;
    desk.call(
        "terminal:viewport",
        json!({ "id": other, "pane": 1, "cols": DESKTOP.0, "rows": DESKTOP.1 }),
    );
    rig.engine.sizes().opened_by(&other, phone.conn.id());
    phone.attach(&other).await;
    phone.call(
        "terminal:viewport",
        json!({ "id": other, "cols": PHONE.0, "rows": PHONE.1 }),
    );
    tokio::time::sleep(SETTLED).await;
    assert_eq!(rig.engine_view(&other).await.0, LAUNCH);
    rig.task.abort();
}
