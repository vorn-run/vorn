//! A test vornd: its own HOME and data directory under a temporary path,
//! never the user's `~/.vorn`, stopped with the session holder it started
//! when dropped.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const PATIENCE: Duration = Duration::from_secs(30);

pub struct TestVornd {
    child: Child,
    pub home: PathBuf,
    /// The grid endpoint vornd's ready line names.
    pub grid: String,
    port: u16,
}

/// Where vornd and vorn-sessiond are: `VORN_APP_TEST_BIN`, else the
/// profile directory this test was built into.
fn bin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VORN_APP_TEST_BIN") {
        return Some(PathBuf::from(dir));
    }
    // target/<profile>/deps/<test> -> target/<profile>
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.to_path_buf())
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A fresh, short directory: Unix socket paths have a 104-byte limit.
fn temp_home() -> std::io::Result<PathBuf> {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = base.join(format!("va-{}-{n}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl TestVornd {
    /// A started vornd with its holder connected, or `None` (said on
    /// stderr) when the binaries are not built.
    pub fn start() -> Option<TestVornd> {
        let dir = bin_dir()?;
        let (vornd, sessiond) = (exe(&dir, "vornd"), exe(&dir, "vorn-sessiond"));
        if !vornd.is_file() || !sessiond.is_file() {
            eprintln!("skipped: no vornd/vorn-sessiond in {}", dir.display());
            return None;
        }
        let home = temp_home().expect("temp dir");
        let mut child = Command::new(&vornd)
            .arg("--data-dir")
            .arg(&home)
            .args(["--port", "0", "--exit-with-stdin"])
            .arg("--sessiond")
            .arg(&sessiond)
            .arg("--log-file")
            .arg(home.join("vornd.log"))
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("VORN_HOME", &home)
            .env("SHELL", "/bin/sh")
            .env("VORND_KEYCHAIN", "0")
            .env("VORN_SESSIOND_IDLE_EXIT", "2")
            .env_remove("VORN_DATA_DIR")
            .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("vornd starts");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut v = TestVornd {
            child,
            home,
            grid: String::new(),
            port: 0,
        };
        let line = rx.recv_timeout(PATIENCE).unwrap_or_default();
        let ready: Value = serde_json::from_str(line.trim())
            .unwrap_or_else(|e| panic!("ready line {line:?}: {e}\n{}", v.log()));
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .expect("port");
        v.grid = ready["grid"].as_str().expect("grid endpoint").to_owned();
        v.wait_connected();
        Some(v)
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("vornd.log")).unwrap_or_default()
    }

    fn get(&self, path: &str) -> Option<Value> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).ok()?;
        s.set_read_timeout(Some(PATIENCE)).ok()?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).ok()?;
        let mut res = String::new();
        s.read_to_string(&mut res).ok()?;
        serde_json::from_str(res.split_once("\r\n\r\n")?.1).ok()
    }

    /// Terminals cannot start until the session holder is connected.
    fn wait_connected(&self) {
        let t = Instant::now();
        loop {
            let health = self.get("/vornd/health").unwrap_or_default();
            let sessions = self.get("/vornd/sessions").unwrap_or_default();
            if health["sessiond"]["current"].is_object() && sessions["connected"] == true {
                return;
            }
            assert!(
                t.elapsed() < PATIENCE,
                "sessiond never connected:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn holder_pids(&self) -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir(self.home.join("run")) else {
            return Vec::new();
        };
        dir.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "info"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("pid=").and_then(|p| p.parse().ok()))
            })
            .collect()
    }
}

fn kill(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: kill(2) takes any pid; a stale one fails with ESRCH.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Drop for TestVornd {
    fn drop(&mut self) {
        let holders = self.holder_pids();
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in holders {
            kill(pid);
        }
        std::thread::sleep(Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Polls `f` until it answers, or fails the test with `what`.
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let t = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(t.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}
