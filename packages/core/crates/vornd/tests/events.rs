//! The public events end to end: a linked extension subscribed to session,
//! mark and agent events receives each once and in order, from a shell
//! started through vornd and a hook an agent posts. The real vornd and
//! vorn-sessiond binaries, on a home and data directory of the test's own.
//! Skipped without `node`.

#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use common::{Reap, Socket, Vornd, CREDENTIAL, PATIENCE};

const PROGRAM: &str = include_str!("../../extensions/tests/fixtures/program.mjs");

const MANIFEST: &str = r#"id = "test.events"
name = "Events"
version = "1.0.0"
api = "vorn:extension@1.0"

[service]
command = ["node", "program.mjs"]
events = ["session.created", "session.mark", "session.exited", "agent.hook", "agent.conversation"]
"#;

fn node() -> bool {
    let found = std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipped: no node on PATH");
    }
    found
}

struct Client {
    ws: Socket,
    next_id: u64,
}

impl Client {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        common::send(&mut self.ws, frame).await;
        loop {
            let frame = common::next(&mut self.ws).await;
            if frame["id"] == json!(id) {
                return frame;
            }
        }
    }

    /// Calls `method` until it answers without an error: vornd reads its sessions once it is up.
    async fn call_ok(&mut self, method: &str, params: Value, v: &Vornd) -> Value {
        let t = Instant::now();
        loop {
            let answer = self.call(method, params.clone()).await;
            if answer.get("error").is_none() {
                return answer["result"].clone();
            }
            assert!(t.elapsed() < PATIENCE, "{method}: {answer}\n{}", v.log());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The events the extension received about `session`, once `done` holds for them.
    async fn events_until(
        &mut self,
        session: &str,
        v: &Vornd,
        done: impl Fn(&[Value]) -> bool,
    ) -> Vec<Value> {
        let t = Instant::now();
        loop {
            let all = self.call_ok("test.events:events", json!({}), v).await;
            let mine: Vec<Value> = all
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["data"]["sessionId"] == json!(session))
                .cloned()
                .collect();
            if done(&mine) {
                return mine;
            }
            assert!(t.elapsed() < PATIENCE, "only {mine:?}\n{}", v.log());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Posts one hook event to the endpoint vornd wrote under its home.
fn post_hook(v: &Vornd, body: &Value) {
    let vorn = v.home.path().join(".vorn");
    let t = Instant::now();
    let (port, token) = loop {
        let port = std::fs::read_to_string(vorn.join("port"));
        let token = std::fs::read_to_string(vorn.join("token"));
        if let (Ok(p), Ok(t)) = (port, token) {
            break (p.trim().to_owned(), t.trim().to_owned());
        }
        assert!(t.elapsed() < PATIENCE, "no hook endpoint\n{}", v.log());
        std::thread::sleep(Duration::from_millis(20));
    };
    let body = body.to_string();
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    write!(
        s,
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut res = String::new();
    s.read_to_string(&mut res).unwrap();
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
}

fn names(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            let name = e["name"].as_str().unwrap_or_default();
            match e["data"]["phase"].as_str() {
                Some(phase) => format!("{name}:{phase}"),
                None => name.to_owned(),
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_subscribed_extension_hears_each_session_mark_and_agent_event_once_in_order() {
    if !node() {
        return;
    }
    let v = Vornd::start();
    let _reap = Reap(v.holder());
    let mut c = Client {
        ws: common::connect(v.port, CREDENTIAL).await,
        next_id: 0,
    };
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    std::fs::write(folder.path().join("program.mjs"), PROGRAM).unwrap();
    c.call_ok("extension:link", json!({ "path": folder.path() }), &v)
        .await;

    let cwd = tempfile::tempdir().unwrap();
    let created = c
        .call_ok("shell:create", json!(cwd.path().to_str().unwrap()), &v)
        .await;
    let id = created["id"].as_str().expect("an id").to_owned();
    c.call_ok("terminal:attach", json!({ "id": id }), &v).await;
    // The shell prints the marks; what is typed carries `\033`, not the escape itself.
    let marks = r"printf '\033]133;A\007\033]133;C\007\033]133;D;3\007'";
    c.call_ok(
        "terminal:write",
        json!({ "id": id, "data": format!("{marks}\r") }),
        &v,
    )
    .await;
    let seen = c
        .events_until(&id, &v, |e| names(e).contains(&"session.mark:end".into()))
        .await;
    assert_eq!(seen.last().unwrap()["data"]["exitCode"], 3);

    post_hook(
        &v,
        &json!({
            "hook_event_name": "SessionStart", "session_id": "conversation-1",
            "cwd": cwd.path(), "vorn_terminal_id": id,
        }),
    );
    c.events_until(&id, &v, |e| names(e).contains(&"agent.conversation".into()))
        .await;
    c.call_ok("terminal:kill", json!(id), &v).await;
    let seen = c
        .events_until(&id, &v, |e| names(e).contains(&"session.exited".into()))
        .await;

    assert_eq!(
        names(&seen),
        [
            "session.created",
            "session.mark:prompt",
            "session.mark:start",
            "session.mark:end",
            "agent.hook",
            "agent.conversation",
            "session.exited",
        ],
        "{seen:#?}"
    );
    assert_eq!(seen[5]["data"]["conversationId"], "conversation-1");
    assert_eq!(seen[0]["data"]["kind"], "terminal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extensions_program_stops_with_vornd() {
    if !node() {
        return;
    }
    let v = Vornd::start();
    let _reap = Reap(v.holder());
    let mut c = Client {
        ws: common::connect(v.port, CREDENTIAL).await,
        next_id: 0,
    };
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    // A program that stays up after its stdin closes: only vornd stopping it ends it.
    let lingering = format!("{PROGRAM}\nsetInterval(() => {{}}, 1000)\n");
    std::fs::write(folder.path().join("program.mjs"), lingering).unwrap();
    c.call_ok("extension:link", json!({ "path": folder.path() }), &v)
        .await;
    let echo = c.call_ok("test.events:echo", json!({}), &v).await;
    let pid = u32::try_from(echo["pid"].as_u64().expect("a pid")).unwrap();
    assert!(vorn_sessiond::launch::alive(pid));
    drop(c);
    // Dropping it closes vornd's stdin and waits for it to exit.
    drop(v);
    assert!(
        !vorn_sessiond::launch::alive(pid),
        "the extension's program outlived vornd"
    );
}
