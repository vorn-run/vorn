//! A shell started through vornd the way the desktop starts one: created,
//! attached as soon as the answer is in, and typed into. The real vornd and
//! vorn-sessiond binaries, on a home and data directory of the test's own.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use vorn_sessiond::launch;
use vorn_term_proto::bytes::BytesFrame;

use common::{Socket, CREDENTIAL};

const VORND: &str = env!("CARGO_BIN_EXE_vornd");

const PATIENCE: Duration = Duration::from_secs(20);

/// The vorn-sessiond binary, built next to vornd with the same profile.
fn sessiond_bin() -> PathBuf {
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
struct Vornd {
    child: Child,
    port: u16,
    home: tempfile::TempDir,
}

impl Vornd {
    fn start() -> Vornd {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(VORND)
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
    fn holder(&self) -> u32 {
        let t = Instant::now();
        loop {
            let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
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

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.path().join("vornd.log")).unwrap_or_default()
    }
}

/// Kills the session holder on drop, so a failing test leaves nothing running.
struct Reap(u32);

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

/// A desktop's connection: its calls, and what the attached terminal printed.
struct Desktop {
    ws: Socket,
    next_id: u64,
    output: Vec<u8>,
    exits: Vec<Value>,
}

impl Desktop {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        common::send(&mut self.ws, frame).await;
        loop {
            let frame = self.read().await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }

    /// The next text frame, keeping the terminal output and exits seen on the way.
    async fn read(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(PATIENCE, self.ws.next())
                .await
                .expect("a frame in time")
                .expect("open")
                .unwrap();
            match msg {
                Message::Binary(b) => {
                    let f = BytesFrame::decode(&b).expect("a bytes frame");
                    self.output.extend_from_slice(f.data);
                }
                Message::Text(t) => {
                    let frame: Value = serde_json::from_str(t.as_str()).unwrap();
                    if frame["method"] == "terminal:exit" {
                        self.exits.push(frame["params"].clone());
                    }
                    return frame;
                }
                _ => {}
            }
        }
    }
}

/// The desktop's `+`: the card attaches the moment create answers, before the
/// shell is up, and must be told the shell is live -- not that it ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_shell_attached_at_once_is_live_and_echoes_what_is_typed() {
    let v = Vornd::start();
    let _reap = Reap(v.holder());
    let ws = common::connect(v.port, CREDENTIAL).await;
    let mut d = Desktop {
        ws,
        next_id: 0,
        output: Vec::new(),
        exits: Vec::new(),
    };
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap().to_owned();

    for round in 0..3 {
        let t = Instant::now();
        let created = loop {
            let answer = d.call("shell:create", json!(cwd)).await;
            if answer.get("error").is_none() {
                break answer["result"].clone();
            }
            // vornd reads its sessions once it is up.
            assert!(t.elapsed() < PATIENCE, "{answer}\n{}", v.log());
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let id = created["id"].as_str().expect("an id").to_owned();

        let attached = d.call("terminal:attach", json!({ "id": id })).await;
        assert_eq!(
            attached["result"]["live"],
            true,
            "round {round}: a shell being started was answered as ended: {attached}\n{}",
            v.log()
        );

        d.output.clear();
        let typed = d
            .call(
                "terminal:write",
                json!({ "id": id, "data": format!("echo vorn-$((6*{}))\r", 7 + round) }),
            )
            .await;
        assert!(typed.get("error").is_none(), "{typed}");
        let want = format!("vorn-{}", 6 * (7 + round));
        let t = Instant::now();
        while !String::from_utf8_lossy(&d.output).contains(&want) {
            assert!(
                t.elapsed() < PATIENCE,
                "round {round}: no echo; printed {:?}\n{}",
                String::from_utf8_lossy(&d.output),
                v.log()
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), d.read()).await;
        }
        assert!(
            !d.exits.iter().any(|e| e["id"] == json!(id)),
            "the shell ended: {:?}",
            d.exits
        );
        d.call("terminal:kill", json!(id)).await;
    }
}
