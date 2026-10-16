//! The Wasm tier end to end, through the router as vornd runs it: a
//! component answers and publishes on the same bridge as a program, the
//! same gate refuses what its manifest does not declare, and a trap, a
//! runaway loop or a memory grab costs a restart inside the crash budget
//! while the host carries on. The component is the bench's
//! (`bench/ext-tiers/component`, built for wasm32-wasip2); skipped when it
//! has not been built.
#![cfg(feature = "wasm-compile")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_extensions::api::types::{CallCtx, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::install::Installs;
use vorn_extensions::lifecycle::{Lifecycle, Policy};
use vorn_extensions::pack::PackStore;
use vorn_extensions::router::{Router, RouterSettings};

/// The bench's component: `VORN_EXT_COMPONENT`, else where cargo puts it.
fn component() -> Option<PathBuf> {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wasm32-wasip2");
    let found = std::env::var_os("VORN_EXT_COMPONENT")
        .map(PathBuf::from)
        .into_iter()
        .chain(["ci", "release"].map(|p| target.join(p).join("ext_tiers_component.wasm")))
        .find(|p| p.is_file());
    if found.is_none() {
        eprintln!("skipped: build bench/ext-tiers/component for wasm32-wasip2 first");
    }
    found
}

/// Answers `worktrees.list` with one worktree, counting what reaches it.
#[derive(Default)]
struct One {
    calls: AtomicUsize,
}

impl World for One {
    fn call(&self, _: &Caller, method: &str, _: Value) -> Answer<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = match method {
            "worktrees.list" => Ok(json!([{
                "path": "/p/api/.worktrees/w1", "projectPath": "/p/api", "projectName": "api",
                "branch": "feature/1", "main": false, "orphan": false, "dirty": false,
                "sessionIds": [], "sizeBytes": 1_000_000, "artifactBytes": 0,
                "verdict": "safe", "reasons": ["merged"],
            }])),
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
}

struct Rig {
    router: Arc<Router>,
    world: Arc<One>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// Worktree cleanup as a component pack declaring `uses`, linked and routed.
fn rig(wasm: &Path, uses: &str, policy: Policy, call_timeout: Duration) -> Rig {
    let (folder, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fs::write(
        folder.path().join("vorn-extension.toml"),
        format!(
            "id = \"vorn.worktrees\"\nname = \"Worktree cleanup\"\nversion = \"0.9.0\"\n\
             api = \"vorn:extension@0.9\"\n\n[declares]\nuses = [{uses}]\n\n\
             [service]\ncomponent = \"ext.wasm\"\nevents = [\"worktree.idle\"]\n"
        ),
    )
    .unwrap();
    fs::copy(wasm, folder.path().join("ext.wasm")).unwrap();
    // Compiled code a package ships is never run: only what Vorn compiled.
    fs::write(folder.path().join("ext.cwasm"), b"not Vorn's").unwrap();
    let env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| ["PATH", "HOME", "TMPDIR"].contains(&k.as_str()))
        .collect();
    let store = PackStore::new(root.path());
    Installs::new(store.clone(), "git", env.clone())
        .link(folder.path())
        .unwrap();
    let world = Arc::new(One::default());
    let mut settings = RouterSettings::new(env, "test");
    settings.call_timeout = call_timeout;
    let router = Router::new(
        store,
        Lifecycle::new(policy),
        Arc::clone(&world) as Arc<dyn World>,
        settings,
        vorn_extensions::events::Bus::new(),
    );
    Rig {
        router,
        world,
        _dirs: (folder, root),
    }
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "test".into(),
        project: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_component_answers_and_publishes_through_the_gate() {
    let Some(wasm) = component() else { return };
    let rig = rig(
        &wasm,
        "\"git.read\", \"git.write\", \"sessions.launch\"",
        Policy::default(),
        Duration::from_secs(10),
    );
    let answer = rig
        .router
        .call("vorn.worktrees", "scan", json!({}), ctx())
        .await
        .unwrap();
    assert_eq!(answer, json!({ "worktrees": 1 }));
    let rows = rig.router.rows("vorn.worktrees", "worktrees");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "/p/api/.worktrees/w1");
    assert_eq!(rig.world.calls.load(Ordering::SeqCst), 1);
    let cache = rig
        .router
        .store()
        .root()
        .join(vorn_extensions::wasm::COMPILED);
    let compiled = vorn_extensions::wasm::compiled(&cache, &fs::read(&wasm).unwrap());
    assert!(compiled.is_file(), "compiled into Vorn's cache");
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gate_refuses_a_component_what_its_manifest_does_not_declare() {
    let Some(wasm) = component() else { return };
    let rig = rig(&wasm, "", Policy::default(), Duration::from_secs(10));
    let err = rig
        .router
        .call("vorn.worktrees", "scan", json!({}), ctx())
        .await
        .unwrap_err();
    assert_eq!(err.code, "denied", "{}", err.message);
    assert!(err.message.contains("git.read"), "{}", err.message);
    assert_eq!(rig.world.calls.load(Ordering::SeqCst), 0);
    rig.router.lifecycle().stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trap_a_runaway_loop_and_a_memory_grab_are_survived_within_the_crash_budget() {
    let Some(wasm) = component() else { return };
    let policy = Policy {
        max_crashes: 3,
        crash_window: Duration::from_secs(60),
        ..Policy::default()
    };
    let rig = rig(&wasm, "", policy, Duration::from_millis(500));
    let call = |action: &'static str, args: Value| {
        let router = Arc::clone(&rig.router);
        async move { router.call("vorn.worktrees", action, args, ctx()).await }
    };
    let echo = json!({ "n": 1 });
    assert_eq!(call("bench.echo", echo.clone()).await.unwrap(), echo);

    let spun = call("bench.spin", json!({})).await.unwrap_err();
    assert_eq!(spun.code, "timeout", "{}", spun.message);
    assert_eq!(call("bench.echo", echo.clone()).await.unwrap(), echo);

    // Over the 64 MiB a component may hold.
    let grabbed = call("bench.grow", json!({ "mb": 256 })).await.unwrap_err();
    assert_eq!(grabbed.code, "exited", "{}", grabbed.message);
    assert_eq!(call("bench.echo", echo.clone()).await.unwrap(), echo);

    let trapped = call("bench.crash", json!({})).await.unwrap_err();
    assert_eq!(trapped.code, "exited", "{}", trapped.message);
    assert!(trapped.message.contains("wasm trap"), "{}", trapped.message);
    assert_eq!(call("bench.echo", echo.clone()).await.unwrap(), echo);

    // A timeout is a stop, not a crash; the third crash spends the budget,
    // so nothing restarts until the window passes.
    call("bench.crash", json!({})).await.unwrap_err();
    let held = call("bench.echo", echo.clone()).await.unwrap_err();
    assert_eq!(held.code, "start", "{}", held.message);
    rig.router.lifecycle().stop_all().await;
}
