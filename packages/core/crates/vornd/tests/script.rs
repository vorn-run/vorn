//! Project scripts run by vornd on a real sessiond: a failing one reports
//! its exit code and everything it printed, and a cancelled one is killed.

#![cfg(all(feature = "engine", unix))]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast;
use vorn_engine::{Config, Summary};
use vorn_sessiond::server::{self, Sessiond};
use vornd::engine::{Engine, EngineHost, Event};
use vornd::groups::{Groups, Mode};
use vornd::holder::{self, Holder};
use vornd::native::script::Scripts;
use vornd::native::sessions::Started;
use vornd::native::Native;

const PATIENCE: Duration = Duration::from_secs(20);

/// A sessiond, a vornd engine held by it, and scripts run through them.
struct Rig {
    engine: Arc<Engine>,
    events: broadcast::Receiver<Event>,
    scripts: Arc<Scripts>,
    data: tempfile::TempDir,
    _native: Arc<Native>,
    _home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
    _holding: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0x5c41,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        let serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
        let engine = Engine::new(Config {
            history: Some(home.path().join("history")),
            build: "test".into(),
            ..Config::default()
        });
        let events = engine.subscribe();
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = d.endpoint();
        let holding = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        let data = tempfile::tempdir().unwrap();
        let native = Native::new();
        native.set_database(data.path().join("vorn.db"));
        native.set_host(Arc::new(EngineHost::new(
            Arc::clone(&engine),
            tokio::runtime::Handle::current(),
        )));
        let groups = Arc::new(Groups::parse("script=native").unwrap());
        let scripts = Scripts::new(Mode::Native, &native, groups);
        Rig {
            engine,
            events,
            scripts,
            data,
            _native: native,
            _home: home,
            _serving: serving,
            _holding: holding,
        }
    }

    /// Runs a bash script under `id`, once the holder is connected, and
    /// waits for its start.
    async fn run(&self, id: &str, content: &str, secrets: Value) -> Started {
        let params = json!({
            "id": id,
            "scriptType": "bash",
            "scriptContent": content,
            "cwd": self.data.path(),
            "args": [],
            "secretEnv": secrets,
        });
        let t = Instant::now();
        loop {
            let scripts = Arc::clone(&self.scripts);
            let params = params.clone();
            let outcome = Arc::new(Mutex::new(None));
            let then = Arc::clone(&outcome);
            let asked = tokio::task::spawn_blocking(move || {
                scripts.run(&params, Box::new(move |o| *then.lock().unwrap() = Some(o)))
            })
            .await
            .unwrap();
            match asked {
                Ok(()) => loop {
                    if let Some(o) = outcome.lock().unwrap().take() {
                        return o.expect("the script starts");
                    }
                    assert!(t.elapsed() < PATIENCE, "{id} never started");
                    tokio::time::sleep(Duration::from_millis(20)).await;
                },
                Err(why) if why.contains("not connected") && t.elapsed() < PATIENCE => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(why) => panic!("{id} refused: {why}"),
            }
        }
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

    /// Waits until the engine has session `id` live.
    async fn live(&self, id: &str) {
        let t = Instant::now();
        while !self
            .engine
            .sessions()
            .await
            .iter()
            .any(|s| s.brief.session == id)
        {
            assert!(t.elapsed() < PATIENCE, "{id} never went live");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The scripts' files left on disk.
    fn files_left(&self) -> usize {
        std::fs::read_dir(self.data.path().join("scripts"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_script_reports_its_exit_code_and_output() {
    let mut rig = Rig::start().await;
    let id = "script-fails";
    let started = rig
        .run(
            id,
            r#"printf "out-$TOKEN "; printf "err-$VORN_DATA_DIR" >&2; exit 3"#,
            json!({ "TOKEN": "t0k" }),
        )
        .await;
    assert!(started.pid > 0);
    assert!(rig.scripts.runs(id));
    assert_eq!(rig.files_left(), 1, "the script is written to a file");

    let s = rig.closed(id).await;
    assert_eq!(s.brief.exited, Some((Some(3), None)), "{s:#?}");
    // stdout and stderr are read as one stream.
    let data = rig.data.path().to_string_lossy().into_owned();
    assert!(s.screen.contains("out-t0k"), "{}", s.screen);
    assert!(s.screen.contains(&format!("err-{data}")), "{}", s.screen);

    rig.scripts.ended(id);
    assert!(!rig.scripts.runs(id));
    assert_eq!(rig.files_left(), 0, "the script's file goes when it ends");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_script_is_killed() {
    let mut rig = Rig::start().await;
    let id = "script-cancelled";
    rig.run(id, "printf waiting; sleep 30; printf never", json!({}))
        .await;
    rig.live(id).await;

    let t = Instant::now();
    rig.scripts.cancel(id).unwrap();
    let s = rig.closed(id).await;
    assert!(t.elapsed() < Duration::from_secs(5), "stopped on SIGTERM");
    assert_eq!(s.brief.exited, Some((None, Some(15))), "{s:#?}");
    assert!(!s.screen.contains("never"), "{}", s.screen);

    assert!(rig.scripts.cancel("script-unknown").is_err());
}
