//! vornd's session engine on a real sessiond: sessions spawned through it,
//! vornd killed (its connection dropped with no last word) and a new one
//! recovering every session from sessiond.

#![cfg(all(feature = "engine", unix))]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vorn_engine::{Base, Config, Fidelity, State, Summary};
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{Io, SpawnSpec, Stdin};
use vornd::engine::Engine;
use vornd::holder::{self, Holder};

const PATIENCE: Duration = Duration::from_secs(20);

struct Rig {
    d: Arc<Sessiond>,
    home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
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

    /// A vornd with the engine, connected to this sessiond. Aborting the
    /// task kills it.
    fn vornd(&self) -> (Arc<Engine>, tokio::task::JoinHandle<()>) {
        let engine = Engine::new(Config {
            history: Some(self.history()),
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = self.d.endpoint();
        let task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        (engine, task)
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
            .find(|s| s.session == id);
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

/// RC-T9 through vornd: a program prints a lot and exits 7 while vornd is
/// dead. The next vornd recovers the session exactly: every line, then the
/// exit with its code, and nothing written to the program while it
/// replayed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn output_then_exit_while_vornd_is_dead() {
    let rig = Rig::start().await;
    let (engine, vornd) = rig.vornd();
    let id = spawn(
        &engine,
        &[
            "sh",
            "-c",
            "i=0; while [ $i -lt 3000 ]; do echo line$i; i=$((i+1)); done; sleep 1; exit 7",
        ],
        Io::Pty { cols: 80, rows: 24 },
    )
    .await;
    wait(&engine, &id, "some output", |s| {
        s.cursor.is_some_and(|c| c.next_offset > 0)
    })
    .await;
    vornd.abort();
    let _ = vornd.await;

    // The program finishes while no vornd is there.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (engine, _vornd) = rig.vornd();
    let s = wait(&engine, &id, "recovered and exited", |s| {
        s.state == State::Live && s.exited.is_some()
    })
    .await;
    assert_eq!(s.exited, Some((Some(7), None)));
    assert_eq!(s.fidelity, Fidelity::Exact, "{s:#?}");
    assert_eq!(
        s.base,
        Some(Base::Newest),
        "the checkpoint cut at spawn, or a later one"
    );
    assert!(s.screen.contains("line2999"), "{}", s.screen);
    let (rseqs, bytes) = history(&rig.history(), &id);
    assert_once_each(&rseqs);
    let text = String::from_utf8_lossy(&bytes);
    let mut from = 0;
    for i in 0..3000 {
        let needle = format!("line{i}\r\n");
        let at = text[from..]
            .find(&needle)
            .unwrap_or_else(|| panic!("line{i} in order"));
        from += at + needle.len();
    }
}

/// RC-T13 through vornd: a piped agent's stdout and stderr, input before
/// vornd dies, stdin closed by the next vornd, and the exit code: all
/// intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_piped_agent_survives_vornd() {
    let rig = Rig::start().await;
    let (engine, vornd) = rig.vornd();
    let id = spawn(
        &engine,
        &[
            "sh",
            "-c",
            "printf out; printf err >&2; read x; printf got-$x; cat >/dev/null; printf bye; exit 7",
        ],
        Io::Piped { stdin: Stdin::Pipe },
    )
    .await;
    wait(&engine, &id, "both streams", |s| {
        s.screen.contains("outerr") || s.screen.contains("errout")
    })
    .await;
    engine.write(&id, b"hi\n".to_vec()).unwrap();
    wait(&engine, &id, "the input echoed", |s| {
        s.screen.contains("got-hi")
    })
    .await;
    vornd.abort();
    let _ = vornd.await;

    let (engine, _vornd) = rig.vornd();
    wait(&engine, &id, "recovered", |s| s.state == State::Live).await;
    engine.close_stdin(&id).unwrap();
    let s = wait(&engine, &id, "exited", |s| s.exited.is_some()).await;
    assert_eq!(s.exited, Some((Some(7), None)));
    assert_eq!(s.fidelity, Fidelity::Exact, "{s:#?}");
    assert!(s.screen.contains("got-hibye"), "{}", s.screen);
    let (rseqs, bytes) = history(&rig.history(), &id);
    assert_once_each(&rseqs);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.ends_with("got-hibye"), "{text:?}");
    assert_eq!(text.len(), "outerrgot-hibye".len(), "{text:?}");
}

/// The debug report says how each session was recovered, and never what
/// is on its screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_report_says_how_and_not_what() {
    let rig = Rig::start().await;
    let (engine, vornd) = rig.vornd();
    let id = spawn(
        &engine,
        &["sh", "-c", "echo secret-text; sleep 30"],
        Io::Pty { cols: 80, rows: 24 },
    )
    .await;
    wait(&engine, &id, "output", |s| s.screen.contains("secret-text")).await;
    vornd.abort();
    let _ = vornd.await;
    let (engine, _vornd) = rig.vornd();
    wait(&engine, &id, "recovered", |s| s.state == State::Live).await;
    let report = engine.report().await;
    let text = report.to_string();
    assert!(!text.contains("secret-text"), "{text}");
    let s = &report["sessions"][0];
    assert_eq!(s["session"], id.as_str());
    assert_eq!(s["state"], "live");
    assert_eq!(s["fidelity"], "exact");
    assert_eq!(s["base"], "newest checkpoint");
    assert!(s["cursor"]["nextRseq"].as_u64().unwrap() > 0, "{s}");
}
