//! A test vornd: its own HOME and `--data-dir` under a temporary path, never the user's `~/.vorn`, stopped with its session holder when dropped.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const PATIENCE: Duration = Duration::from_secs(30);

pub struct TestVornd {
    child: Child,
    pub home: PathBuf,
    port: u16,
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A fresh, short directory: Unix socket paths have a 104-byte limit.
fn temp_home() -> Result<PathBuf, String> {
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = base.join(format!("vwb-{}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

impl TestVornd {
    /// vornd and vorn-sessiond from `bin`, started on a temporary HOME and data directory.
    pub fn start(bin: &Path) -> Result<TestVornd, String> {
        let (vornd, sessiond) = (exe(bin, "vornd"), exe(bin, "vorn-sessiond"));
        if !vornd.is_file() || !sessiond.is_file() {
            return Err(format!("no vornd and vorn-sessiond in {}", bin.display()));
        }
        let home = temp_home()?;
        let mut child = Command::new(&vornd)
            .env("VORN_EXIT_WITH", std::process::id().to_string())
            .env("VORN_TAILSCALE", "none")
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
            .map_err(|e| format!("vornd: {e}"))?;
        let stdout = child.stdout.take().ok_or("vornd has no stdout")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let mut v = TestVornd {
            child,
            home,
            port: 0,
        };
        let line = rx.recv_timeout(PATIENCE).unwrap_or_default();
        let ready: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("ready line {line:?}: {e}\n{}", v.log()))?;
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or("the ready line names no port")?;
        v.wait_connected()?;
        Ok(v)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("vornd.log")).unwrap_or_default()
    }

    fn get(&self, path: &str) -> Option<Value> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).ok()?;
        s.set_read_timeout(Some(PATIENCE)).ok()?;
        // vornd's reports want the local credential it wrote beside its data.
        let token = std::fs::read_to_string(self.home.join("local-token")).unwrap_or_default();
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            token.trim()
        );
        s.write_all(req.as_bytes()).ok()?;
        let mut res = String::new();
        s.read_to_string(&mut res).ok()?;
        serde_json::from_str(res.split_once("\r\n\r\n")?.1).ok()
    }

    fn wait_connected(&self) -> Result<(), String> {
        let t = Instant::now();
        loop {
            let health = self.get("/vornd/health").unwrap_or_default();
            let sessions = self.get("/vornd/sessions").unwrap_or_default();
            if health["sessiond"]["current"].is_object() && sessions["connected"] == true {
                return Ok(());
            }
            if t.elapsed() > PATIENCE {
                return Err(format!("sessiond never connected:\n{}", self.log()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn holder_pids(&self) -> Vec<i32> {
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

impl Drop for TestVornd {
    fn drop(&mut self) {
        let holders = self.holder_pids();
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(unix)]
        for pid in holders {
            // SAFETY: kill(2) takes any pid; a stale one fails with ESRCH.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        for pid in holders {
            let _ = Command::new("taskkill")
                .args(["/F", "/PID", &pid.to_string()])
                .output();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
