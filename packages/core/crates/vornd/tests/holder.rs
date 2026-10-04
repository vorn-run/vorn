//! vornd keeping a session holder: the real vornd and vorn-sessiond
//! binaries, started the way the app starts them.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use vorn_sessiond::launch::{self, Instance};
use vorn_sessiond::server::{self, Config, Sessiond};
use vorn_sessiond_wire::PROTO;

const VORND: &str = env!("CARGO_BIN_EXE_vornd");

/// Long enough for a slow CI machine to start a process or two.
const PATIENCE: Duration = Duration::from_secs(20);

/// The vorn-sessiond binary, built next to vornd. Cargo only builds the
/// binaries of the package under test, so it has to be built first.
fn sessiond_bin() -> PathBuf {
    let dir = Path::new(VORND).parent().expect("vornd is in a directory");
    let bin = dir.join(format!("vorn-sessiond{}", std::env::consts::EXE_SUFFIX));
    assert!(
        bin.exists(),
        "{} is missing: run `cargo build -p vorn-sessiond` with the same profile (add --release for a release test run) first",
        bin.display()
    );
    bin
}

/// Kills, on drop, every sessiond a test saw, so a failing test leaves
/// nothing running.
#[derive(Default)]
struct Reap(Vec<u32>);

impl Reap {
    fn add(&mut self, pid: u32) {
        if pid != std::process::id() && !self.0.contains(&pid) {
            self.0.push(pid);
        }
    }
}

impl Drop for Reap {
    fn drop(&mut self) {
        for &pid in &self.0 {
            if launch::alive(pid) {
                let _ = launch::kill(pid);
            }
        }
    }
}

/// A running vornd with a session holder under `home`. Killed on drop.
struct Vornd {
    child: Child,
    port: u16,
    log: PathBuf,
}

impl Vornd {
    fn start(home: &Path, env: &[(&str, &str)]) -> Vornd {
        let log = home.join("vornd.log");
        let mut cmd = Command::new(VORND);
        // Nothing listens on the discard port: the holder does not need the
        // Node server.
        cmd.args([
            "--upstream",
            "127.0.0.1:9",
            "--exit-with-stdin",
            "--sessiond",
        ])
        .arg(sessiond_bin())
        .arg("--home")
        .arg(home)
        .arg("--log-file")
        .arg(&log)
        .env_remove("VORND_GROUPS")
        .env_remove("VORN_SESSIOND_IDLE_EXIT")
        .env("VORND_LOG", "debug")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("start vornd");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut v = Vornd {
            child,
            port: 0,
            log,
        };
        let line = rx
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| panic!("vornd printed no port: {}", v.log_text()));
        let ready: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("port line {line:?}: {e}; {}", v.log_text()));
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .expect("a port");
        v
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The health check's body. The status is 503 here, as the upstream is
    /// down; the body is what matters.
    fn health(&self) -> Value {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect to vornd");
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(b"GET /vornd/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .expect("send");
        let mut res = String::new();
        s.read_to_string(&mut res).expect("read the health check");
        let (_, body) = res.split_once("\r\n\r\n").expect("a body");
        serde_json::from_str(body).unwrap_or_else(|e| panic!("health body {body:?}: {e}"))
    }

    /// Poll the health check until its `sessiond` part satisfies `ok`.
    fn wait_for(&self, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
        let t = Instant::now();
        loop {
            let h = self.health();
            if ok(&h["sessiond"]) {
                return h["sessiond"].clone();
            }
            if t.elapsed() > PATIENCE {
                panic!(
                    "{what}: not within {PATIENCE:?}; last health {h}\nvornd log:\n{}",
                    self.log_text()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait for a current sessiond and return it.
    fn current(&self) -> Value {
        self.wait_for("a current sessiond", |s| s["current"].is_object())["current"].clone()
    }

    /// Stop it the way the app does, by closing its stdin.
    fn stop(mut self) {
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < PATIENCE {
            if self.child.try_wait().expect("wait for vornd").is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("vornd did not stop: {}", self.log_text());
    }
}

impl Drop for Vornd {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn pid(i: &Value) -> u32 {
    i["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())
        .unwrap_or_else(|| panic!("no pid in {i}"))
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

fn sessiond_version() -> String {
    let out = Command::new(sessiond_bin())
        .arg("--version")
        .output()
        .expect("vorn-sessiond --version");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Quitting and relaunching the app keeps the sessiond and its sessions: the
/// next vornd finds the one the last one started.
#[test]
fn a_relaunched_vornd_finds_the_same_sessiond() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let v = Vornd::start(home.path(), &[]);
    let first = v.current();
    reap.add(pid(&first));
    assert_eq!(first["build"], sessiond_version().as_str());
    assert_eq!(first["proto"], PROTO);
    assert_eq!(first["compatible"], true);
    let installed = launch::installed_path(home.path(), &sessiond_version());
    assert!(installed.exists(), "installed as {}", installed.display());
    v.stop();
    assert!(launch::alive(pid(&first)), "sessiond outlives vornd");

    let v = Vornd::start(home.path(), &[]);
    let again = v.current();
    assert_eq!(pid(&again), pid(&first));
    assert_eq!(again["instance"], first["instance"]);
    assert_eq!(launch::running(home.path()).len(), 1, "no second one");
    v.stop();
}

/// A sessiond of an older build is told to drain: it is reported under
/// `older`, a new one of this build takes its place, and it exits once it
/// holds no sessions.
#[test]
fn an_older_build_is_drained_and_exits() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let old = Sessiond::new(Config {
        home: home.path().to_owned(),
        instance: 0x01d,
        build: "0.0.1".into(),
        // Only the drain may end it.
        idle_exit: Duration::from_secs(600),
        spool_cap: 1 << 20,
    });
    let listener = rt.block_on(async { server::bind(&old) }).expect("bind");
    let serving = rt.spawn(server::serve(Arc::clone(&old), listener));

    let v = Vornd::start(home.path(), &[]);
    let current = v.current();
    reap.add(pid(&current));
    let holder = v.wait_for("the older one reported", |s| {
        s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    let older = holder["older"].as_array().unwrap();
    assert_eq!(older.len(), 1, "{holder}");
    assert_eq!(older[0]["build"], "0.0.1");
    assert_eq!(older[0]["instance"], "1d");
    assert_eq!(older[0]["compatible"], true);
    assert_eq!(older[0]["sessions"], 0);
    assert_ne!(current["instance"], "1d");

    let t = Instant::now();
    while !serving.is_finished() {
        assert!(
            t.elapsed() < PATIENCE,
            "the drained sessiond did not exit: {}",
            v.log_text()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    rt.block_on(serving).unwrap().expect("it exits cleanly");
    let left: Vec<u128> = launch::running(home.path())
        .iter()
        .map(|i| i.instance)
        .collect();
    assert!(!left.contains(&0x01d), "its announcement is withdrawn");
    v.stop();
}

/// A sessiond speaking a protocol this vornd does not is left running with
/// its sessions, and reported as incompatible so the app can ask first.
#[test]
fn an_incompatible_sessiond_is_left_alone() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    // An announcement for a live process: this one.
    let foreign = Instance {
        endpoint: "nowhere".into(),
        pid: std::process::id(),
        proto: 999,
        build: "99.0.0".into(),
        instance: 0xf00,
    };
    launch::announce(home.path(), &foreign).unwrap();

    let v = Vornd::start(home.path(), &[]);
    let current = v.current();
    reap.add(pid(&current));
    let holder = v.wait_for("the foreign one reported", |s| {
        s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    let older = holder["older"].as_array().unwrap();
    assert_eq!(older.len(), 1, "{holder}");
    assert_eq!(older[0]["instance"], "f00");
    assert_eq!(older[0]["proto"], 999);
    assert_eq!(older[0]["compatible"], false);
    assert_eq!(older[0]["sessions"], Value::Null);
    assert!(
        launch::running(home.path()).contains(&foreign),
        "its announcement stays"
    );
    v.stop();
}

/// Once vornd stops, as it does when the switch is turned off, a sessiond
/// holding no sessions exits after its idle time (60 s by default, shortened
/// here) and leaves nothing behind.
#[test]
fn a_short_idle_exit_leaves_nothing_behind() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let v = Vornd::start(home.path(), &[("VORN_SESSIOND_IDLE_EXIT", "1")]);
    let current = v.current();
    reap.add(pid(&current));
    v.stop();
    assert!(
        gone_within(pid(&current), Duration::from_secs(10)),
        "sessiond exits after vornd"
    );
    assert!(launch::running(home.path()).is_empty());
}

/// A sessiond that dies is replaced by a new one, and the health check
/// clears once it is up.
#[test]
fn a_killed_sessiond_is_replaced() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let v = Vornd::start(home.path(), &[]);
    let first = v.current();
    reap.add(pid(&first));
    launch::kill(pid(&first)).unwrap();

    let holder = v.wait_for("a new sessiond", |s| {
        s["current"].is_object() && s["current"]["pid"] != first["pid"]
    });
    let second = &holder["current"];
    reap.add(pid(second));
    assert_ne!(second["instance"], first["instance"]);
    assert!(launch::alive(pid(second)));
    let holder = v.wait_for("no error", |s| s["error"].is_null());
    assert_eq!(holder["current"]["pid"], second["pid"]);
    assert_eq!(launch::running(home.path()).len(), 1);
    v.stop();
}
