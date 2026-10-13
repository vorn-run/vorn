//! vornd started as the server on a data directory and home of the test's own, and a client of it.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use vorn_sessiond::launch;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The desktop's credential, which vornd as the server takes as its own.
pub const CREDENTIAL: &str = "vornd-test-credential";

/// vornd serving a directory of its own; killed when dropped.
pub struct Served {
    child: Child,
    pub port: u16,
    pub data: tempfile::TempDir,
    _home: tempfile::TempDir,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn serve() -> Served {
    let (data, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_vornd"))
        .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
        .env("VORN_TAILSCALE", "none")
        .arg("--data-dir")
        .arg(data.path())
        .args(["--port", "0"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("SECRET_VORN_BOOTSTRAP_TOKEN", CREDENTIAL)
        .env("VORND_KEYCHAIN", "0")
        .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("vornd starts");
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: Value = serde_json::from_str(&line).expect("vornd says where it listens");
    let port = u16::try_from(ready["port"].as_u64().unwrap()).unwrap();
    Served {
        child,
        port,
        data,
        _home: home,
    }
}

/// A socket admitted with `bearer`.
pub async fn connect(port: u16, bearer: &str) -> Socket {
    let mut req = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {bearer}").parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

pub async fn send(ws: &mut Socket, frame: Value) {
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

/// The next frame that is not a notification: an answer, or a call vornd makes.
pub async fn next(ws: &mut Socket) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("a frame in time")
            .expect("open")
            .unwrap();
        if let Message::Text(text) = msg {
            let frame: Value = serde_json::from_str(text.as_str()).unwrap();
            if frame.get("id").is_some() {
                return frame;
            }
        }
    }
}

// vornd with a session holder, as the desktop starts it.

pub const VORND: &str = env!("CARGO_BIN_EXE_vornd");

pub const PATIENCE: Duration = Duration::from_secs(20);

/// The vorn-sessiond binary, built next to vornd with the same profile.
pub fn sessiond_bin() -> PathBuf {
    let dir = Path::new(VORND).parent().expect("vornd is in a directory");
    let bin = dir.join(format!("vorn-sessiond{}", std::env::consts::EXE_SUFFIX));
    assert!(
        bin.exists(),
        "{} is missing: run `cargo build -p vorn-sessiond` with the same profile first",
        bin.display()
    );
    bin
}

/// vornd with a session holder under a home of its own; it and its holder
/// stop when dropped.
pub struct Vornd {
    child: Child,
    pub port: u16,
    pub home: tempfile::TempDir,
}

impl Vornd {
    pub fn start() -> Vornd {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(VORND)
            .env(vorn_spawn::watch::EXIT_WITH, std::process::id().to_string())
            .env("VORN_TAILSCALE", "none")
            .arg("--data-dir")
            .arg(home.path())
            .args(["--port", "0", "--exit-with-stdin", "--sessiond"])
            .arg(sessiond_bin())
            .arg("--log-file")
            .arg(home.path().join("vornd.log"))
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("SHELL", "/bin/sh")
            .env("SECRET_VORN_BOOTSTRAP_TOKEN", CREDENTIAL)
            .env("VORND_KEYCHAIN", "0")
            .env_remove("VORN_ALLOW_DEFAULT_DATA_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("vornd starts");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let ready: Value = serde_json::from_str(&line).expect("vornd says where it listens");
        let port = u16::try_from(ready["port"].as_u64().unwrap()).unwrap();
        Vornd { child, port, home }
    }

    /// The session holder's pid, once vornd has one.
    pub fn holder(&self) -> u32 {
        let t = Instant::now();
        loop {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", self.port)).unwrap();
            s.write_all(
                b"GET /vornd/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
            let mut res = String::new();
            s.read_to_string(&mut res).unwrap();
            let body: Value = serde_json::from_str(res.split_once("\r\n\r\n").unwrap().1).unwrap();
            if let Some(pid) = body["sessiond"]["current"]["pid"].as_u64() {
                return u32::try_from(pid).unwrap();
            }
            assert!(t.elapsed() < PATIENCE, "no session holder: {body}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.home.path().join("vornd.log")).unwrap_or_default()
    }
}

/// Kills the session holder on drop, so a failing test leaves nothing running.
pub struct Reap(pub u32);

impl Drop for Reap {
    fn drop(&mut self) {
        if launch::alive(self.0) {
            let _ = launch::kill(self.0);
        }
    }
}

impl Drop for Vornd {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let t = Instant::now();
        while t.elapsed() < PATIENCE {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

