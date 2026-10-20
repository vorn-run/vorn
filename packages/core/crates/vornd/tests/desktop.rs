//! The desktop bridge end to end: the app claims its connection to vornd with
//! `bridge:identify`, an agent's `device:*` calls reach the app through vornd
//! and come back, and its `browser:*` calls go to the Browser built-in, which
//! refuses them while switched off. (The built-in driving the app's web view
//! with `web:*` is covered end to end by vorn-app's `panes` tests.)

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
async fn an_agents_device_call_reaches_main_and_comes_back() {
    let served = serve();
    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(
        identify(&mut main, 1).await,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } })
    );

    let mut agent = connect(served.port, CREDENTIAL).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "device:list", "params": { "sessionId": "s" } }),
    )
    .await;
    let asked = next(&mut main).await;
    assert_eq!(asked["method"], "device:list");
    assert_eq!(asked["params"], json!({ "sessionId": "s" }));
    let id = asked["id"].clone();
    assert!(id.as_str().is_some_and(|i| i.starts_with("vornd-")), "{id}");
    send(
        &mut main,
        json!({ "jsonrpc": "2.0", "id": id, "result": [{ "id": "t1" }] }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 5, "result": [{ "id": "t1" }] })
    );

    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "device:claim", "params": { "udid": "u" } }),
    )
    .await;
    let asked = next(&mut main).await;
    send(
        &mut main,
        json!({ "jsonrpc": "2.0", "id": asked["id"], "error": { "code": -32000, "message": "no such device" } }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 6, "error": { "code": -32000, "message": "no such device" } })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_main_at_a_time_and_its_close_fails_what_waits() {
    let served = serve();
    let mut agent = connect(served.port, CREDENTIAL).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "device:list" }),
    )
    .await;
    assert_eq!(
        next(&mut agent).await["error"]["message"],
        "Vorn app is not running (no main process connected)"
    );

    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(identify(&mut main, 1).await["result"]["ok"], true);
    let mut thief = connect(served.port, CREDENTIAL).await;
    assert_eq!(identify(&mut thief, 2).await["result"]["ok"], false);

    // A guessed answer from another socket settles nothing.
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "device:list" }),
    )
    .await;
    let asked = next(&mut main).await;
    send(
        &mut thief,
        json!({ "jsonrpc": "2.0", "id": asked["id"], "result": "forged" }),
    )
    .await;
    main.close(None).await.unwrap();
    assert_eq!(
        next(&mut agent).await,
        json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": -32000, "message": "Vorn main process disconnected" } })
    );
    assert_eq!(identify(&mut thief, 3).await["result"]["ok"], true);
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

fn compiler_beside_vornd() -> bool {
    let vornd = std::path::Path::new(env!("CARGO_BIN_EXE_vornd"));
    let vorn = vornd.with_file_name(format!("vorn{}", std::env::consts::EXE_SUFFIX));
    if !vorn.exists() {
        eprintln!("skipped: no vorn command beside {}", vornd.display());
    }
    vorn.exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agents_browser_call_is_the_built_ins_and_refused_when_off() {
    if !compiler_beside_vornd() {
        return;
    }
    let served = serve();
    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(identify(&mut main, 1).await["result"]["ok"], true);
    let mut agent = connect(served.port, CREDENTIAL).await;

    // The built-in answers, not the app: it knows no session `s`.
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
        json!({ "jsonrpc": "2.0", "id": 4, "method": "extension:setBuiltin",
                "params": { "id": "vorn.browser", "on": false } }),
    )
    .await;
    let switched = wanted(&mut agent, 4).await;
    assert!(switched.get("error").is_none(), "{switched}");
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "browser:listTabs", "params": { "sessionId": "s" } }),
    )
    .await;
    assert_eq!(
        wanted(&mut agent, 5).await["error"]["message"],
        "Browser is off. Turn it on in Settings \u{2192} Extensions."
    );
}
