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
    /// The line vornd printed once it was listening.
    ready: Value,
    log: PathBuf,
}

impl Vornd {
    fn start(home: &Path, env: &[(&str, &str)]) -> Vornd {
        Vornd::start_with(home, &sessiond_bin(), env)
    }

    /// Started with `bundled` as the sessiond shipped with the app.
    fn start_with(home: &Path, bundled: &Path, env: &[(&str, &str)]) -> Vornd {
        let log = home.join("vornd.log");
        let mut cmd = Command::new(VORND);
        cmd.arg("--data-dir")
            .arg(home)
            .args(["--port", "0", "--exit-with-stdin", "--sessiond"])
            .arg(bundled)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("VORND_KEYCHAIN", "0")
            .arg("--log-file")
            .arg(&log)
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
            ready: Value::Null,
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
        v.ready = ready;
        v
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The health check's body.
    fn health(&self) -> Value {
        self.get("/vornd/health")
    }

    /// The JSON body vornd answers a GET of `path` with.
    fn get(&self, path: &str) -> Value {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).expect("connect to vornd");
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).expect("send");
        let mut res = String::new();
        s.read_to_string(&mut res).expect("read the answer");
        let (_, body) = res.split_once("\r\n\r\n").expect("a body");
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{path} body {body:?}: {e}"))
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
    assert_eq!(
        sessiond_version(),
        env!("CARGO_PKG_VERSION"),
        "sessiond reports vornd's version"
    );
    assert_eq!(first["proto"], PROTO);
    assert_eq!(first["compatible"], true);
    let installed = launch::installed_path(home.path(), &sessiond_version());
    assert!(installed.exists(), "installed as {}", installed.display());
    assert_eq!(
        installed.parent(),
        Some(launch::installed_dir(home.path(), &sessiond_version()).as_path()),
        "each version in a directory of its own"
    );
    v.stop();
    assert!(launch::alive(pid(&first)), "sessiond outlives vornd");

    let v = Vornd::start(home.path(), &[]);
    let again = v.current();
    assert_eq!(pid(&again), pid(&first));
    assert_eq!(again["instance"], first["instance"]);
    assert_eq!(launch::running(home.path()).len(), 1, "no second one");
    v.stop();
}

/// A sessiond of an older build that cannot hand its sessions over is told
/// to drain: it is reported under `older`, a new one of this build takes its
/// place, and it exits once it holds no sessions.
#[test]
fn an_older_build_without_handoff_is_drained_and_exits() {
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
        // The label every sessiond carried before it took the app's version.
        build: "0.7.5".into(),
        // Only the drain may end it.
        idle_exit: Duration::from_secs(600),
        spool_cap: 1 << 20,
    });
    let listener = rt.block_on(async { server::bind(&old) }).expect("bind");
    // Announced as a sessiond from before handoffs.
    let mut announced = launch::running(home.path()).pop().expect("announced");
    announced.handoff = None;
    launch::announce(home.path(), &announced).unwrap();
    let serving = rt.spawn(server::serve(Arc::clone(&old), listener));

    let v = Vornd::start(home.path(), &[]);
    let current = v.current();
    reap.add(pid(&current));
    let holder = v.wait_for("the older one reported", |s| {
        s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    let older = holder["older"].as_array().unwrap();
    assert_eq!(older.len(), 1, "{holder}");
    assert_eq!(older[0]["build"], "0.7.5");
    assert_eq!(older[0]["instance"], "1d");
    assert_eq!(older[0]["compatible"], true);
    assert_eq!(older[0]["sessions"], 0);
    assert_eq!(older[0]["handedOff"], false);
    assert_ne!(current["instance"], "1d");
    #[cfg(unix)]
    assert!(!old.handed_off());

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

/// The sessiond binary copied into an app bundle of its own under `home`.
fn bundle(home: &Path, name: &str) -> PathBuf {
    let dir = home.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join(format!("vorn-sessiond{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(sessiond_bin(), &bin).unwrap();
    bin
}

/// The same build of this version from another bundle, as after the app is
/// moved or reinstalled, keeps the sessiond already running it.
#[test]
fn the_same_build_from_another_bundle_keeps_its_sessiond() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let v = Vornd::start_with(home.path(), &bundle(home.path(), "a"), &[]);
    let first = v.current();
    reap.add(pid(&first));
    v.stop();

    let v = Vornd::start_with(home.path(), &bundle(home.path(), "b"), &[]);
    let again = v.current();
    assert_eq!(again["instance"], first["instance"]);
    assert_eq!(v.health()["sessiond"]["older"], serde_json::json!([]));
    assert_eq!(launch::running(home.path()).len(), 1, "no second one");
    let installs = std::fs::read_dir(home.path().join("bin")).unwrap().count();
    assert_eq!(installs, 1, "installed once");
    v.stop();
}

/// A local rebuild with the version unchanged is a new build: it is
/// installed beside the stale one, which hands over its sessions (none
/// here) and exits.
#[test]
fn a_rebuild_of_the_same_version_replaces_its_sessiond() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let bundled = bundle(home.path(), "app");
    let v = Vornd::start_with(home.path(), &bundled, &[]);
    let first = v.current();
    reap.add(pid(&first));
    v.stop();
    let stale = launch::installed_path(home.path(), &sessiond_version());

    // Trailing bytes change the binary but not what it runs as.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&bundled)
        .unwrap();
    f.write_all(b"rebuilt").unwrap();
    drop(f);

    let v = Vornd::start_with(home.path(), &bundled, &[]);
    let holder = v.wait_for("the stale one reported", |s| {
        s["current"].is_object() && s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    let current = &holder["current"];
    reap.add(pid(current));
    assert_ne!(current["instance"], first["instance"]);
    assert_eq!(current["build"], first["build"], "the same version");
    let older = holder["older"].as_array().unwrap();
    assert_eq!(older.len(), 1, "{holder}");
    assert_eq!(older[0]["instance"], first["instance"]);
    assert_eq!(older[0]["compatible"], true);
    assert_eq!(older[0]["handedOff"], cfg!(unix));
    assert!(
        gone_within(pid(&first), PATIENCE),
        "the older sessiond did not exit: {}",
        v.log_text()
    );

    let running = launch::running(home.path());
    assert_eq!(running.len(), 1);
    let exe = running[0].exe.clone().expect("it names its binary");
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        std::fs::read(&bundled).unwrap()
    );
    assert!(stale.exists(), "the stale install is left where it was");
    assert!(!running[0].runs(&stale));
    v.stop();

    // Started again, the rebuild finds its own install and sessiond.
    let v = Vornd::start_with(home.path(), &bundled, &[]);
    assert_eq!(v.current()["instance"], current["instance"]);
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
        exe: None,
        handoff: None,
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

/// The session engine's report, once vornd is connected to its sessiond:
/// no sessions yet, and the connection up.
#[cfg(feature = "engine")]
#[test]
fn the_session_report_is_served() {
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();

    let v = Vornd::start(home.path(), &[]);
    reap.add(pid(&v.current()));
    let t = Instant::now();
    let report = loop {
        let r = v.get("/vornd/sessions");
        if r["connected"] == true {
            break r;
        }
        assert!(t.elapsed() < PATIENCE, "not connected: {r}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(report["sessions"], serde_json::json!([]));
    let with_digests = v.get("/vornd/sessions?digest=1");
    assert_eq!(with_digests["connected"], true, "{with_digests}");
    assert_eq!(with_digests["sessions"], serde_json::json!([]));
    v.stop();
}

/// vornd sweeps the sockets that killed vornds left under its home, keeps
/// another home's, and takes its own endpoints back when it stops.
#[cfg(unix)]
#[test]
fn run_holds_only_live_sockets() {
    use std::os::unix::net::UnixListener;
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();
    let dead = {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    };
    let stale = |home: &Path| {
        let run = home.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let paths = [
            run.join(format!("vornd-app-{dead}.sock")),
            run.join(format!("vornd-grid-{dead}.sock")),
        ];
        for p in &paths {
            drop(UnixListener::bind(p).unwrap());
        }
        paths
    };
    let mine = stale(home.path());
    let theirs = stale(other.path());

    let v = Vornd::start(home.path(), &[]);
    reap.add(pid(&v.current()));
    for p in &mine {
        assert!(!p.exists(), "{} is left", p.display());
    }
    for p in &theirs {
        assert!(p.exists(), "{} was removed", p.display());
    }
    // Only the session engine serves the grid endpoint.
    let own: Vec<PathBuf> = v.ready["grid"]
        .as_str()
        .map(PathBuf::from)
        .into_iter()
        .collect();
    assert_eq!(own.len(), if cfg!(feature = "engine") { 1 } else { 0 });
    for p in &own {
        assert!(p.exists(), "{} is missing", p.display());
    }
    v.stop();
    for p in &own {
        assert!(!p.exists(), "{} is left after a stop", p.display());
    }
}

/// A blocking client of one sessiond, as vornd: keeps each session's
/// records and acks them.
#[cfg(unix)]
struct Client {
    sock: std::os::unix::net::UnixStream,
    frames: vorn_sessiond_wire::FrameReader,
    logs: std::collections::HashMap<String, Vec<vorn_term_proto::Entry>>,
}

#[cfg(unix)]
impl Client {
    fn hello(endpoint: &str) -> (Client, vorn_sessiond_wire::Welcome) {
        use vorn_sessiond_wire::{Hello, ToSessiond, ToVornd};
        let sock = std::os::unix::net::UnixStream::connect(endpoint).expect("connect");
        sock.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut c = Client {
            sock,
            frames: Default::default(),
            logs: Default::default(),
        };
        c.send(&ToSessiond::Hello(Hello {
            proto_min: PROTO,
            proto_max: PROTO,
            vornd_instance: 7,
            vornd_build: "test".into(),
        }));
        match c.recv() {
            ToVornd::Welcome(w) => (c, w),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    fn send(&mut self, m: &vorn_sessiond_wire::ToSessiond) {
        use vorn_sessiond_wire::Message;
        self.sock.write_all(&m.encode()).expect("send");
    }

    fn recv(&mut self) -> vorn_sessiond_wire::ToVornd {
        use vorn_sessiond_wire::{Ack, ToSessiond, ToVornd};
        let mut buf = vec![0u8; 64 << 10];
        loop {
            if let Some(m) = self.frames.read::<ToVornd>().expect("well-formed") {
                if let ToVornd::Entries(e) = &m {
                    let delivered = e.entries.last().expect("non-empty").after();
                    self.logs
                        .entry(e.session.clone())
                        .or_default()
                        .extend(e.entries.iter().cloned());
                    self.send(&ToSessiond::Ack(Ack {
                        session: e.session.clone(),
                        delivered,
                    }));
                }
                return m;
            }
            let n = self.sock.read(&mut buf).expect("sessiond answers");
            assert_ne!(n, 0, "sessiond closed the connection");
            self.frames.push(&buf[..n]);
        }
    }

    /// A shell that echoes each line it reads, attached from its start.
    fn spawn_echo(&mut self) -> String {
        use vorn_sessiond_wire::{Attach, AttachFrom, Io, Spawn, SpawnSpec, ToSessiond, ToVornd};
        self.send(&ToSessiond::Spawn(Spawn {
            req: 1,
            spec: SpawnSpec {
                argv: ["sh", "-c", r#"while read l; do echo "got:$l"; done"#]
                    .map(String::from)
                    .to_vec(),
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: Vec::new(),
                io: Io::Pty { cols: 80, rows: 24 },
                ring_bytes: None,
            },
        }));
        let id = loop {
            match self.recv() {
                ToVornd::Spawned(s) => break s.session,
                ToVornd::Failed(f) => panic!("spawn failed: {}", f.error),
                _ => {}
            }
        };
        self.send(&ToSessiond::Attach(Attach {
            session: id.clone(),
            from: AttachFrom::SessionStart,
        }));
        id
    }

    /// Types `line` and reads until the shell echoed it back.
    fn echo(&mut self, session: &str, seq: u64, line: &str) {
        use vorn_sessiond_wire::{ToSessiond, Write as In};
        self.send(&ToSessiond::Write(In {
            session: session.into(),
            input_seq: seq,
            bytes: format!("{line}\n").into_bytes(),
        }));
        let want = format!("got:{line}");
        while !String::from_utf8_lossy(&self.output(session)).contains(&want) {
            self.recv();
        }
    }

    fn output(&self, session: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for e in self.logs.get(session).into_iter().flatten() {
            if let vorn_term_proto::Record::Data { bytes, .. } = &e.rec {
                out.extend(bytes);
            }
        }
        out
    }

    fn cursor(&self, session: &str) -> vorn_term_proto::Cursor {
        self.logs[session].last().expect("records").after()
    }
}

/// An older sessiond of another build, in this process, holding one live
/// shell that has answered a line.
#[cfg(unix)]
struct Older {
    rt: tokio::runtime::Runtime,
    d: Arc<Sessiond>,
    serving: tokio::task::JoinHandle<std::io::Result<()>>,
    client: Client,
    session: String,
}

#[cfg(unix)]
impl Older {
    fn start(home: &Path) -> Older {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let d = Sessiond::new(Config {
            home: home.to_owned(),
            instance: 0x01d,
            build: "0.7.5".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 1 << 20,
        });
        let listener = rt.block_on(async { server::bind(&d) }).expect("bind");
        let serving = rt.spawn(server::serve(Arc::clone(&d), listener));
        let (mut client, _) = Client::hello(&d.endpoint());
        let session = client.spawn_echo();
        client.echo(&session, 1, "one");
        Older {
            rt,
            d,
            serving,
            client,
            session,
        }
    }

    fn pid(&self) -> u32 {
        let (_, w) = Client::hello(&self.d.endpoint());
        w.sessions
            .iter()
            .find(|s| s.session == self.session)
            .expect("listed")
            .pid
    }

    fn exits_within(self, d: Duration) -> bool {
        let t = Instant::now();
        while !self.serving.is_finished() {
            if t.elapsed() > d {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.rt.block_on(self.serving).unwrap().is_ok()
    }
}

/// An older build's sessiond hands its live sessions to the new one: the
/// shell carries on there from the cursor its last client reached, and the
/// older one stays only until it has reaped the shell.
#[cfg(unix)]
#[test]
fn an_older_build_hands_its_live_sessions_over() {
    use vorn_sessiond_wire::{Attach, AttachFrom, ToSessiond};
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();
    let old = Older::start(home.path());
    let id = old.session.clone();
    let shell = old.pid();

    let v = Vornd::start(home.path(), &[]);
    let holder = v.wait_for("the older one reported", |s| {
        s["current"].is_object() && s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    let current = holder["current"].clone();
    reap.add(pid(&current));
    let older = &holder["older"][0];
    assert_eq!(older["instance"], "1d");
    assert_eq!(older["handedOff"], true, "{holder}\n{}", v.log_text());
    assert_eq!(older["sessions"], 0);
    assert!(old.d.handed_off() && !old.d.holds(&id));
    assert!(launch::alive(shell), "the shell runs on");
    v.stop();

    // The new sessiond holds the shell and continues its log from where the
    // older one's client stopped, with nothing in between.
    let endpoint = launch::running(home.path())
        .into_iter()
        .find(|i| i.pid == pid(&current))
        .expect("the current one is announced")
        .endpoint;
    let (mut c, w) = Client::hello(&endpoint);
    let info = w
        .sessions
        .iter()
        .find(|s| s.session == id)
        .expect("adopted");
    assert_eq!(info.pid, shell);
    let at = old.client.cursor(&id);
    c.send(&ToSessiond::Attach(Attach {
        session: id.clone(),
        from: AttachFrom::Cursor(at),
    }));
    c.echo(&id, 2, "two");
    let first = c.logs[&id].first().expect("records");
    assert!(at.is_followed_by(&first.hdr), "{at:?} then {:?}", first.hdr);
    let mut all = old.client.output(&id);
    all.extend(c.output(&id));
    let text = String::from_utf8_lossy(&all);
    assert!(
        text.contains("got:one") && text.contains("got:two"),
        "{text}"
    );

    launch::kill(shell).unwrap();
    assert!(
        old.exits_within(PATIENCE),
        "the older sessiond stays after reaping the shell"
    );
}

/// A handoff that fails leaves every session on the older sessiond, which
/// is then drained as before handoffs.
#[cfg(unix)]
#[test]
fn a_failed_handoff_drains_the_older_build_instead() {
    use vorn_sessiond::server::{Fault, Step};
    let home = tempfile::tempdir().unwrap();
    let mut reap = Reap::default();
    let old = Older::start(home.path());
    old.d.inject(Fault {
        step: Step::Send,
        stall: false,
    });
    let id = old.session.clone();
    let shell = old.pid();

    let v = Vornd::start(home.path(), &[]);
    let holder = v.wait_for("the older one reported", |s| {
        s["current"].is_object() && s["older"].as_array().is_some_and(|o| !o.is_empty())
    });
    reap.add(pid(&holder["current"]));
    let older = &holder["older"][0];
    assert_eq!(older["handedOff"], false, "{holder}");
    assert_eq!(older["sessions"], 1);
    assert!(!old.d.handed_off() && old.d.holds(&id));
    assert!(
        v.log_text().contains("could not hand over"),
        "{}",
        v.log_text()
    );

    v.stop();

    // Still the older one's, carrying on from where its client stopped.
    use vorn_sessiond_wire::{Attach, AttachFrom, ToSessiond};
    let at = old.client.cursor(&id);
    let (mut c, _) = Client::hello(&old.d.endpoint());
    c.send(&ToSessiond::Attach(Attach {
        session: id.clone(),
        from: AttachFrom::Cursor(at),
    }));
    c.echo(&id, 2, "two");
    let first = c.logs[&id].first().expect("records");
    assert!(at.is_followed_by(&first.hdr), "{at:?} then {:?}", first.hdr);

    // It exits once its last session has ended and been released.
    launch::kill(shell).unwrap();
    while !c.logs[&id]
        .iter()
        .any(|e| matches!(e.rec, vorn_term_proto::Record::Exit { .. }))
    {
        c.recv();
    }
    c.send(&ToSessiond::Release(vorn_sessiond_wire::SessionRef {
        session: id.clone(),
    }));
    assert!(old.exits_within(PATIENCE), "the drained sessiond stays");
}
