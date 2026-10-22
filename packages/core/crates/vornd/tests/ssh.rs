//! `ssh:testConnection` end to end: the installed SSH extension answers it
//! with the core's test of the host, in the shape clients have always read,
//! and it is refused once SSH is uninstalled.

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

/// Whether this build ships SSH, whose component the `vorn` command beside vornd compiles.
fn runs_ssh() -> bool {
    if !vornd::shipped::SET.iter().any(|s| s.id == "vorn.ssh") {
        eprintln!("skipped: this build does not ship vorn.ssh (see packages/core/shipped.toml)");
        return false;
    }
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
async fn a_test_is_the_installed_extensions_and_refused_once_uninstalled() {
    if !runs_ssh() {
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
        json!({ "jsonrpc": "2.0", "id": 3, "method": "extension:remove",
                "params": { "id": "vorn.ssh" } }),
    )
    .await;
    let removed = wanted(&mut client, 3).await;
    assert!(removed.get("error").is_none(), "{removed}");
    send(&mut client, test_connection(4)).await;
    assert_eq!(
        wanted(&mut client, 4).await["error"]["message"],
        "SSH is not installed. Install it in Settings \u{2192} Extensions."
    );
}
