//! The desktop bridge end to end: the app claims its connection to vornd with
//! `bridge:identify`, one connection at a time, and an agent's `browser:*` and
//! `device:*` calls go to the installed Browser and Device (from the catalog) extensions, and
//! are refused once those are uninstalled. (The extensions driving the app's web views and
//! simulators with `web:*` and `video:*` are covered end to end by
//! vorn-app's `panes` tests; the bridge's requests by its unit tests.)

mod common;

use common::{connect, next, send, serve, Socket, CREDENTIAL};
use serde_json::{json, Value};

async fn identify(ws: &mut Socket, id: u64) -> Value {
    send(
        ws,
        json!({ "jsonrpc": "2.0", "id": id, "method": "bridge:identify" }),
    )
    .await;
    next(ws).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_main_at_a_time_and_its_close_frees_the_bridge() {
    let served = serve();
    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(
        identify(&mut main, 1).await,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } })
    );
    let mut thief = connect(served.port, CREDENTIAL).await;
    assert_eq!(identify(&mut thief, 2).await["result"]["ok"], false);
    main.close(None).await.unwrap();
    // The close is seen in its own time; the next claim then holds.
    let t = std::time::Instant::now();
    let mut id = 3;
    while identify(&mut thief, id).await["result"]["ok"] != true {
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "main never let go"
        );
        id += 1;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The frame on `ws` that answers `id`, past any event.
async fn wanted(ws: &mut Socket, id: u64) -> Value {
    loop {
        let frame = next(ws).await;
        if frame["id"] == json!(id) {
            return frame;
        }
    }
}

/// Whether this build ships `id`, or has its pack for the catalog, and can compile its component.
fn runs_shipped(id: &str) -> bool {
    if !vornd::shipped::SET.iter().any(|s| s.id == id) && common::catalog_pack(id).is_none() {
        eprintln!(
            "skipped: this build has no pack of {id} (see packages/core/scripts/shipped.mjs)"
        );
        return false;
    }
    let vornd = std::path::Path::new(env!("CARGO_BIN_EXE_vornd"));
    let vorn = vornd.with_file_name(format!("vorn{}", std::env::consts::EXE_SUFFIX));
    if !vorn.exists() {
        eprintln!("skipped: no vorn command beside {}", vornd.display());
    }
    vorn.exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agents_browser_call_is_the_installed_browsers_and_refused_once_uninstalled() {
    if !runs_shipped("vorn.browser") {
        return;
    }
    let served = serve();
    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(identify(&mut main, 1).await["result"]["ok"], true);
    let mut agent = connect(served.port, CREDENTIAL).await;

    // The extension answers, not the app: it knows no session `s`.
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "browser:openPane", "params": { "sessionId": "s" } }),
    )
    .await;
    let opened = wanted(&mut agent, 2).await;
    assert_eq!(
        opened["error"]["message"], "No session s in this app.",
        "{opened}"
    );

    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "extension:remove",
                "params": { "id": "vorn.browser" } }),
    )
    .await;
    let removed = wanted(&mut agent, 4).await;
    assert!(removed.get("error").is_none(), "{removed}");
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "browser:listTabs", "params": { "sessionId": "s" } }),
    )
    .await;
    assert_eq!(
        wanted(&mut agent, 5).await["error"]["message"],
        "Browser is not installed. Install it in Settings \u{2192} Extensions."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agents_device_call_is_the_installed_devices_and_refused_once_uninstalled() {
    if !runs_shipped("vorn.device") {
        return;
    }
    // Device is not shipped: it installs from the catalog.
    let (version, sha256, bytes) = common::catalog_pack("vorn.device").unwrap();
    let home = tempfile::tempdir().unwrap();
    common::offer_in_catalog(home.path(), vec![("vorn.device", version, sha256, bytes)]);
    let served = common::serve_at(tempfile::tempdir().unwrap(), home);
    let mut agent = connect(served.port, CREDENTIAL).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "connector:installPack",
                "params": { "kind": "catalog", "id": "vorn.device" } }),
    )
    .await;
    let installed = wanted(&mut agent, 1).await;
    assert_eq!(installed["result"]["ok"], true, "{installed}");

    // The extension answers, not the app: it knows no session `s`, and the
    // chrome needs none.
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "device:list", "params": { "sessionId": "s" } }),
    )
    .await;
    let listed = wanted(&mut agent, 2).await;
    assert_eq!(
        listed["error"]["message"], "No session s in this app.",
        "{listed}"
    );
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "device:chrome", "params": {} }),
    )
    .await;
    assert_eq!(wanted(&mut agent, 3).await["result"], Value::Null);

    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "extension:remove",
                "params": { "id": "vorn.device" } }),
    )
    .await;
    let removed = wanted(&mut agent, 4).await;
    assert!(removed.get("error").is_none(), "{removed}");
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "device:claim", "params": { "sessionId": "s", "udid": "u" } }),
    )
    .await;
    assert_eq!(
        wanted(&mut agent, 5).await["error"]["message"],
        "Device is not installed. Install it in Settings \u{2192} Extensions."
    );
}
