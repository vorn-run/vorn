//! The desktop bridge end to end: main claims its connection to vornd with
//! `bridge:identify`, and an agent's `browser:*` and `device:*` calls reach
//! main through vornd and come back.

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
async fn an_agents_browser_call_reaches_main_and_comes_back() {
    let served = serve();
    let mut main = connect(served.port, CREDENTIAL).await;
    assert_eq!(
        identify(&mut main, 1).await,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } })
    );

    let mut agent = connect(served.port, CREDENTIAL).await;
    send(
        &mut agent,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "browser:tabs", "params": { "sessionId": "s" } }),
    )
    .await;
    let asked = next(&mut main).await;
    assert_eq!(asked["method"], "browser:tabs");
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
        json!({ "jsonrpc": "2.0", "id": 1, "method": "browser:tabs" }),
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
