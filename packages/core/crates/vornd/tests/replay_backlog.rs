//! A backlog of cursor-drawn output with no newline replays once and stays connected, not overflowing the queue to sessiond on every reconnect.

#![cfg(all(feature = "engine", unix))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use vorn_engine::Config;
use vorn_sessiond::server::{self, Sessiond};
use vorn_sessiond_wire::{Io, SpawnSpec};
use vornd::engine::{Engine, Event, DEFAULT_SCROLLBACK};
use vornd::holder::{self, Holder};

/// What the program prints while no vornd is connected.
const BACKLOG: u64 = 48 << 20;
const PATIENCE: Duration = Duration::from_secs(300);

fn engine(history: std::path::PathBuf) -> Arc<Engine> {
    Engine::new(Config {
        scrollback: DEFAULT_SCROLLBACK,
        history: Some(history),
        build: "test".into(),
        ..Config::default()
    })
}

/// Ends the test's program, its pid written by the script, when the test ends or panics.
struct Reap(std::path::PathBuf);

impl Drop for Reap {
    fn drop(&mut self) {
        let pid = std::fs::read_to_string(&self.0).ok();
        if let Some(pid) = pid.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            let _ = std::process::Command::new("kill").arg(pid).status();
        }
    }
}

/// Total bytes in sessiond's spool directory.
fn spooled(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

async fn ready(events: &mut broadcast::Receiver<Event>, id: &str) {
    loop {
        match events.recv().await {
            Ok(Event::Ready { session, .. }) if session == id => return,
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(e) => panic!("events: {e}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backlog_of_redraws_recovers_and_stays_connected() {
    let home = tempfile::tempdir().unwrap();
    let d = Sessiond::new(server::Config {
        home: home.path().to_path_buf(),
        instance: 0xb4c1,
        build: "test".into(),
        idle_exit: Duration::from_secs(600),
        spool_cap: 512 << 20,
    });
    let listener = server::bind(&d).unwrap();
    let _serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
    let history = home.path().join("history");

    // Frames drawn after the first vornd is gone, as an agent's sign-in animation.
    let first = engine(history.clone());
    let holder = Holder::with_engine(Arc::clone(&first));
    let endpoint = d.endpoint();
    let task = tokio::spawn(async move {
        let _ = holder::connect(&endpoint, &holder).await;
    });
    let pid = home.path().join("pid");
    let done = home.path().join("done");
    let _reap = Reap(pid.clone());
    let script = format!(
        "echo $$ > '{}'; sleep 1; f=$(printf '\\033[2;18H+ ;\\033[3;18H,/ _.\\\\\\033[4;17H|_||.\\033[5;17H|/|^~,\\033[49m'); \
         yes \"$f\" | tr -d '\\n' | head -c {BACKLOG}; touch '{}'; exec sleep 600",
        pid.display(),
        done.display()
    );
    let t = Instant::now();
    let id = loop {
        let spec = SpawnSpec {
            argv: vec!["sh".into(), "-c".into(), script.clone()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: Vec::new(),
            io: Io::Pty {
                cols: 133,
                rows: 44,
            },
            ring_bytes: None,
        };
        match first.spawn(spec).await {
            Ok(id) => break id,
            // The connection may not be up yet.
            Err(_) if t.elapsed() < Duration::from_secs(20) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("spawn: {e}"),
        }
    };
    task.abort();
    let _ = task.await;

    // Until the program has drawn everything and sessiond's spool has stopped growing.
    let spool = home.path().join("spool");
    let t = Instant::now();
    let mut last = u64::MAX;
    loop {
        let now = spooled(&spool);
        if done.exists() && now == last {
            break;
        }
        last = now;
        assert!(t.elapsed() < PATIENCE, "the program never finished drawing");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // The next vornd replays it all and stays.
    let second = engine(history);
    let mut events = second.subscribe();
    let holder = Holder::with_engine(Arc::clone(&second));
    let endpoint = d.endpoint();
    let mut conn = tokio::spawn(async move { holder::connect(&endpoint, &holder).await });
    tokio::select! {
        () = ready(&mut events, &id) => {}
        r = &mut conn => panic!("vornd gave the connection up while replaying: {r:?}"),
        () = tokio::time::sleep(PATIENCE) => panic!("{id} never went live"),
    }
    tokio::select! {
        r = &mut conn => panic!("vornd gave the connection up after replaying: {r:?}"),
        () = tokio::time::sleep(Duration::from_secs(5)) => {}
    }
    conn.abort();
}
