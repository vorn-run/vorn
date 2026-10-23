//! The Wasm tier end to end, through the router as vornd runs it: a
//! component answers, publishes, keeps files and tables and starts sessions
//! on the same bridge as a program; the same gate refuses what its manifest
//! does not declare; it reaches no file, variable or socket of the host; and
//! a trap, a runaway loop or a memory grab costs a restart inside the crash
//! budget while the host carries on. The component is `tests/probe`, built
//! for wasm32-wasip2 by these tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use vorn_extensions::api::types::{CallCtx, Request, Response, SessionStart};
use vorn_extensions::api::ExtError;
use vorn_extensions::component::{Bundled, Compiler, COMPILED};
use vorn_extensions::gate::{Answer, Caller, StorageCall, World};
use vorn_extensions::install::{sha256_hex, Installs, Trust};
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Hook, Router, RouterSettings};
use vorn_extensions::shipped::Shipped;

/// The probe component, built once per test run.
fn probe() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| vorn_guest_build::component("vorn-ext-probe").expect("the probe builds"))
}

/// Answers core methods, keeps a table in memory and counts what reaches it.
#[derive(Default)]
struct Core {
    calls: Mutex<Vec<(String, String)>>,
    /// Whether each caller owned its own namespace, as its calls came.
    owned: Mutex<Vec<bool>>,
    started: Mutex<Vec<(String, SessionStart)>>,
}

impl World for Core {
    fn call(&self, caller: &Caller, method: &str, _: Value) -> Answer<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((caller.extension.clone(), method.to_owned()));
        self.owned
            .lock()
            .unwrap()
            .push(caller.owns(&format!("{}:w", caller.extension)));
        let answer = match method {
            "sessions.list" => Ok(json!([{ "id": "s1" }])),
            m => Err(ExtError::new("unknown-method", m)),
        };
        Box::pin(async move { answer })
    }
    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "none")) })
    }
    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn storage(&self, caller: &Caller, _: Option<PathBuf>, call: StorageCall) -> Answer<Value> {
        let id = caller.extension.clone();
        Box::pin(async move {
            match call {
                StorageCall::Prefix => Ok(json!(format!("x_{id}_"))),
                StorageCall::Transaction(statements) => Ok(Value::Array(
                    statements.iter().map(|(sql, _)| json!(sql.len())).collect(),
                )),
                other => Err(ExtError::new("unsupported", format!("{other:?}"))),
            }
        })
    }
    fn start_session(&self, caller: &Caller, req: SessionStart) -> Answer<String> {
        let mut started = self.started.lock().unwrap();
        started.push((caller.extension.clone(), req));
        let id = format!("s{}", started.len());
        Box::pin(async move { Ok(id) })
    }
}

struct Rig {
    router: Arc<Router>,
    core: Arc<Core>,
    root: tempfile::TempDir,
    _folder: tempfile::TempDir,
}

const ID: &str = "test.probe";

fn manifest(uses: &str) -> String {
    format!(
        "id = \"{ID}\"\nname = \"Probe\"\nversion = \"1.2.3\"\napi = \"vorn:extension@1.0\"\n\n\
         [declares]\nuses = [{uses}]\n\n\
         [service]\ncomponent = \"probe.wasm\"\nevents = [\"session.status\"]\n"
    )
}

/// The probe as an installed package declaring `uses`, linked and routed.
fn rig(uses: &str, policy: Policy, call_timeout: Duration) -> Rig {
    rig_of(&fs::read(probe()).unwrap(), uses, policy, call_timeout)
}

/// [`rig`] for the component `wasm`.
fn rig_of(wasm: &[u8], uses: &str, policy: Policy, call_timeout: Duration) -> Rig {
    rig_fueled(
        wasm,
        uses,
        policy,
        call_timeout,
        vorn_extensions::router::CALL_FUEL,
    )
}

/// [`rig_of`] with each call's fuel `fuel`.
fn rig_fueled(wasm: &[u8], uses: &str, policy: Policy, call_timeout: Duration, fuel: u64) -> Rig {
    let (folder, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fs::write(folder.path().join("vorn-extension.toml"), manifest(uses)).unwrap();
    fs::write(folder.path().join("probe.wasm"), wasm).unwrap();
    // Compiled code a package ships is never run: only what Vorn compiled.
    fs::write(folder.path().join("probe.cwasm"), b"not Vorn's").unwrap();
    let store = PackStore::new(root.path().join("extensions"));
    Installs::new(store.clone(), "git", Vec::new())
        .link(folder.path())
        .unwrap();
    let core = Arc::new(Core::default());
    let mut settings = RouterSettings::new(Vec::new(), "9.9.9");
    settings.call_timeout = call_timeout;
    settings.call_fuel = fuel;
    settings.compiler = Compiler::InProcess;
    settings.blobs = Some(root.path().join("files"));
    let router = Router::new(
        store,
        Lifecycle::new(policy),
        Arc::clone(&core) as Arc<dyn World>,
        settings,
        vorn_extensions::events::Bus::new(),
    );
    router.reindex();
    Rig {
        router,
        core,
        root,
        _folder: folder,
    }
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "test".into(),
        project: None,
    }
}

impl Rig {
    async fn call(&self, action: &str, args: Value) -> Result<Value, ExtError> {
        self.router.call(ID, action, args, ctx()).await
    }
}

const ALL: &str = "\"sessions.read\", \"sessions.launch\", \"process.run\", \"storage\"";

/// The probe as if built against API `version`: its import names rewritten
/// in place, the same length, so the component stays valid.
fn probe_built_for(version: &str) -> Vec<u8> {
    let mut wasm = fs::read(probe()).unwrap();
    let (from, to) = ("@1.0.0".as_bytes(), version.as_bytes());
    assert_eq!(from.len(), to.len());
    let prefix = b"vorn:extension/";
    let mut i = 0;
    while let Some(at) = wasm[i..].windows(prefix.len()).position(|w| w == prefix) {
        let start = i + at + prefix.len();
        let end = (start..wasm.len())
            .find(|&j| !(wasm[j].is_ascii_alphanumeric() || wasm[j] == b'-'))
            .unwrap();
        if wasm[end..].starts_with(from) {
            wasm[end..end + from.len()].copy_from_slice(to);
        }
        i = end;
    }
    wasm
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_built_for_a_newer_or_retired_api_is_refused_naming_both() {
    let started = |version: &str| {
        let rig = rig_of(
            &probe_built_for(version),
            ALL,
            Policy::default(),
            Duration::from_secs(10),
        );
        async move { rig.call("whoami", Value::Null).await.map(|_| ()) }
    };
    // Equal.
    started("@1.0.0")
        .await
        .expect("a component built for 1.0 runs on 1.0");
    // Newer.
    let newer = started("@1.1.0").await.unwrap_err();
    assert!(
        newer
            .message
            .contains("built for vorn:extension@1.1; this Vorn speaks vorn:extension@1.0"),
        "{}",
        newer.message
    );
    // Older: a major this Vorn no longer speaks.
    let older = started("@0.9.0").await.unwrap_err();
    assert!(
        older.message.contains("built for vorn:extension@0.9, which this Vorn no longer speaks; it speaks vorn:extension@1.0"),
        "{}",
        older.message
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_answers_publishes_and_reads_back_through_the_bridge() {
    let rig = rig(ALL, Policy::default(), Duration::from_secs(10));
    let me = rig.call("whoami", Value::Null).await.unwrap();
    assert_eq!(
        me,
        json!({ "id": ID, "version": "1.2.3", "vorn": "9.9.9", "caller": "test", "events": 0 })
    );
    let listed = rig
        .call("call", json!({ "method": "sessions.list", "args": {} }))
        .await
        .unwrap();
    assert_eq!(listed, json!([{ "id": "s1" }]));
    rig.call(
        "publish",
        json!({ "collection": "c", "rows": [{ "id": "a" }, { "id": "b", "n": 1 }] }),
    )
    .await
    .unwrap();
    assert_eq!(
        rig.router.rows(ID, "c"),
        [json!({ "id": "a" }), json!({ "id": "b", "n": 1 })]
    );
    let read = rig
        .call("rows", json!({ "collection": "c" }))
        .await
        .unwrap();
    assert_eq!(read, json!([{ "id": "a" }, { "id": "b", "n": 1 }]));
    // Events reach it in a batch, and what it published survives a restart.
    rig.router
        .emit("session.status", json!({ "sessionId": "s1" }));
    rig.router.settled(ID).await;
    assert_eq!(rig.router.rows(ID, "events")[0]["id"], "session.status");
    rig.router.stop(ID).await;
    let read = rig
        .call("rows", json!({ "collection": "c" }))
        .await
        .unwrap();
    assert_eq!(read.as_array().unwrap().len(), 2);
    // Only Vorn's compiled copy runs, never the package's own.
    let cache = rig.root.path().join("extensions").join(COMPILED);
    let compiled = vorn_extensions::wasm::compiled(&cache, &fs::read(probe()).unwrap());
    assert!(compiled.is_file(), "compiled into Vorn's cache");
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn starts_sessions_keeps_files_and_runs_transactions() {
    let rig = rig(ALL, Policy::default(), Duration::from_secs(10));
    let id = rig
        .call(
            "start",
            json!({ "project": "/p", "agent": "claude", "prompt": "hi" }),
        )
        .await
        .unwrap();
    assert_eq!(id, json!("s1"));
    let started = rig.core.started.lock().unwrap().clone();
    assert_eq!(started[0].0, ID);
    assert_eq!(
        (
            started[0].1.project.as_str(),
            started[0].1.agent.as_deref(),
            started[0].1.prompt.as_deref()
        ),
        ("/p", Some("claude"), Some("hi"))
    );

    rig.call("put", json!({ "name": "notes/a.txt", "text": "héllo" }))
        .await
        .unwrap();
    rig.call("put", json!({ "name": "b.bin", "text": "x" }))
        .await
        .unwrap();
    assert_eq!(
        rig.call("get", json!({ "name": "notes/a.txt" }))
            .await
            .unwrap(),
        json!("héllo")
    );
    assert_eq!(
        rig.call("get", json!({ "name": "nope" })).await.unwrap(),
        Value::Null
    );
    let listed = rig.call("list", json!({ "prefix": "" })).await.unwrap();
    assert_eq!(
        listed,
        json!([{ "name": "b.bin", "size": 1 }, { "name": "notes/a.txt", "size": 6 }])
    );
    assert_eq!(
        rig.call("delete", json!({ "name": "b.bin" }))
            .await
            .unwrap(),
        json!(true)
    );
    let escape = rig
        .call("put", json!({ "name": "../../etc/x", "text": "x" }))
        .await
        .unwrap_err();
    assert_eq!(escape.code, "bad-request", "{}", escape.message);
    // Its files go when it is removed.
    rig.router.forget_files(ID).unwrap();
    assert_eq!(
        rig.call("list", json!({ "prefix": "" })).await.unwrap(),
        json!([])
    );

    let done = rig
        .call(
            "tx",
            json!({ "statements": [{ "sql": "INSERT 1", "params": [1] }, { "sql": "SELECT", "params": [] }] }),
        )
        .await
        .unwrap();
    assert_eq!(done, json!([8, 6]));
    assert_eq!(
        rig.call("prefix", Value::Null).await.unwrap(),
        json!("x_test.probe_")
    );
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gate_refuses_a_component_what_its_manifest_does_not_declare() {
    let rig = rig("", Policy::default(), Duration::from_secs(10));
    let call = rig
        .call("call", json!({ "method": "sessions.list", "args": {} }))
        .await
        .unwrap_err();
    assert_eq!(call.code, "denied", "{}", call.message);
    assert!(call.message.contains("sessions.read"), "{}", call.message);
    let start = rig
        .call("start", json!({ "project": "/p" }))
        .await
        .unwrap_err();
    assert_eq!(start.code, "denied");
    assert!(
        start.message.contains("sessions.launch"),
        "{}",
        start.message
    );
    for (action, args) in [
        ("put", json!({ "name": "a", "text": "x" })),
        ("tx", json!({ "statements": [] })),
        ("set", json!({ "key": "k", "value": 1 })),
    ] {
        let err = rig.call(action, args).await.unwrap_err();
        assert_eq!(err.code, "denied", "{action}: {}", err.message);
        assert!(err.message.contains("storage"), "{}", err.message);
    }
    assert!(rig.core.calls.lock().unwrap().is_empty());
    assert!(rig.core.started.lock().unwrap().is_empty());
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_reaches_no_file_variable_or_socket_of_the_host() {
    let rig = rig(ALL, Policy::default(), Duration::from_secs(10));
    let seen = rig.call("fs", Value::Null).await.unwrap();
    assert_eq!(
        seen,
        json!({ "root": "refused", "write": "refused", "env": 0, "tcp": "refused" })
    );
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_past_its_fuel_is_stopped_long_before_its_deadline() {
    let wasm = fs::read(probe()).unwrap();
    let rig = rig_fueled(
        &wasm,
        ALL,
        Policy::default(),
        Duration::from_secs(60),
        50_000_000,
    );
    let echo = json!({ "n": 1 });
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);
    let started = std::time::Instant::now();
    let spun = rig.call("spin", Value::Null).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(spun.code, "timeout", "{}", spun.message);
    assert!(spun.message.contains("compute"), "{}", spun.message);
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trap_a_runaway_loop_and_a_memory_grab_are_survived_within_the_crash_budget() {
    let policy = Policy {
        max_crashes: 3,
        crash_window: Duration::from_secs(60),
        ..Policy::default()
    };
    let rig = rig(ALL, policy, Duration::from_millis(500));
    let echo = json!({ "n": 1 });
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);

    let spun = rig.call("spin", Value::Null).await.unwrap_err();
    assert_eq!(spun.code, "timeout", "{}", spun.message);
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);

    // Over the 64 MiB a component may hold.
    let grabbed = rig.call("grow", json!({ "mb": 256 })).await.unwrap_err();
    assert_eq!(grabbed.code, "exited", "{}", grabbed.message);
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);
    // Within it, a grab is fine.
    assert_eq!(
        rig.call("grow", json!({ "mb": 8 })).await.unwrap(),
        json!(8 << 20)
    );

    let trapped = rig.call("crash", Value::Null).await.unwrap_err();
    assert_eq!(trapped.code, "exited", "{}", trapped.message);
    assert!(
        trapped.message.contains("the probe was asked to crash"),
        "its panic is said: {}",
        trapped.message
    );
    assert_eq!(rig.call("echo", echo.clone()).await.unwrap(), echo);

    // A timeout is a stop, not a crash; the third crash spends the budget,
    // so nothing restarts until the window passes.
    rig.call("crash", Value::Null).await.unwrap_err();
    let held = rig.call("echo", echo.clone()).await.unwrap_err();
    assert_eq!(held.code, "start", "{}", held.message);
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundled_component_runs_through_the_same_router_and_draws_a_tree() {
    let rig = rig(ALL, Policy::default(), Duration::from_secs(10));
    let wasm: &'static [u8] = Box::leak(fs::read(probe()).unwrap().into_boxed_slice());
    let text = manifest(ALL).replace(ID, "vorn.probe");
    let bundled = Bundled {
        manifest: Arc::new(vorn_extensions::manifest::file::parse_builtin(&text).unwrap()),
        wasm,
    };
    rig.router.prepare_bundled(&bundled).await.unwrap();
    let me = rig
        .router
        .call_bundled(&bundled, "whoami", Value::Null, ctx())
        .await
        .unwrap();
    assert_eq!(me["id"], "vorn.probe");
    let event = Arc::new(vorn_extensions::api::types::Event {
        name: "session.status".into(),
        data: json!({}),
    });
    rig.router.hand_bundled(&bundled, &[event]).await;
    let render = json!({ "point": "page", "surface": { "cells": true } });
    let drawn = rig
        .router
        .call_bundled(&bundled, vorn_tree::RENDER, render, ctx())
        .await
        .unwrap();
    let page = vorn_tree::read(&drawn).expect("a tree the client draws");
    let text = serde_json::to_string(&page.tree).unwrap();
    assert!(text.contains("page on cells"), "{text}");
    let click = json!({ "point": "page", "event": { "id": "hello", "kind": "click" } });
    let done = rig
        .router
        .call_bundled(&bundled, vorn_tree::EVENT, click, ctx())
        .await
        .unwrap();
    assert_eq!(done, json!({ "toast": "hello clicked" }));
    assert_eq!(
        rig.router.rows("vorn.probe", "clicked"),
        [json!({ "id": "hello" })]
    );
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_compiler_only_what_the_cache_holds_runs() {
    let rig = rig(ALL, Policy::default(), Duration::from_secs(10));
    let (folder, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fs::write(folder.path().join("vorn-extension.toml"), manifest(ALL)).unwrap();
    fs::copy(probe(), folder.path().join("probe.wasm")).unwrap();
    let store = PackStore::new(root.path());
    Installs::new(store.clone(), "git", Vec::new())
        .link(folder.path())
        .unwrap();
    let bare = Router::new(
        store,
        Lifecycle::new(Policy::default()),
        Arc::clone(&rig.core) as Arc<dyn World>,
        RouterSettings::new(Vec::new(), "9.9.9"),
        vorn_extensions::events::Bus::new(),
    );
    let err = bare.call(ID, "echo", json!(1), ctx()).await.unwrap_err();
    assert_eq!(err.code, "start");
    assert!(
        err.message.contains("nothing here compiles it"),
        "{}",
        err.message
    );
    // The vorn command fills the cache; then it runs.
    vorn_extensions::wasm::precompile(
        &folder.path().join("probe.wasm"),
        &root.path().join(COMPILED),
    )
    .unwrap();
    assert_eq!(
        bare.call(ID, "echo", json!(1), ctx()).await.unwrap(),
        json!(1)
    );
    bare.lifecycle().stop_all().await;
    rig.router.lifecycle().stop_all().await;
}

/// The probe packed as Vorn packs what it ships: `vorn.probe`, its manifest naming its hooks.
fn probe_pack() -> Vec<u8> {
    let text =
        manifest(ALL).replace(ID, "vorn.probe") + "start = \"started\"\nstop = \"stopped\"\n";
    let wasm = fs::read(probe()).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in [
        ("vorn-extension.toml", text.as_bytes()),
        ("probe.wasm", wasm.as_slice()),
    ] {
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        tar.append_data(&mut header, format!("vorn.probe-1.2.3/{path}"), bytes)
            .unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &tar.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pack_vorn_pins_owns_its_namespace_and_hears_its_start_and_stop() {
    let bytes = probe_pack();
    let shipped = Shipped {
        id: "vorn.probe",
        version: "1.2.3",
        sha256: &sha256_hex(&bytes),
        default: true,
        bytes: &bytes,
    };
    let trust = Trust {
        pins: vec![shipped.pinned()],
        bundled: Default::default(),
    };
    let root = tempfile::tempdir().unwrap();
    let store = PackStore::new(root.path().join("extensions"));
    let installs = Installs::new(store.clone(), "git", Vec::new()).trusting(trust.clone());
    vorn_extensions::shipped::install(&installs, &shipped).unwrap();
    let core = Arc::new(Core::default());
    let router_trusting = |trust: Trust| {
        let mut settings = RouterSettings::new(Vec::new(), "9.9.9");
        settings.compiler = Compiler::InProcess;
        settings.trust = trust;
        Router::new(
            store.clone(),
            Lifecycle::new(Policy::default()),
            Arc::clone(&core) as Arc<dyn World>,
            settings,
            vorn_extensions::events::Bus::new(),
        )
    };
    let list = json!({ "method": "sessions.list", "args": {} });

    let router = router_trusting(trust.clone());
    router.start_installed().await;
    assert_eq!(
        router.rows("vorn.probe", "hooks"),
        [json!({ "id": "started" })]
    );
    router
        .call("vorn.probe", "call", list.clone(), ctx())
        .await
        .unwrap();
    router.call_hook("vorn.probe", Hook::Stop).await;
    assert_eq!(
        router.rows("vorn.probe", "hooks"),
        [json!({ "id": "started" }), json!({ "id": "stopped" })]
    );
    router.lifecycle().stop_all().await;

    // The same files, not vouched for by this build, own nothing of Vorn's.
    let unpinned = router_trusting(Trust::default());
    unpinned
        .call("vorn.probe", "call", list.clone(), ctx())
        .await
        .unwrap();
    assert_eq!(*core.owned.lock().unwrap(), [true, false]);
    unpinned.lifecycle().stop_all().await;

    // Nor do they once a file is added beside the pinned pack's.
    let installed = store.describe("vorn.probe").unwrap();
    fs::write(installed.path.join("extra.txt"), "").unwrap();
    let changed = router_trusting(trust);
    changed
        .call("vorn.probe", "call", list, ctx())
        .await
        .unwrap();
    assert_eq!(*core.owned.lock().unwrap(), [true, false, false]);
    changed.lifecycle().stop_all().await;
}
