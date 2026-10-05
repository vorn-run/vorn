//! Grid mode through vornd's endpoint, on a real sessiond: a headless client
//! (vorn-grid-client) says Hello, attaches to a session the engine runs,
//! types into it, follows its frames, and resumes after vornd is killed.

#![cfg(all(feature = "engine", unix))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use vorn_engine::{Config, State};
use vorn_grid_client::{Client, Got};
use vorn_sessiond::os;
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{Io, SpawnSpec};
use vorn_term_proto::msg::{
    caps, kind, Attach, AttachMode, ClientKind, ClientMsg, Fidelity, GridResume, Hello, InputEvent,
    Resume, ResyncReason, ServerMsg, Size, PROTO_MAJOR,
};
use vornd::engine::Engine;
use vornd::grid;
use vornd::holder::{self, Holder};

const PATIENCE: Duration = Duration::from_secs(20);

struct Rig {
    d: Arc<Sessiond>,
    _home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
}

struct Vornd {
    engine: Arc<Engine>,
    task: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0x6d1d,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        let serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
        Rig {
            d,
            _home: home,
            _serving: serving,
        }
    }

    fn vornd(&self) -> Vornd {
        let engine = Engine::new(Config {
            scrollback: 1 << 20,
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = self.d.endpoint();
        let task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        Vornd { engine, task }
    }
}

impl Vornd {
    async fn kill(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    async fn spawn(&self, argv: &[&str]) -> String {
        let t = Instant::now();
        loop {
            let spec = SpawnSpec {
                argv: argv.iter().map(|s| s.to_string()).collect(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io: Io::Pty { cols: 60, rows: 12 },
                ring_bytes: None,
            };
            match self.engine.spawn(spec).await {
                Ok(id) => return id,
                Err(_) if t.elapsed() < PATIENCE => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("spawn: {e}"),
            }
        }
    }

    async fn live(&self, id: &str) {
        let t = Instant::now();
        loop {
            let live = self
                .engine
                .sessions()
                .await
                .iter()
                .any(|s| s.brief.session == id && s.brief.state == State::Live);
            if live {
                return;
            }
            assert!(t.elapsed() < PATIENCE, "{id} never went live");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The session's screen as the engine has it.
    async fn screen(&self, id: &str) -> String {
        self.engine
            .sessions()
            .await
            .into_iter()
            .find(|s| s.brief.session == id)
            .map(|s| s.screen)
            .unwrap_or_default()
    }

    /// A grid client connected to this vornd's endpoint.
    fn app(&self, hello: Hello) -> App {
        let (ours, theirs) = tokio::io::duplex(1 << 20);
        let engine = Arc::clone(&self.engine);
        tokio::spawn(async move { grid::serve_conn(theirs, engine, "test", 1).await });
        let mut client = Client::new();
        client.send(&ClientMsg::Hello(hello));
        App {
            client,
            io: ours,
            got: Vec::new(),
        }
    }
}

fn hello(major: u16) -> Hello {
    Hello {
        proto_major: major,
        proto_minor: 3,
        caps: caps::ALL | 1 << 50,
        client: ClientKind::Native,
        build: "test".into(),
    }
}

/// The app's end of a grid connection.
struct App {
    client: Client,
    io: DuplexStream,
    got: Vec<Got>,
}

impl App {
    async fn flush(&mut self) {
        let out = self.client.take_out();
        if !out.is_empty() {
            self.io.write_all(&out).await.unwrap();
        }
    }

    /// Reads until `done` holds, acknowledging every frame as it comes.
    /// False if the connection ended first.
    async fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) -> bool {
        let t = Instant::now();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            self.flush().await;
            if done(self) {
                return true;
            }
            let left = PATIENCE.saturating_sub(t.elapsed());
            let n = match tokio::time::timeout(left, self.io.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return false,
                Ok(Ok(n)) => n,
                Err(_) => panic!("{what}: timed out; got {:?}", self.got),
            };
            for g in self.client.receive(&buf[..n]).unwrap() {
                if let Got::Frame { sid, rev, .. } = g {
                    self.client.ack(sid, rev);
                }
                if let Got::Refused { why, .. } = g {
                    panic!("{what}: a frame was refused: {why:?}");
                }
                self.got.push(g);
            }
        }
    }

    fn text(&self, sid: u32) -> String {
        self.client
            .pane(sid)
            .and_then(|p| p.mirror())
            .map(|m| m.text().join("\n"))
            .unwrap_or_default()
    }

    fn attached(&self) -> Option<u32> {
        self.got.iter().rev().find_map(|g| match g {
            Got::Attached(a) => Some(a.sid),
            _ => None,
        })
    }
}

fn attach(session: &str, resume: Option<GridResume>) -> Attach {
    Attach {
        session: session.to_owned(),
        mode: AttachMode::Grid,
        view: Size {
            cols: 60,
            rows: 12,
            px_w: 0,
            px_h: 0,
        },
        visible: true,
        resume: resume.map(Resume::Grid),
        history_tail: 50,
    }
}

/// The screen as the engine formats it, trailing blank lines dropped, for
/// comparing with a mirror's text.
fn trimmed(s: &str) -> String {
    let lines: Vec<&str> = s.lines().map(|l| l.trim_end()).collect();
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    lines[..end].join("\n")
}

/// TP-T18 at the endpoint: an app one major ahead gets Error 426 and the
/// connection ends; one a major behind attaches with the capabilities both
/// have, and unknown fields and optional kinds are ignored on the way.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn t18_versions_at_the_endpoint() {
    let rig = Rig::start().await;
    let v = rig.vornd();
    let id = v.spawn(&["sh", "-c", "echo ready; sleep 30"]).await;
    v.live(&id).await;

    let mut newer = v.app(hello(PROTO_MAJOR + 1));
    assert!(!newer.until("426", |_| false).await, "the connection ended");
    assert!(newer
        .got
        .iter()
        .any(|g| matches!(g, Got::Message(ServerMsg::Error { code: 426, .. }))));

    let mut older = v.app(hello(PROTO_MAJOR - 1));
    older
        .until("welcome", |a| a.client.welcome().is_some())
        .await;
    let w = older.client.welcome().unwrap();
    assert_eq!((w.proto_major, w.caps), (PROTO_MAJOR - 1, caps::ALL));
    // An optional kind this vornd does not know, then an Attach with a field
    // it does not know: both are fine.
    let mut raw = Vec::new();
    raw.extend_from_slice(&2u32.to_le_bytes());
    raw.extend_from_slice(&[kind::OPTIONAL | 0x1f, 0xa0]);
    older.io.write_all(&raw).await.unwrap();
    let mut frame = Vec::new();
    ClientMsg::Attach(attach(&id, None)).encode(&mut frame);
    // Splice an unknown key (40: "x") into the Attach map before its end.
    let end = frame.len() - 1;
    frame.splice(end..end, [0x18, 40, 0x61, b'x']);
    let len = (frame.len() - 4) as u32;
    frame[..4].copy_from_slice(&len.to_le_bytes());
    older.io.write_all(&frame).await.unwrap();
    older
        .until("ready", |a| {
            a.attached().is_some_and(|s| a.text(s).contains("ready"))
        })
        .await;
    assert!(!older
        .got
        .iter()
        .any(|g| matches!(g, Got::Message(ServerMsg::Error { .. }))));
    v.kill().await;
}

/// A session typed into and followed through the endpoint: input goes in
/// encoded, is acknowledged once written, and the mirror shows what the
/// engine's terminal shows. Then vornd dies and a new one recovers the
/// session: the app resumes and gets a Resync and a snapshot under a new
/// `state_gen`, exact as the recovery was (TP-T13 and T14 end to end).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_app_types_follows_and_survives_vornd() {
    let rig = Rig::start().await;
    let first = rig.vornd();
    let id = first.spawn(&["cat"]).await;
    first.live(&id).await;
    let mut app = first.app(hello(PROTO_MAJOR));
    app.client.attach(attach(&id, None));
    app.until("attached", |a| a.attached().is_some()).await;
    let sid = app.attached().unwrap();
    for word in ["hello", "grid"] {
        let seq = app.client.input_seq();
        app.client.send(&ClientMsg::Input {
            sid,
            input_seq: seq,
            event: InputEvent::Text {
                utf8: format!("{word}\r"),
            },
        });
        app.until("the input acknowledged", |a| {
            a.got.iter().any(|g| {
                matches!(g, Got::Message(ServerMsg::InputAck { input_seq, .. }) if *input_seq == seq)
            })
        })
        .await;
    }
    app.until("the echo", |a| a.text(sid).matches("grid").count() == 2)
        .await;
    let screen = trimmed(&first.screen(&id).await);
    assert_eq!(trimmed(&app.text(sid)), screen);
    let resume = app.client.pane(sid).unwrap().resume().unwrap();
    let old_gen = resume.state_gen;

    first.kill().await;
    let second = rig.vornd();
    second.live(&id).await;
    // The app's connection went with the first vornd; it reconnects.
    let mut app2 = second.app(hello(PROTO_MAJOR));
    app2.client.attach(attach(&id, Some(resume)));
    app2.until("the snapshot", |a| {
        a.attached()
            .and_then(|s| a.client.pane(s))
            .is_some_and(|p| p.snapshots > 0)
    })
    .await;
    let sid2 = app2.attached().unwrap();
    let pane = app2.client.pane(sid2).unwrap();
    assert_eq!(pane.attached.as_ref().unwrap().fidelity, Fidelity::Exact);
    assert!(app2.got.iter().any(|g| matches!(
        g,
        Got::Message(ServerMsg::Resync {
            reason: ResyncReason::Restarted,
            ..
        })
    )));
    assert_ne!(pane.mirror().unwrap().state_gen(), old_gen);
    assert_eq!(trimmed(&app2.text(sid2)), screen);
    second.kill().await;
}

/// An attach to a session the engine does not run is refused by name, and
/// bytes mode is pointed at the WebSocket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn attaches_vornd_cannot_serve_are_refused() {
    let rig = Rig::start().await;
    let v = rig.vornd();
    let mut app = v.app(hello(PROTO_MAJOR));
    app.until("welcome", |a| a.client.welcome().is_some()).await;
    app.client.attach(attach("no-such-session", None));
    let mut bytes = attach("no-such-session", None);
    bytes.mode = AttachMode::Bytes;
    app.client.attach(bytes);
    app.until("two errors", |a| {
        a.got
            .iter()
            .filter(|g| matches!(g, Got::Message(ServerMsg::Error { .. })))
            .count()
            == 2
    })
    .await;
    let codes: Vec<u16> = app
        .got
        .iter()
        .filter_map(|g| match g {
            Got::Message(ServerMsg::Error { code, .. }) => Some(*code),
            _ => None,
        })
        .collect();
    assert_eq!(codes, [grid::code::NOT_FOUND, grid::code::NOT_SERVED]);
    v.kill().await;
}

/// The endpoint is a user-only local socket in `run/`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_endpoint_is_a_user_only_socket() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let endpoint = grid::endpoint(home.path());
    let listener = os::Listener::bind(home.path(), &endpoint).unwrap();
    let engine = Engine::new(Config::default());
    tokio::spawn(grid::serve(listener, engine, "test".into(), 1));
    let mode = std::fs::metadata(&endpoint).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let mut s = tokio::net::UnixStream::connect(&endpoint).await.unwrap();
    let mut c = Client::hello("test");
    s.write_all(&c.take_out()).await.unwrap();
    let mut buf = vec![0u8; 4096];
    let n = s.read(&mut buf).await.unwrap();
    let got = c.receive(&buf[..n]).unwrap();
    assert!(matches!(got.first(), Some(Got::Welcome(_))), "{got:?}");
}
