//! A connector child in the connector protocol, through the fixture the app's
//! tests use (`tests/fixtures/native-connector.mjs`). Skipped without `node`.

use std::path::PathBuf;

use serde_json::{json, Map};
use vorn_connectors::child::Launch;
use vorn_connectors::sdk::{self, OpenError, SdkLaunch, Source, Timeouts};

fn node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|d| [d.join("node"), d.join("node.exe")])
        .find(|p| p.is_file())
}

fn fixture(mode: &str) -> Option<SdkLaunch> {
    let Some(node) = node() else {
        eprintln!("skipped: no node on PATH");
        return None;
    };
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../tests/fixtures/native-connector.mjs");
    Some(SdkLaunch {
        launch: Launch {
            program: node,
            args: vec![script.to_string_lossy().into_owned(), mode.to_owned()],
            cwd: std::env::temp_dir(),
            env: std::env::vars().collect(),
        },
        source: Source::Command,
        protocol: None,
    })
}

#[tokio::test]
async fn greets_a_child_and_asks_it_everything() {
    let Some(launch) = fixture("normal") else { return };
    let client = sdk::open(&launch, "Fixture", "1.0", Timeouts::default())
        .await
        .unwrap();
    assert_eq!(client.hello()["protocol"], 1);
    assert_eq!(
        client.preflight().await.unwrap(),
        json!({ "ok": true, "message": "ready" })
    );
    let page = client.poll("items", Some("c1"), None).await.unwrap();
    assert_eq!(page["hasMore"], false);
    assert_eq!(page["items"][0]["cursor"], "c1");
    let mut args = Map::new();
    args.insert("title".into(), json!("T"));
    let echo = client.action("create", &args, Some("call-1")).await.unwrap();
    assert_eq!(echo["echo"]["title"], "T");
    assert_eq!(echo["sessionCall"], "call-1");
    args.insert("fail".into(), json!(true));
    let failed = client.action("create", &args, None).await.unwrap_err();
    assert_eq!(failed.message, "the upstream said no");
    assert_eq!(failed.data.kind.as_deref(), Some("upstream"));
    assert_eq!(failed.data.field.as_deref(), Some("title"));
    // The fixture's manifest declares nothing, which an install refuses.
    let refused = client.manifest().await.unwrap_err();
    assert!(refused.message.contains("reports no triggers and no actions"));
    let malformed = client.poll("malformed", None, None).await.unwrap_err();
    assert_eq!(malformed.message, "Fixture answered trigger/poll without a page of items");
    client.close().await;
    assert!(client.exited());
}

#[tokio::test]
async fn tells_an_old_or_newer_child_from_a_broken_one() {
    let Some(launch) = fixture("mcp-only") else { return };
    let err = sdk::open(&launch, "Old", "1.0", Timeouts::default()).await.unwrap_err();
    assert!(matches!(err, OpenError::Outdated(_)), "{err}");

    let Some(launch) = fixture("hello-unsupported") else { return };
    let err = sdk::open(&launch, "New", "1.0", Timeouts::default()).await.unwrap_err();
    assert_eq!(err, OpenError::Unsupported("New speaks a newer connector protocol, which needs a newer Vorn".into()));

    let Some(launch) = fixture("crash-on-start") else { return };
    let err = sdk::open(&launch, "Crash", "1.0", Timeouts::default()).await.unwrap_err();
    let OpenError::Failed(message) = err else { panic!("a failed start") };
    assert!(message.starts_with("Crash did not answer vorn/hello: Crash exited (code 3)"), "{message}");
    assert!(message.contains("Error: the fixture could not start"), "{message}");
}
