//! The hosts against a real child: started with a token of their own,
//! answering footers and handlers, started again after a crash, stopped
//! with their project.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use vorn_extensions::host::{Asked, HostKey, HostSettings, Supervisor};
use vorn_extensions::pack::PackStore;

const FIXTURE: &str = include_str!("fixtures/extension.js");

fn install(root: &Path, id: &str, manifest: Value) {
    let dir = root.join(id).join("1.0.0");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("index.js"), FIXTURE).unwrap();
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    fs::write(
        root.join(id).join("current.json"),
        json!({ "version": "1.0.0", "installedAt": 1 }).to_string(),
    )
    .unwrap();
}

fn extension(id: &str) -> Value {
    json!({ "id": id, "name": id.to_uppercase(), "kind": "extension", "protocol": 1,
            "contributes": { "footers": [{ "id": "f", "every": 5 }] } })
}

struct World {
    _packs: tempfile::TempDir,
    project: tempfile::TempDir,
    hosts: Arc<Supervisor>,
}

fn world() -> World {
    let packs = tempfile::tempdir().unwrap();
    install(packs.path(), "x", extension("x"));
    install(packs.path(), "y", extension("y"));
    let mut outdated = extension("old");
    outdated.as_object_mut().unwrap().remove("protocol");
    install(packs.path(), "old", outdated);
    let hosts = Supervisor::new(
        PackStore::new(packs.path()),
        HostSettings {
            program: PathBuf::from("node"),
            base_env: std::env::vars()
                .filter(|(k, _)| k == "PATH" || k == "SYSTEMROOT")
                .collect(),
            bridge_origin: "http://127.0.0.1:9".into(),
            version: "test".into(),
        },
    );
    World {
        _packs: packs,
        project: tempfile::tempdir().unwrap(),
        hosts,
    }
}

fn key(world: &World, id: &str) -> HostKey {
    HostKey {
        extension_id: id.into(),
        project_path: world.project.path().to_string_lossy().into_owned(),
    }
}

const ASKED: Asked<'static> = Asked {
    session_id: "s1",
    worktree_path: "/w",
    agent: "claude",
};

fn value_of(items: &[vorn_extensions::footer::Item], label: &str) -> String {
    items
        .iter()
        .find(|i| i.label == label)
        .unwrap()
        .value
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_holds_its_own_token_and_answers() {
    let w = world();
    let x = key(&w, "x");
    let host = w.hosts.get_or_start(&x).await.unwrap();
    let items = host.footer("f", ASKED).await.unwrap();
    let token = value_of(&items, "token");
    assert_eq!(Some(token.clone()), w.hosts.token_for(&x));
    assert_eq!(
        value_of(&items, "host"),
        "http://127.0.0.1:9/extensions/x/bridge"
    );
    assert_eq!(
        value_of(&items, "cwd"),
        w.project.path().file_name().unwrap().to_string_lossy()
    );
    assert_eq!(value_of(&items, "session"), "s1");
    assert_eq!(host.footer("fail", ASKED).await.unwrap_err(), "no reading");
    assert_eq!(
        host.handler("h", ASKED, "https://a/pane")
            .await
            .unwrap()
            .as_deref(),
        Some("p")
    );
    assert_eq!(host.handler("h", ASKED, "https://a/").await.unwrap(), None);

    // A second extension in the same project cannot answer as the first.
    let y = w.hosts.get_or_start(&key(&w, "y")).await.unwrap();
    assert_ne!(y.token(), token);
    assert_eq!(w.hosts.by_token("x", &token), Some(x.clone()));
    assert_eq!(w.hosts.by_token("y", &token), None);
    assert_eq!(w.hosts.by_token("x", y.token()), None);

    // One start is shared by whoever asks meanwhile.
    let again = w.hosts.get_or_start(&x).await.unwrap();
    assert!(Arc::ptr_eq(&host, &again));

    w.hosts.stop_project(&x.project_path).await;
    assert!(w.hosts.running().is_empty());
    assert!(host.exited());
    assert_eq!(w.hosts.by_token("x", &token), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_crashed_host_starts_again_with_a_new_token() {
    let w = world();
    let x = key(&w, "x");
    let first = w.hosts.get_or_start(&x).await.unwrap();
    let old = first.token().to_owned();
    let err = first.footer("crash", ASKED).await.unwrap_err();
    assert!(
        err.contains("exited (code 3) before it answered extension/footer"),
        "{err}"
    );
    assert!(err.ends_with("Error: asked to crash"), "{err}");

    let mut restarted = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(token) = w.hosts.token_for(&x) {
            restarted = Some(token);
            break;
        }
    }
    let token = restarted.expect("the host started again");
    assert_ne!(token, old);
    assert_eq!(w.hosts.by_token("x", &old), None);
    let host = w.hosts.get_or_start(&x).await.unwrap();
    assert_eq!(
        value_of(&host.footer("f", ASKED).await.unwrap(), "token"),
        token
    );
    w.hosts.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn says_why_a_host_does_not_start() {
    let w = world();
    let err = |id: &str| {
        let hosts = Arc::clone(&w.hosts);
        let k = key(&w, id);
        async move { hosts.get_or_start(&k).await.unwrap_err() }
    };
    assert_eq!(err("nope").await, "No extension \"nope\" is installed");
    assert!(err("old")
        .await
        .starts_with("OLD was built for an older Vorn."));
}
