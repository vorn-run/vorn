//! The server link on a real sessiond, with a stand-in for the Node server:
//! the server starts, feeds and signals its sessions through vornd, hears
//! every record and effect, and after vornd dies hears again exactly what
//! may be repeated (RC §7, TP-T27 at the link).

#![cfg(all(feature = "engine", unix))]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use vorn_engine::Config;
use vorn_sessiond::server::{self, Sessiond};
use vornd::engine::Engine;
use vornd::holder::{self, Holder};
use vornd::node_link::{self, LinkConfig, IDENTIFY};

const PATIENCE: Duration = Duration::from_secs(20);
const TOKEN: &str = "link-test-credential";

struct Rig {
    d: Arc<Sessiond>,
    _home: tempfile::TempDir,
    _serving: tokio::task::JoinHandle<std::io::Result<()>>,
    /// Where the stand-in server listens for the link.
    listener: TcpListener,
}

/// One vornd: the engine on sessiond and the link to the server. Aborting
/// both kills it, as a crash would.
struct Vornd {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Vornd {
    async fn kill(self) {
        for t in self.tasks {
            t.abort();
            let _ = t.await;
        }
    }
}

impl Rig {
    async fn start() -> Rig {
        let home = tempfile::tempdir().unwrap();
        let d = Sessiond::new(server::Config {
            home: home.path().to_path_buf(),
            instance: 0x11c,
            build: "test".into(),
            idle_exit: Duration::from_secs(600),
            spool_cap: 64 << 20,
        });
        let listener = server::bind(&d).unwrap();
        let serving = tokio::spawn(server::serve(Arc::clone(&d), listener));
        Rig {
            d,
            _home: home,
            _serving: serving,
            listener: TcpListener::bind("127.0.0.1:0").await.unwrap(),
        }
    }

    fn vornd(&self) -> Vornd {
        let engine = Engine::new(Config {
            build: "test".into(),
            ..Config::default()
        });
        let holder = Holder::with_engine(Arc::clone(&engine));
        let endpoint = self.d.endpoint();
        let engine_task = tokio::spawn(async move {
            let _ = holder::connect(&endpoint, &holder).await;
        });
        let cfg = LinkConfig {
            upstream: self.listener.local_addr().unwrap(),
            token: TOKEN.into(),
        };
        let (linked, _) = watch::channel(false);
        let link_task = tokio::spawn(node_link::run(Arc::clone(&engine), cfg, linked));
        Vornd {
            tasks: vec![engine_task, link_task],
        }
    }

    /// The next link vornd opens, once it has claimed it.
    async fn node(&self) -> Node {
        let (stream, _) = tokio::time::timeout(PATIENCE, self.listener.accept())
            .await
            .expect("vornd never dialled")
            .unwrap();
        Node::accept(stream).await
    }
}

/// The Node server's side of the link, as far as these tests need it:
/// it accepts the identify call and remembers every record and effect.
struct Node {
    tx: mpsc::UnboundedSender<Message>,
    rx: mpsc::UnboundedReceiver<Value>,
    next_id: i64,
    /// Every notification heard, in order.
    notes: Vec<Value>,
    /// Effects by id, and how many times each was heard.
    effects: HashMap<String, (Value, u32)>,
    /// Each session's output as text, records deduplicated by rseq the way
    /// the server does it.
    output: HashMap<String, String>,
    seen: HashSet<(String, u32, u64)>,
}

impl Node {
    async fn accept(stream: TcpStream) -> Node {
        let check = |req: &Request, res: Response| {
            let auth = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());
            assert_eq!(auth, Some(format!("Bearer {TOKEN}").as_str()));
            Ok(res)
        };
        let ws = tokio_tungstenite::accept_hdr_async(stream, check)
            .await
            .unwrap();
        let (mut sink, mut source) = ws.split();
        let (tx, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let (in_tx, rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            while let Some(m) = outgoing.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
        });
        let answer = tx.clone();
        let (claimed, was_claimed) = tokio::sync::oneshot::channel::<()>();
        let mut claimed = Some(claimed);
        tokio::spawn(async move {
            while let Some(frame) = source.next().await {
                let t = match frame {
                    Ok(Message::Text(t)) => t,
                    other => {
                        eprintln!("link frame: {other:?}");
                        break;
                    }
                };
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                if v["method"] == IDENTIFY {
                    assert_eq!(v["params"]["protocol"], 1);
                    let ok = json!({ "jsonrpc": "2.0", "id": v["id"], "result": { "ok": true } });
                    let _ = answer.send(Message::text(ok.to_string()));
                    if let Some(c) = claimed.take() {
                        let _ = c.send(());
                    }
                    continue;
                }
                if in_tx.send(v).is_err() {
                    break;
                }
            }
        });
        // The real server greets first; the link must look past it.
        let hello = json!({ "jsonrpc": "2.0", "method": "server:hello", "params": {} });
        let _ = tx.send(Message::text(hello.to_string()));
        // The server calls nothing before the link is claimed.
        tokio::time::timeout(PATIENCE, was_claimed)
            .await
            .expect("vornd never claimed the link")
            .expect("the link closed before it was claimed");
        Node {
            tx,
            rx,
            next_id: 0,
            notes: Vec::new(),
            effects: HashMap::new(),
            output: HashMap::new(),
            seen: HashSet::new(),
        }
    }

    fn heard(&mut self, v: Value) -> Option<Value> {
        if v.get("method").is_none() {
            return Some(v);
        }
        let p = &v["params"];
        match v["method"].as_str() {
            Some("vornd:records") => {
                let id = p["id"].as_str().unwrap().to_owned();
                for r in p["records"].as_array().unwrap() {
                    let key = (
                        id.clone(),
                        r["epoch"].as_u64().unwrap() as u32,
                        r["rseq"].as_u64().unwrap(),
                    );
                    if !self.seen.insert(key) {
                        continue;
                    }
                    if let Some(d) = r["data"].as_str() {
                        let bytes = data_encoding::BASE64.decode(d.as_bytes()).unwrap();
                        self.output
                            .entry(id.clone())
                            .or_default()
                            .push_str(&String::from_utf8_lossy(&bytes));
                    }
                }
            }
            Some("vornd:effect") => {
                let key = p["effect"].as_str().unwrap().to_owned();
                let e = self.effects.entry(key).or_insert((p.clone(), 0));
                e.1 += 1;
            }
            other => panic!("unexpected {other:?}"),
        }
        self.notes.push(v);
        None
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.tx.send(Message::text(req.to_string())).unwrap();
        let t = Instant::now();
        loop {
            let left = PATIENCE.saturating_sub(t.elapsed());
            let v = tokio::time::timeout(left, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("{method} was never answered"))
                .expect("the link closed");
            if let Some(reply) = self.heard(v) {
                if reply["id"] == id {
                    return match reply.get("error") {
                        Some(e) => Err(e["message"].as_str().unwrap_or("").to_owned()),
                        None => Ok(reply["result"].clone()),
                    };
                }
            }
        }
    }

    /// Lists and follows, as the server does once linked.
    async fn take_on(&mut self) -> Value {
        let listed = self.call("vornd:list", json!({})).await.unwrap();
        self.call("vornd:follow", json!({})).await.unwrap();
        listed
    }

    /// Spawns, retrying while vornd has no sessiond yet.
    async fn spawn(&mut self, params: Value) -> Value {
        let t = Instant::now();
        loop {
            match self.call("vornd:spawn", params.clone()).await {
                Ok(v) => return v,
                Err(e) if t.elapsed() < PATIENCE && e.contains("no session holder") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => panic!("spawn: {e}"),
            }
        }
    }

    async fn until(&mut self, what: &str, ok: impl Fn(&Node) -> bool) {
        let t = Instant::now();
        while !ok(self) {
            let left = PATIENCE.saturating_sub(t.elapsed());
            match tokio::time::timeout(left, self.rx.recv()).await {
                Ok(Some(v)) => {
                    self.heard(v);
                }
                Ok(None) => panic!("the link closed waiting for {what}"),
                Err(_) => panic!("timed out waiting for {what}"),
            }
        }
    }

    /// Whatever else arrives within `dur`.
    async fn settle(&mut self, dur: Duration) {
        let end = Instant::now() + dur;
        while let Ok(Some(v)) = tokio::time::timeout_at(end.into(), self.rx.recv()).await {
            self.heard(v);
        }
    }

    fn effects_of(&self, id: &str, kind: &str) -> Vec<(&Value, u32)> {
        let mut v: Vec<_> = self
            .effects
            .values()
            .filter(|(p, _)| p["id"] == id && p["kind"] == kind)
            .map(|(p, n)| (p, *n))
            .collect();
        v.sort_by_key(|(p, _)| (p["rseq"].as_u64(), p["index"].as_u64()));
        v
    }

    fn exit_of(&self, id: &str) -> Option<Value> {
        self.notes.iter().find_map(|n| {
            (n["method"] == "vornd:records" && n["params"]["id"] == id)
                .then(|| {
                    n["params"]["records"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find_map(|r| r.get("exit").cloned())
                })
                .flatten()
        })
    }
}

fn env() -> Value {
    json!({ "PATH": std::env::var("PATH").unwrap_or_default(), "TERM": "xterm-256color" })
}

fn tmp() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_server_starts_feeds_and_signals_its_terminals_through_vornd() {
    let rig = Rig::start().await;
    let v = rig.vornd();
    let mut node = rig.node().await;
    node.take_on().await;

    let spawned = node
        .spawn(json!({
            "id": "term-1",
            "argv": ["sh", "-c", "read x; echo got-$x; sleep 30"],
            "cwd": tmp(),
            "env": env(),
            "cols": 100,
            "rows": 30,
        }))
        .await;
    assert_eq!(spawned["id"], "term-1");
    assert!(spawned["pid"].as_u64().unwrap() > 0);
    assert_ne!(spawned["epoch"], 0);

    node.call("vornd:write", json!({ "id": "term-1", "data": "hi\r" }))
        .await
        .unwrap();
    node.until("the echo", |n| {
        n.output.get("term-1").is_some_and(|o| o.contains("got-hi"))
    })
    .await;

    // A resize is recorded and reaches the server in order with the output.
    node.call(
        "vornd:resize",
        json!({ "id": "term-1", "cols": 120, "rows": 40 }),
    )
    .await
    .unwrap();
    node.until("the resize record", |n| {
        n.notes.iter().any(|x| {
            x["params"]["records"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["resize"] == json!([120, 40])))
        })
    })
    .await;

    let listed = node.call("vornd:list", json!({})).await.unwrap();
    let s = &listed["sessions"][0];
    assert_eq!(s["id"], "term-1");
    assert_eq!(s["kind"], "pty");
    assert_eq!(s["pid"], spawned["pid"]);

    // Kill goes to sessiond as a signal; the exit comes back as a record and
    // as an effect.
    node.call("vornd:signal", json!({ "id": "term-1", "signal": "kill" }))
        .await
        .unwrap();
    node.until("the exit", |n| n.exit_of("term-1").is_some())
        .await;
    assert_eq!(node.exit_of("term-1").unwrap()["signal"], 9);
    node.until("the exit effect", |n| {
        !n.effects_of("term-1", "exit").is_empty()
    })
    .await;
    let gone = Instant::now();
    while rig.d.holds("term-1") {
        assert!(gone.elapsed() < PATIENCE, "never released");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let listed = node.call("vornd:list", json!({})).await.unwrap();
    assert_eq!(listed["sessions"], json!([]));
    assert_eq!(listed["ended"][0]["id"], "term-1");

    // A name used again is a new session in an epoch of its own.
    let again = node
        .spawn(json!({ "id": "term-1", "argv": ["sh", "-c", "echo second"], "cwd": tmp(), "env": env() }))
        .await;
    assert_ne!(again["epoch"], spawned["epoch"]);
    v.kill().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_piped_agent_gets_its_prompt_on_stdin_and_its_exit_code_back() {
    let rig = Rig::start().await;
    let v = rig.vornd();
    let mut node = rig.node().await;
    node.take_on().await;
    node.spawn(json!({
        "id": "agent-1",
        "argv": ["sh", "-c", "printf 'prompt='; cat; printf ' done' >&2; exit 4"],
        "cwd": tmp(),
        "env": env(),
        "piped": true,
        "stdin": "fix the tests",
    }))
    .await;
    node.until("the exit", |n| n.exit_of("agent-1").is_some())
        .await;
    assert_eq!(node.exit_of("agent-1").unwrap()["code"], 4);
    assert_eq!(node.output["agent-1"], "prompt=fix the tests done");
    let listed = node.call("vornd:list", json!({})).await.unwrap();
    assert_eq!(listed["ended"][0]["exited"]["code"], 4);
    // A name in use is refused, not spawned beside it.
    let held = node
        .spawn(json!({ "id": "agent-2", "argv": ["sleep", "30"], "cwd": tmp(), "env": env(), "piped": true }))
        .await;
    assert_eq!(held["id"], "agent-2");
    let refused = node
        .call(
            "vornd:spawn",
            json!({ "id": "agent-2", "argv": ["true"], "cwd": tmp(), "env": env() }),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("still held"), "{refused}");
    v.kill().await;
}

/// TP-T27 and RC-T22 (a), (b) and (d) at the link: vornd dies after a
/// notification, a bell and a clipboard write reached the server and before
/// any checkpoint covered them. The next vornd sends the notification again
/// under the same effect id, so the server shows it once by that id; the
/// bell and the clipboard write are not sent again. Status converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_a_crash_only_what_may_repeat_is_sent_again() {
    let rig = Rig::start().await;
    let first = rig.vornd();
    let mut node = rig.node().await;
    node.take_on().await;
    node.spawn(json!({
        "id": "fx-1",
        "argv": ["sh", "-c", concat!(
            "printf '\\033]9;build done\\007'; ",
            "printf 'ding\\007'; ",
            "printf '\\033]52;c;c2VjcmV0\\007'; ",
            "printf 'Continue? (y/n) '; ",
            "sleep 30",
        )],
        "cwd": tmp(),
        "env": env(),
    }))
    .await;
    node.until("the three effects", |n| {
        !n.effects_of("fx-1", "notify").is_empty()
            && !n.effects_of("fx-1", "bell").is_empty()
            && !n.effects_of("fx-1", "clipboard").is_empty()
    })
    .await;
    node.until("waiting", |n| {
        n.effects_of("fx-1", "status")
            .last()
            .is_some_and(|(p, _)| p["status"] == 2)
    })
    .await;
    let notify = node.effects_of("fx-1", "notify")[0].0["effect"].clone();
    assert_eq!(
        node.effects_of("fx-1", "notify")[0].0["body"],
        "build done"
    );
    assert!(!serde_json::to_string(&node.notes)
        .unwrap()
        .contains("c2VjcmV0"));
    first.kill().await;

    let second = rig.vornd();
    let mut again = rig.node().await;
    again.effects = std::mem::take(&mut node.effects);
    let listed = again.take_on().await;
    assert_eq!(listed["sessions"][0]["id"], "fx-1");
    again
        .until("the notification again", |n| {
            n.effects_of("fx-1", "notify").iter().any(|(_, k)| *k == 2)
        })
        .await;
    again.settle(Duration::from_millis(500)).await;
    let notified = again.effects_of("fx-1", "notify");
    assert_eq!(notified.len(), 1, "one notification, by id: {notified:?}");
    assert_eq!(notified[0].0["effect"], notify);
    assert_eq!(notified[0].1, 2, "at least once: delivered by both vornds");
    for kind in ["bell", "clipboard"] {
        let got = again.effects_of("fx-1", kind);
        assert_eq!(got.len(), 1, "{kind}: {got:?}");
        assert_eq!(got[0].1, 1, "{kind} is at most once");
    }
    let status = again.effects_of("fx-1", "status");
    assert_eq!(status.last().unwrap().0["status"], 2, "status converges");
    second.kill().await;
}
