//! The real `vorn-sessiond` binary, started the way the app will start it.

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vorn_sessiond::launch::{self, Instance};
use vorn_sessiond::os;
use vorn_sessiond::wire::*;

const BIN: &str = env!("CARGO_BIN_EXE_vorn-sessiond");

/// A minimal vornd for one sessiond.
struct V {
    s: Box<dyn Duplex>,
    frames: FrameReader,
}

trait Duplex: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Duplex for T {}

impl V {
    async fn hello(i: &Instance) -> V {
        let s = os::connect(&i.endpoint).await.expect("connect");
        let mut v = V {
            s: Box::new(s),
            frames: FrameReader::default(),
        };
        v.send(ToSessiond::Hello(Hello {
            proto_min: 1,
            proto_max: 1,
            vornd_instance: 9,
            vornd_build: "test".into(),
        }))
        .await;
        assert!(matches!(v.recv().await, Some(ToVornd::Welcome(_))));
        v
    }

    async fn send(&mut self, m: ToSessiond) {
        self.s.write_all(&m.encode()).await.expect("send");
    }

    async fn recv(&mut self) -> Option<ToVornd> {
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                // Answer ConPTY's start-up cursor query, as vornd would.
                if let ToVornd::Entries(e) = &m {
                    let asks = e.entries.iter().any(|x| {
                        matches!(&x.rec, vorn_term_proto::Record::Data { bytes, .. }
                            if bytes.windows(4).any(|w| w == b"\x1b[6n"))
                    });
                    if asks {
                        self.send(ToSessiond::Write(Write {
                            session: e.session.clone(),
                            input_seq: 0,
                            bytes: b"\x1b[1;1R".to_vec(),
                        }))
                        .await;
                    }
                }
                return Some(m);
            }
            let n = tokio::time::timeout(Duration::from_secs(20), self.s.read(&mut buf))
                .await
                .expect("answer within 20 s")
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.frames.push(&buf[..n]);
        }
    }

    async fn spawn(&mut self, argv: &[&str], io: Io) -> ToVornd {
        self.send(ToSessiond::Spawn(Spawn {
            req: 1,
            spec: SpawnSpec {
                argv: argv.iter().map(|s| s.to_string()).collect(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io,
                ring_bytes: None,
            },
        }))
        .await;
        loop {
            if let m @ (ToVornd::Spawned(_) | ToVornd::Failed(_)) = self.recv().await.expect("open")
            {
                return m;
            }
        }
    }
}

fn start(home: &Path) -> Instance {
    let bin = launch::install(Path::new(BIN), home, "test").expect("install");
    launch::start(&bin, home, Duration::from_secs(10)).expect("start")
}

fn gone_within(pid: u32, d: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < d {
        if !launch::alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(unix)]
const LONG: &[&str] = &["sh", "-c", "sleep 100"];
#[cfg(windows)]
const LONG: &[&str] = &["cmd.exe", "/c", "ping -n 100 127.0.0.1 >nul"];

#[cfg(unix)]
const SHORT: &[&str] = &["sh", "-c", "sleep 0.3"];
#[cfg(windows)]
const SHORT: &[&str] = &["cmd.exe", "/c", "ping -n 2 127.0.0.1 >nul"];

/// RC-T14: a sessiond crash ends its sessions, on each OS, within 2 s.
#[tokio::test]
async fn a_sessiond_crash_ends_its_sessions() {
    let home = tempfile::tempdir().unwrap();
    let i = start(home.path());
    assert_eq!(launch::running(home.path()), vec![i.clone()]);
    let mut v = V::hello(&i).await;
    let ToVornd::Spawned(s) = v.spawn(LONG, Io::Pty { cols: 80, rows: 24 }).await else {
        panic!("spawn failed");
    };
    assert!(launch::alive(s.pid));
    launch::kill(i.pid).unwrap();
    assert!(
        gone_within(i.pid, Duration::from_secs(2)),
        "sessiond is gone"
    );
    assert!(
        gone_within(s.pid, Duration::from_secs(2)),
        "its session ended with it"
    );
    assert_eq!(v.recv().await, None);
    // Its announcement is cleaned up by the next look.
    assert!(launch::running(home.path()).is_empty());
}

/// RC-T11: an old sessiond drains. New sessions go to the new one, and the
/// old one exits after its last session is released.
#[tokio::test]
async fn an_old_sessiond_drains_and_exits() {
    let home = tempfile::tempdir().unwrap();
    let old = start(home.path());
    let new = start(home.path());
    assert_ne!(old.endpoint, new.endpoint);
    assert_eq!(launch::running(home.path()).len(), 2);

    let mut o = V::hello(&old).await;
    let mut n = V::hello(&new).await;
    let ToVornd::Spawned(kept) = o.spawn(SHORT, Io::Pty { cols: 80, rows: 24 }).await else {
        panic!("spawn on old");
    };
    o.send(ToSessiond::Drain(Drain)).await;
    assert!(matches!(
        o.spawn(SHORT, Io::Pty { cols: 80, rows: 24 }).await,
        ToVornd::Failed(_)
    ));
    assert!(matches!(
        n.spawn(LONG, Io::Pty { cols: 80, rows: 24 }).await,
        ToVornd::Spawned(_)
    ));

    // The old session runs to its end and is released.
    o.send(ToSessiond::Attach(Attach {
        session: kept.session.clone(),
        from: AttachFrom::SessionStart,
    }))
    .await;
    'exit: loop {
        if let Some(ToVornd::Entries(e)) = o.recv().await {
            for x in e.entries {
                if matches!(x.rec, vorn_term_proto::Record::Exit { .. }) {
                    break 'exit;
                }
            }
        }
    }
    assert!(launch::alive(old.pid), "it waits for the release");
    o.send(ToSessiond::Release(SessionRef {
        session: kept.session,
    }))
    .await;
    assert!(
        gone_within(old.pid, Duration::from_secs(5)),
        "the old one exits"
    );
    let left = launch::running(home.path());
    assert_eq!(left, vec![new.clone()]);
    launch::kill(new.pid).unwrap();
}

/// RC-T16 (Linux): sessiond runs in its own transient scope, so stopping the
/// app's scope does not take it along. Skipped where no user manager runs.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn on_linux_it_gets_its_own_scope() {
    let ok = std::process::Command::new("systemd-run")
        .args(["--user", "--scope", "--quiet", "--collect", "true"])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("no systemd user manager here; skipping");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let i = start(home.path());
    let cgroup = std::fs::read_to_string(format!("/proc/{}/cgroup", i.pid)).unwrap();
    assert!(cgroup.contains("vorn-sessiond-"), "{cgroup}");
    launch::kill(i.pid).unwrap();
}
