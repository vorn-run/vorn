//! vornd's session engine on a real sessiond: sessions spawned through it,
//! vornd killed (its connection dropped with no last word) and a new one
//! recovering every session from sessiond.

#![cfg(all(feature = "engine", unix))]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use vorn_engine::{Base, Config, Effect, Fidelity, State, Summary};
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{Io, SpawnSpec, Stdin};
use vornd::engine::{Engine, Event};
use vornd::holder::{self, Holder};

const PATIENCE: Duration = Duration::from_secs(20);

struct Rig {
    d: Arc<Sessiond>,
    home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// A vornd with the engine: the engine, what happens to its sessions from
/// the start, and its task. Aborting the task kills it.
struct Vornd {
    engine: Arc<Engine>,
    events: broadcast::Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0xe17e,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        let serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
        Rig {
            d,
            home,
            _serving: serving,
        }
    }

    fn history(&self) -> std::path::PathBuf {
        self.home.path().join("history")
    }

    /// A vornd with the engine, connected to this sessiond.
    fn vornd(&self) -> Vornd {
        let engine = Engine::new(Config {
            history: Some(self.history()),
            build: "test".into(),
            ..Config::default()
        });
        let events = engine.subscribe();
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = self.d.endpoint();
        let task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        Vornd {
            engine,
            events,
            task,
        }
    }

    /// Waits until sessiond has let session `id` go.
    async fn released(&self, id: &str) {
        let t = Instant::now();
        while self.d.holds(id) {
            assert!(t.elapsed() < PATIENCE, "{id} never released");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Vornd {
    async fn kill(self) {
        self.task.abort();
        let _ = self.task.await;
    }

    /// How session `id` stood when it left the engine.
    async fn closed(&mut self, id: &str) -> Arc<Summary> {
        let t = Instant::now();
        loop {
            let left = PATIENCE.saturating_sub(t.elapsed());
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Ok(Event::Closed(s))) if s.brief.session == id => return s,
                Ok(Ok(_)) => {}
                Ok(Err(e)) => panic!("events: {e}"),
                Err(_) => panic!("{id} never left the engine"),
            }
        }
    }
}

async fn spawn(engine: &Engine, argv: &[&str], io: Io) -> String {
    let t = Instant::now();
    loop {
        let spec = SpawnSpec {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: Vec::new(),
            io,
            ring_bytes: None,
        };
        match engine.spawn(spec).await {
            Ok(id) => return id,
            // The connection may not be up yet.
            Err(e) if t.elapsed() < PATIENCE => {
                let _ = e;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("spawn: {e}"),
        }
    }
}

/// The session as the engine has it, once `ok` holds.
async fn wait(engine: &Engine, id: &str, what: &str, ok: impl Fn(&Summary) -> bool) -> Summary {
    let t = Instant::now();
    loop {
        let found = engine
            .sessions()
            .await
            .into_iter()
            .find(|s| s.brief.session == id);
        if let Some(s) = &found {
            if ok(s) {
                return s.clone();
            }
        }
        assert!(t.elapsed() < PATIENCE, "{what}: last seen {found:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Every history record's rseq and the bytes of its output, in file order.
fn history(dir: &Path, id: &str) -> (Vec<u64>, Vec<u8>) {
    let bytes = std::fs::read(dir.join(format!("{id}.log"))).unwrap();
    let (_, _, frames, _) = vorn_pipeline::history::parse(&bytes).expect("a history log");
    let mut out = Vec::new();
    for f in &frames {
        if let vorn_term_proto::Record::Data { bytes, .. } = &f.record {
            out.extend(bytes);
        }
    }
    (frames.iter().map(|f| f.rseq).collect(), out)
}

fn assert_once_each(rseqs: &[u64]) {
    assert!(
        rseqs.windows(2).all(|w| w[0] < w[1]),
        "each rseq once, in order: {rseqs:?}"
    );
}

/// No file of session `id`'s history is left.
fn no_history(dir: &Path, id: &str) {
    let left: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        left.iter().all(|n| !n.to_string_lossy().starts_with(id)),
        "{left:?}"
    );
}

/// RC-T9 through vornd: a program prints a lot and exits 7 while vornd is
/// dead. The next vornd recovers the session exactly, through the last
/// line and the exit with its code, writing nothing to the program while
/// it replays; then releases it in sessiond and deletes its history.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn output_then_exit_while_vornd_is_dead() {
    let rig = Rig::start().await;
    let first = rig.vornd();
    let id = spawn(
        &first.engine,
        &[
            "sh",
            "-c",
            "i=0; while [ $i -lt 3000 ]; do echo line$i; i=$((i+1)); done; sleep 1; exit 7",
        ],
        Io::Pty { cols: 80, rows: 24 },
    )
    .await;
    wait(&first.engine, &id, "some output", |s| {
        s.brief.cursor.is_some_and(|c| c.next_offset > 0)
    })
    .await;
    first.kill().await;

    // The program finishes while no vornd is there.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut second = rig.vornd();
    let s = second.closed(&id).await;
    assert_eq!(s.brief.state, State::Ended, "{s:#?}");
    assert_eq!(s.brief.exited, Some((Some(7), None)));
    assert_eq!(s.brief.fidelity, Fidelity::Exact, "{s:#?}");
    assert_eq!(
        s.brief.base,
        Some(Base::Newest),
        "the checkpoint cut at spawn, or a later one"
    );
    assert!(s.screen.contains("line2999"), "{}", s.screen);
    rig.released(&id).await;
    no_history(&rig.history(), &id);
    let report = second.engine.report();
    assert_eq!(report["closed"][0]["session"], id.as_str(), "{report}");
    assert_eq!(report["closed"][0]["state"], "ended", "{report}");
}

/// RC-T13 through vornd: a piped agent's stdout and stderr, input before
/// vornd dies, stdin closed by the next vornd, and the exit code: all
/// intact. The disk history holds each record once across the two vornds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_piped_agent_survives_vornd() {
    let rig = Rig::start().await;
    let first = rig.vornd();
    let id = spawn(
        &first.engine,
        &[
            "sh",
            "-c",
            "printf out; printf err >&2; read x; printf got-$x; cat >/dev/null; printf bye; exit 7",
        ],
        Io::Piped { stdin: Stdin::Pipe },
    )
    .await;
    wait(&first.engine, &id, "both streams", |s| {
        s.screen.contains("outerr") || s.screen.contains("errout")
    })
    .await;
    first.engine.write(&id, b"hi\n".to_vec()).unwrap();
    wait(&first.engine, &id, "the input echoed", |s| {
        s.screen.contains("got-hi")
    })
    .await;
    first.kill().await;

    let mut second = rig.vornd();
    wait(&second.engine, &id, "recovered", |s| {
        s.brief.state == State::Live
    })
    .await;
    let (rseqs, bytes) = history(&rig.history(), &id);
    assert_once_each(&rseqs);
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.len(), "outerrgot-hi".len(), "{text:?}");
    assert!(text.ends_with("got-hi"), "{text:?}");

    second.engine.close_stdin(&id).unwrap();
    let s = second.closed(&id).await;
    assert_eq!(s.brief.exited, Some((Some(7), None)));
    assert_eq!(s.brief.fidelity, Fidelity::Exact, "{s:#?}");
    assert!(s.screen.contains("got-hibye"), "{}", s.screen);
    rig.released(&id).await;
    no_history(&rig.history(), &id);
}

/// The debug report says how each session was recovered, and never what
/// is on its screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_report_says_how_and_not_what() {
    let rig = Rig::start().await;
    let first = rig.vornd();
    let id = spawn(
        &first.engine,
        &["sh", "-c", "echo secret-text; sleep 30"],
        Io::Pty { cols: 80, rows: 24 },
    )
    .await;
    wait(&first.engine, &id, "output", |s| {
        s.screen.contains("secret-text")
    })
    .await;
    first.kill().await;
    let second = rig.vornd();
    wait(&second.engine, &id, "recovered", |s| {
        s.brief.state == State::Live
    })
    .await;
    let report = second.engine.report();
    let text = report.to_string();
    assert!(!text.contains("secret-text"), "{text}");
    let s = &report["sessions"][0];
    assert_eq!(s["session"], id.as_str());
    assert_eq!(s["state"], "live");
    assert_eq!(s["fidelity"], "exact");
    assert_eq!(s["base"], "newest checkpoint");
    assert!(s["cursor"]["nextRseq"].as_u64().unwrap() > 0, "{s}");
}

/// A session started again under its name as soon as its exit is told, as
/// the app resumes a shell that ended, is started rather than refused while
/// the ended one is still leaving the engine.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_is_free_again_once_its_exit_is_told() {
    let rig = Rig::start().await;
    let mut v = rig.vornd();
    let spec = || SpawnSpec {
        argv: vec!["sh".into(), "-c".into(), "exit 3".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: Vec::new(),
        io: Io::Pty { cols: 80, rows: 24 },
        ring_bytes: None,
    };
    let t = Instant::now();
    while let Err(e) = v.engine.spawn_as(spec(), Some("again".into())).await {
        assert!(t.elapsed() < PATIENCE, "the first spawn: {e}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for run in 0..10 {
        loop {
            match tokio::time::timeout(PATIENCE, v.events.recv()).await {
                Ok(Ok(Event::Effect(fx, Effect::Exit { .. }))) if fx.session == "again" => break,
                Ok(Ok(_)) => {}
                Ok(Err(e)) => panic!("events: {e}"),
                Err(_) => panic!("run {run} never exited"),
            }
        }
        let s = v.engine.spawn_as(spec(), Some("again".into())).await;
        assert_eq!(s.map(|s| s.id), Ok("again".to_owned()), "run {run}");
    }
}
