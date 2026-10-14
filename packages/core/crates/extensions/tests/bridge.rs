//! The bridge against real programs over stdio: every method of the API
//! driven from the mapping the WIT generates, a stalled extension beside a
//! busy one, crashes and timeouts, idle shutdown and the cap. Skipped
//! without `node`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use vorn_extensions::api::types::{CallCtx, Header, Request, Response};
use vorn_extensions::api::{mapping, ExtError, Method, Side};
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

const PROGRAM: &str = include_str!("fixtures/program.mjs");

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

/// The world behind the gate, recording what reached it.
#[derive(Default)]
struct Fake {
    fetched: Mutex<Vec<Request>>,
    published: Mutex<Vec<String>>,
}

impl World for Fake {
    fn call(&self, _: &Caller, method: &str, _: Value) -> Answer<Value> {
        let answer = match method {
            "sessions.list" => Ok(json!([{ "id": "s1" }])),
            other => Err(ExtError::new("unknown-method", format!("no {other}"))),
        };
        Box::pin(async move { answer })
    }
    fn fetch(&self, _: &Caller, request: Request) -> Answer<Response> {
        let body = request.url.clone();
        self.fetched.lock().unwrap().push(request);
        Box::pin(async move {
            Ok(Response {
                status: 200,
                headers: vec![Header {
                    name: "content-type".into(),
                    value: "text/plain".into(),
                }],
                body,
            })
        })
    }
    fn secret(&self, _: &Caller, connection: &str, _: &str) -> Answer<Option<String>> {
        let known = (connection == "c1").then(|| "t0ken".to_owned());
        Box::pin(async move { Ok(known) })
    }
    fn published(&self, caller: &Caller, collection: &str) {
        self.published
            .lock()
            .unwrap()
            .push(format!("{}/{collection}", caller.extension));
    }
}

fn program_manifest(id: &str, extra: &str) -> String {
    format!(
        r#"id = "{id}"
name = "{id}"
version = "1.0.0"
api = "vorn:extension@0.9"

[service]
command = ["node", "program.mjs"]
events = ["test.ping"]

[actions.once]
title = "Once"
command = ["node", "program.mjs"]

[declares]
network = ["api.example.com"]
uses = ["sessions.read"]

[credential]
field = "token"
header = "Authorization: Bearer {{token}}"
{extra}"#
    )
}

struct Bench {
    _packs: tempfile::TempDir,
    _folders: Vec<tempfile::TempDir>,
    router: Arc<Router>,
    lifecycle: Arc<Lifecycle>,
    world: Arc<Fake>,
}

/// `ids` linked as program extensions, under `policy`.
fn bench(ids: &[&str], policy: Policy, call_timeout: Duration) -> Bench {
    let packs = tempfile::tempdir().unwrap();
    let store = PackStore::new(packs.path());
    let installs = Installs::new(store.clone(), "git", Vec::new());
    let mut folders = Vec::new();
    for id in ids {
        let folder = tempfile::tempdir().unwrap();
        fs::write(
            folder.path().join("vorn-extension.toml"),
            program_manifest(id, ""),
        )
        .unwrap();
        fs::write(folder.path().join("program.mjs"), PROGRAM).unwrap();
        installs.link(folder.path()).unwrap();
        folders.push(folder);
    }
    let lifecycle = Lifecycle::new(policy);
    let world = Arc::new(Fake::default());
    let mut settings = RouterSettings::new(
        std::env::vars()
            .filter(|(k, _)| k == "PATH" || k == "SYSTEMROOT")
            .collect(),
        "test",
    );
    settings.call_timeout = call_timeout;
    let router = Router::new(
        store,
        Arc::clone(&lifecycle),
        Arc::clone(&world) as Arc<dyn World>,
        settings,
    );
    Bench {
        _packs: packs,
        _folders: folders,
        router,
        lifecycle,
        world,
    }
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "test".into(),
        project: Some("/p".into()),
    }
}

impl Bench {
    async fn call(&self, id: &str, action: &str, args: Value) -> Result<Value, ExtError> {
        self.router.call(id, action, args, ctx()).await
    }

    /// Asks the program to send `method` with `params` to Vorn, and what came back.
    async fn relay(&self, method: Method, params: Value) -> Value {
        let args = json!({
            "method": method.name(), "params": params,
            "notification": method.is_notification(),
        });
        self.call("a", "relay", args).await.unwrap()
    }
}

/// Every method of the API, driven over stdio with params made from the
/// mapping the WIT generates, its answers checked against the mapping.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_conformance_suite_drives_every_method_over_stdio() {
    if !node() {
        return;
    }
    let b = bench(&["a"], Policy::default(), Duration::from_secs(10));
    let map = mapping();
    let check = |ty: &str, value: &Value| {
        map.check(ty, value)
            .unwrap_or_else(|e| panic!("{ty}: {e} in {value}"))
    };
    let check_error = |answer: &Value| {
        let error = &answer["error"];
        assert_eq!(error["code"], -32000, "{answer}");
        check(
            "ext-error",
            &json!({ "code": error["data"]["code"], "message": error["message"] }),
        );
    };
    for method in Method::ALL {
        let doc = map.method(method).expect("every method is in the mapping");
        let sample = map.sample_params(method);
        for (param, ty) in &doc.params {
            check(ty, &sample[param.as_str()]);
        }
        match (method.side(), method) {
            (Side::Guest, Method::GuestInit) => {
                let echo = b.call("a", "echo", json!({})).await.unwrap();
                check("init-ctx", &echo["inited"]);
                assert_eq!(echo["inited"]["id"], "a");
            }
            (Side::Guest, Method::GuestHandle) => {
                let echo = b.call("a", "echo", sample["args"].clone()).await.unwrap();
                check(doc.result.as_deref().unwrap(), &echo);
                check("call-ctx", &echo["ctx"]);
                assert_eq!(echo["args"], sample["args"]);
            }
            (Side::Guest, Method::GuestOnEvent) => {
                let reached = b.router.emit("test.ping", json!({ "n": 1 })).await;
                assert_eq!(reached, 1);
                let mut events = Value::Null;
                for _ in 0..50 {
                    events = b.call("a", "events", json!({})).await.unwrap();
                    if events.as_array().is_some_and(|e| !e.is_empty()) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                check("list<event>", &events);
                assert_eq!(events[0]["name"], "test.ping");
            }
            (Side::Host, Method::HostCall) => {
                check_error(&b.relay(method, sample).await);
                let listed = b
                    .relay(method, json!({ "method": "sessions.list", "args": {} }))
                    .await;
                check(doc.result.as_deref().unwrap(), &listed["result"]);
                assert_eq!(listed["result"][0]["id"], "s1");
                let refused = b
                    .relay(method, json!({ "method": "git.status", "args": {} }))
                    .await;
                assert_eq!(refused["error"]["data"]["code"], "denied");
                assert_eq!(
                    refused["error"]["message"],
                    "a does not declare git.read, which git.status needs"
                );
            }
            (Side::Host, Method::HostPublish) => {
                assert_eq!(
                    b.relay(method, sample.clone()).await,
                    json!({ "sent": true })
                );
                let row = sample["ops"][0]["upsert"].clone();
                let mut rows = Vec::new();
                for _ in 0..50 {
                    rows = b.router.rows("a", sample["collection"].as_str().unwrap());
                    if !rows.is_empty() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert_eq!(rows, [row]);
                assert_eq!(*b.world.published.lock().unwrap(), ["a/x"]);
            }
            (Side::Host, Method::HostLog) => {
                assert_eq!(b.relay(method, sample).await, json!({ "sent": true }));
            }
            (Side::Host, Method::NetFetch) => {
                // The sample's URL is no https URL, so the gate refuses it.
                check_error(&b.relay(method, sample).await);
                let req = json!({ "req": { "method": "GET", "url": "https://api.example.com/x",
                    "headers": [{ "name": "authorization", "value": "Bearer mine" }], "body": null },
                    "connection": "c1" });
                let fetched = b.relay(method, req).await;
                check(doc.result.as_deref().unwrap(), &fetched["result"]);
                let sent = b.world.fetched.lock().unwrap().pop().unwrap();
                assert_eq!(
                    sent.headers,
                    [Header {
                        name: "Authorization".into(),
                        value: "Bearer t0ken".into()
                    }]
                );
            }
            (side, method) => {
                panic!("{method:?} is answered by {side:?}, which nothing here drives")
            }
        }
    }
    // Params that do not read, a method the API does not have, and one only the extension answers.
    let unread = b.relay(Method::HostCall, json!(null)).await;
    assert_eq!(unread["error"]["code"], -32602, "{unread}");
    for name in ["nope.never", "guest.handle"] {
        let args = json!({ "method": name, "params": {} });
        let answer = b.call("a", "relay", args).await.unwrap();
        assert_eq!(answer["error"]["code"], -32601, "{answer}");
    }
    b.lifecycle.stop_all().await;
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

async fn timed_calls(b: &Bench, id: &str, n: usize) -> Duration {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let started = Instant::now();
        b.call(id, "echo", json!({})).await.unwrap();
        samples.push(started.elapsed());
    }
    median(samples)
}

/// A stalled extension holds nothing another extension's call needs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_extension_adds_under_a_millisecond_to_another_extensions_call() {
    if !node() {
        return;
    }
    let b = Arc::new(bench(
        &["a", "b"],
        Policy::default(),
        Duration::from_secs(60),
    ));
    timed_calls(&b, "a", 20).await;
    timed_calls(&b, "b", 20).await;
    let alone = timed_calls(&b, "b", 200).await;
    let stalled = {
        let b = Arc::clone(&b);
        tokio::spawn(async move { b.call("a", "stall", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let beside = timed_calls(&b, "b", 200).await;
    eprintln!("call to b: {alone:?} alone, {beside:?} beside a stalled a");
    assert!(
        beside.saturating_sub(alone) < Duration::from_millis(1),
        "{alone:?} alone, {beside:?} beside a stalled extension"
    );
    assert!(!stalled.is_finished());
    stalled.abort();
    b.lifecycle.stop_all().await;
}

/// A crash or a timeout costs the call it happened on, and the next call starts the program again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crash_and_a_timeout_cost_one_call() {
    if !node() {
        return;
    }
    let b = bench(&["a"], Policy::default(), Duration::from_millis(500));
    let first = b.call("a", "echo", json!({})).await.unwrap()["pid"].clone();
    let crashed = b.call("a", "crash", json!({})).await.unwrap_err();
    assert_eq!(crashed.code, "exited");
    assert!(
        crashed.message.ends_with("Error: asked to crash"),
        "{crashed}"
    );
    let second = b.call("a", "echo", json!({})).await.unwrap()["pid"].clone();
    assert_ne!(first, second);

    let timed_out = b.call("a", "stall", json!({})).await.unwrap_err();
    assert_eq!(timed_out.code, "timeout");
    let third = b.call("a", "echo", json!({})).await.unwrap()["pid"].clone();
    assert_ne!(second, third, "a program that timed out starts fresh");
    b.lifecycle.stop_all().await;
}

fn alive(pid: u64) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with signal 0 only asks whether the process exists.
        unsafe { libc::kill(i32::try_from(pid).unwrap_or(0), 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..300 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{what} did not happen");
}

/// With 20 Node services installed and used once, none is running after the idle time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_idle_node_services_all_stop() {
    if !node() {
        return;
    }
    let ids: Vec<String> = (0..20).map(|i| format!("svc{i:02}")).collect();
    let names: Vec<&str> = ids.iter().map(String::as_str).collect();
    let policy = Policy {
        idle: Duration::from_millis(800),
        cap: 32,
        ..Policy::default()
    };
    let b = bench(&names, policy, Duration::from_secs(20));
    let mut pids = Vec::new();
    for id in &names {
        let echo = b.call(id, "echo", json!({})).await.unwrap();
        pids.push(echo["pid"].as_u64().unwrap());
    }
    assert_eq!(b.lifecycle.running().len(), 20);
    until("every idle service stopping", || {
        b.lifecycle.running().is_empty()
    })
    .await;
    until("every process ending", || pids.iter().all(|p| !alive(*p))).await;
    // The next call starts one again.
    b.call("svc07", "echo", json!({})).await.unwrap();
    assert_eq!(b.lifecycle.running().len(), 1);
    b.lifecycle.stop_all().await;
}

/// With the cap at N, starting service N+1 stops the least recently used.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cap_stops_the_least_recently_used() {
    if !node() {
        return;
    }
    let policy = Policy {
        cap: 3,
        ..Policy::default()
    };
    let b = bench(&["a", "b", "c", "d"], policy, Duration::from_secs(20));
    let mut pids = Vec::new();
    for id in ["a", "b", "c"] {
        pids.push(
            b.call(id, "echo", json!({})).await.unwrap()["pid"]
                .as_u64()
                .unwrap(),
        );
    }
    // `a` used again: `b` is now the least recently used.
    b.call("a", "echo", json!({})).await.unwrap();
    b.call("d", "echo", json!({})).await.unwrap();
    let running: Vec<String> = b
        .lifecycle
        .running()
        .into_iter()
        .map(|k| k.extension)
        .collect();
    assert_eq!(running, ["a", "c", "d"]);
    until("the evicted process ending", || !alive(pids[1])).await;
    b.lifecycle.stop_all().await;
}

/// A service stopped for idling answers its next call within what a one-shot run of it takes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_service_answers_within_its_one_shot_start_time() {
    if !node() {
        return;
    }
    let policy = Policy {
        idle: Duration::from_millis(100),
        ..Policy::default()
    };
    let b = bench(&["a"], policy, Duration::from_secs(20));
    let mut one_shot = Vec::new();
    let mut restarted = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        b.call("a", "once", json!({})).await.unwrap_err();
        one_shot.push(started.elapsed());
        until("the service idling", || b.lifecycle.running().is_empty()).await;
        let started = Instant::now();
        b.call("a", "echo", json!({})).await.unwrap();
        restarted.push(started.elapsed());
    }
    let (one_shot, restarted) = (median(one_shot), median(restarted));
    eprintln!("one-shot {one_shot:?}, restarted service {restarted:?}");
    assert!(
        restarted <= one_shot + one_shot / 4 + Duration::from_millis(10),
        "a restart took {restarted:?}; a one-shot run takes {one_shot:?}"
    );
    b.lifecycle.stop_all().await;
}

/// Sizes of what tests leave behind, so a removal can be shown to leave nothing.
fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Removing an extension stops its processes and drops its rows and files.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_leaves_no_files_rows_or_processes() {
    if !node() {
        return;
    }
    let b = bench(&["a"], Policy::default(), Duration::from_secs(20));
    let pid = b.call("a", "echo", json!({})).await.unwrap()["pid"]
        .as_u64()
        .unwrap();
    b.call(
        "a",
        "publish",
        json!({ "ops": [{ "upsert": { "id": "r1" } }] }),
    )
    .await
    .unwrap();
    until("the row arriving", || {
        !b.router.rows("a", "rows").is_empty()
    })
    .await;
    b.router.forget("a").await;
    let installs = Installs::new(b.router.store().clone(), "git", Vec::new());
    installs.remove("a").unwrap();
    assert!(b.router.rows("a", "rows").is_empty());
    assert!(b.lifecycle.running().is_empty());
    until("its process ending", || !alive(pid)).await;
    assert!(entries(b.router.store().root()).is_empty());
}
