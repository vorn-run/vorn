//! A test vornd: its own data directory under a temporary path (never the
//! user's), started with `--debug-spawn` so the bench can start sessions over
//! the WebSocket, and stopped with everything it started when dropped.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const PATIENCE: Duration = Duration::from_secs(30);

pub struct TestVornd {
    child: Child,
    pub home: PathBuf,
    pub port: u16,
    /// The grid endpoint from the ready line.
    pub grid: String,
}

/// Where the release binaries are: `VORN_SPIKE_BIN`, else `.deps/bin` next to
/// the spike's workspaces.
fn bin_dir() -> PathBuf {
    std::env::var_os("VORN_SPIKE_BIN").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../.deps/bin"),
        PathBuf::from,
    )
}

fn exe(name: &str) -> PathBuf {
    bin_dir().join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
    let dir = base.join(format!("vu-{}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl TestVornd {
    pub fn start() -> Result<TestVornd, String> {
        let home = temp_home().map_err(|e| format!("temp dir: {e}"))?;
        let log = home.join("vornd.log");
        let mut child = Command::new(exe("vornd"))
            .arg("--data-dir")
            .arg(&home)
            .args(["--port", "0", "--exit-with-stdin", "--debug-spawn"])
            .arg("--sessiond")
            .arg(exe("vorn-sessiond"))
            .arg("--log-file")
            .arg(&log)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("VORN_HOME", &home)
            .env("VORND_KEYCHAIN", "0")
            .env("VORN_SESSIOND_IDLE_EXIT", "2")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("start {}: {e}", exe("vornd").display()))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
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
            grid: String::new(),
        };
        let line = rx
            .recv_timeout(PATIENCE)
            .map_err(|_| format!("vornd printed no ready line: {}", v.log()))?;
        let ready: Value =
            serde_json::from_str(line.trim()).map_err(|e| format!("ready line {line:?}: {e}"))?;
        v.port = ready["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or("no port")?;
        v.grid = ready["grid"].as_str().ok_or("no grid endpoint")?.to_owned();
        v.wait_connected()?;
        Ok(v)
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("vornd.log")).unwrap_or_default()
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(PATIENCE))
            .map_err(|e| e.to_string())?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut res = String::new();
        s.read_to_string(&mut res).map_err(|e| e.to_string())?;
        let (_, body) = res.split_once("\r\n\r\n").ok_or("no body")?;
        serde_json::from_str(body).map_err(|e| format!("{path}: {e}"))
    }

    fn wait_connected(&self) -> Result<(), String> {
        let t = Instant::now();
        loop {
            let health = self.get("/vornd/health")?;
            let sessions = self.get("/vornd/sessions")?;
            if health["sessiond"]["current"].is_object() && sessions["connected"] == true {
                return Ok(());
            }
            if t.elapsed() > PATIENCE {
                return Err(format!("sessiond never connected: {}", self.log()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Starts one session per argv at `cols`x`rows`; answers their ids.
    pub fn spawn(
        &self,
        argvs: &[Vec<String>],
        cols: u16,
        rows: u16,
    ) -> Result<Vec<String>, String> {
        use tungstenite::client::IntoClientRequest;
        let token = std::fs::read_to_string(self.home.join("local-token"))
            .map_err(|e| format!("local-token: {e}"))?;
        let mut req = format!("ws://127.0.0.1:{}/ws", self.port)
            .into_client_request()
            .map_err(|e| e.to_string())?;
        let auth = format!("Bearer {}", token.trim())
            .parse()
            .map_err(|_| "bad token")?;
        req.headers_mut().insert("authorization", auth);
        let tcp = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        tcp.set_read_timeout(Some(PATIENCE))
            .map_err(|e| e.to_string())?;
        let (mut ws, _) = tungstenite::client(req, tcp).map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for (i, argv) in argvs.iter().enumerate() {
            let id = i as u64 + 1;
            let body = json!({"jsonrpc": "2.0", "id": id, "method": "vornd:spawn", "params": {
                "name": format!("pane-{i}"), "argv": argv, "cwd": self.home,
                "cols": cols, "rows": rows,
            }});
            ws.send(tungstenite::Message::text(body.to_string()))
                .map_err(|e| e.to_string())?;
            loop {
                let msg = ws.read().map_err(|e| e.to_string())?;
                let tungstenite::Message::Text(text) = msg else {
                    continue;
                };
                let v: Value = serde_json::from_str(text.as_str()).map_err(|e| e.to_string())?;
                if v["id"].as_u64() != Some(id) {
                    continue;
                }
                if let Some(e) = v.get("error") {
                    return Err(format!("spawn: {e}"));
                }
                ids.push(v["result"]["id"].as_str().ok_or("no id")?.to_owned());
                break;
            }
        }
        let _ = ws.close(None);
        Ok(ids)
    }

    fn sessiond_pids(&self) -> Vec<u32> {
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
        let holders = self.sessiond_pids();
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in holders {
            crate::metrics::kill(pid);
        }
        std::thread::sleep(Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&self.home);
    }
}
