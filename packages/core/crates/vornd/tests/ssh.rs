//! `ssh:testConnection` end to end: the SSH built-in answers it with the
//! core's test of the host, in the shape clients have always read, and
//! refuses it while switched off.

mod common;

use common::{connect, next, send, serve, Socket, CREDENTIAL};
use serde_json::{json, Value};

/// The frame on `ws` that answers `id`, past any event.
async fn wanted(ws: &mut Socket, id: u64) -> Value {
    loop {
        let frame = next(ws).await;
        if frame["id"] == json!(id) {
            return frame;
        }
    }
}

/// The built-ins are compiled by the `vorn` command beside vornd.
fn compiler_beside_vornd() -> bool {
    let vornd = std::path::Path::new(env!("CARGO_BIN_EXE_vornd"));
    let vorn = vornd.with_file_name(format!("vorn{}", std::env::consts::EXE_SUFFIX));
    if !vorn.exists() {
        eprintln!("skipped: no vorn command beside {}", vornd.display());
    }
    vorn.exists()
}

fn test_connection(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "ssh:testConnection",
            "params": { "id": "h", "label": "Nowhere", "hostname": "nowhere.invalid",
                        "user": "nobody", "port": 22 } })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_is_the_built_ins_and_refused_when_off() {
    if !compiler_beside_vornd() {
        return;
    }
    let served = serve();
    let mut client = connect(served.port, CREDENTIAL).await;

    send(&mut client, test_connection(2)).await;
    let tested = wanted(&mut client, 2).await;
    let result = &tested["result"];
    assert_eq!(result["success"], false, "{tested}");
    assert!(
        result["message"].as_str().is_some_and(|m| !m.is_empty()),
        "{tested}"
    );
    assert!(result["durationMs"].as_u64().is_some(), "{tested}");

    send(
        &mut client,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "extension:setBuiltin",
                "params": { "id": "vorn.ssh", "on": false } }),
    )
    .await;
    let switched = wanted(&mut client, 3).await;
    assert!(switched.get("error").is_none(), "{switched}");
    send(&mut client, test_connection(4)).await;
    assert_eq!(
        wanted(&mut client, 4).await["error"]["message"],
        "SSH is off. Turn it on in Settings \u{2192} Extensions."
    );
}
