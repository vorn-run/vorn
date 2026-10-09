//! vornd started as the server on a data directory and home of the test's own, and a client of it.

#![allow(dead_code)]

use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
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
