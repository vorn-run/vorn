//! The stage's end to end: a connector installs from a git URL after the
//! typed confirmation, builds, polls from a service, runs an action
//! one-shot, survives a crash and a timeout and passes its recorded checks;
//! a Bash extension linked from a folder answers an action; an idle service
//! stops and starts again on the next event. Skipped without git or node.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use vorn_extensions::api::types::{CallCtx, Request, Response};
use vorn_extensions::api::ExtError;
use vorn_extensions::child::Launch;
use vorn_extensions::connector::{self, SdkClient, SdkLaunch, Source, Timeouts};
use vorn_extensions::gate::{Answer, Caller, World};
use vorn_extensions::install::{self, Installs};
use vorn_extensions::lifecycle::{Lifecycle, Policy, ServiceKey};
use vorn_extensions::pack::{InstalledPack, PackStore};
use vorn_extensions::router::{Router, RouterSettings};

fn has(program: &str) -> bool {
    let found = Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipped: no {program} on PATH");
    }
    found
}

fn env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            ["PATH", "HOME", "SYSTEMROOT", "TMPDIR", "TEMP", "TMP"].contains(&k.as_str())
        })
        .collect()
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The pilot's own repository, committed.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture("pilot"), dir.path());
    for args in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "-A"],
        &["commit", "--quiet", "-m", "Feeds 1.2.0"],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }
    dir
}

/// The connector's program as a connection runs it, under the lifecycle.
fn launch(pack: &InstalledPack, argv: &[String]) -> SdkLaunch {
    let (program, args) = pack.resolve(argv).unwrap();
    SdkLaunch {
        launch: Launch {
            program,
            args,
            cwd: pack.path.clone(),
            env: env(),
        },
        source: Source::Pack,
        protocol: pack.manifest.protocol,
        redact: Vec::new(),
    }
}

fn timeouts() -> Timeouts {
    Timeouts {
        call: Duration::from_millis(800),
        ..Timeouts::default()
    }
}

async fn service(
    lifecycle: &Lifecycle,
    pack: &InstalledPack,
) -> vorn_extensions::lifecycle::Lease<SdkClient> {
    let key = ServiceKey::new(pack.id.clone(), "connection:c1");
    let argv = pack.manifest.service.clone().unwrap().command;
    lifecycle
        .get_or_start(&key, || async {
            connector::open(&launch(pack, &argv), "Feeds", "test", timeouts())
                .await
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connector_installs_from_git_and_polls_acts_crashes_and_times_out() {
    if !has("git") || !has("node") {
        return;
    }
    let source = repository();
    let root = tempfile::tempdir().unwrap();
    let installs = Installs::new(PackStore::new(root.path()), "git", env());
    let url = format!("file://{}", source.path().display());

    let started = Instant::now();
    let sheet = installs.inspect_git(&url, None).unwrap();
    eprintln!("inspect: {:?}", started.elapsed());
    assert_eq!(sheet.publisher, "Vorn tests");
    assert_eq!(sheet.hosts, ["feeds.example.com"]);
    assert!(
        sheet.adds.contains(&"Connector: Feeds".to_owned()),
        "{:?}",
        sheet.adds
    );
    assert!(sheet.runs_as_you.contains("runs as you"));
    // Nothing of it has run: its build would have made dist/.
    let staged: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .flat_map(|e| fs::read_dir(e.path()).into_iter().flatten().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!staged.contains(&"dist".to_owned()), "{staged:?}");
    assert!(installs.confirm(&sheet.token, "feeds").is_err());

    let started = Instant::now();
    let pack = installs.confirm(&sheet.token, "test.feeds").unwrap();
    eprintln!("confirm and build: {:?}", started.elapsed());
    assert!(pack.path.join("dist/built.txt").is_file(), "it built");
    assert_eq!(pack.manifest.protocol, Some(1));

    // Polls from a service, which the next poll reuses.
    let lifecycle = Lifecycle::new(Policy::default());
    let polled = service(&lifecycle, &pack)
        .await
        .poll("new-item", None, None)
        .await
        .unwrap();
    assert_eq!(polled["items"][0]["title"], "First post");
    let service_pid = polled["pid"].clone();
    let again = service(&lifecycle, &pack)
        .await
        .poll("new-item", Some("c1"), None)
        .await
        .unwrap();
    assert_eq!(again["pid"], service_pid);

    // Runs an action one-shot: a process of its own, gone after.
    let run = pack.manifest.run("read").unwrap();
    let started = Instant::now();
    let client = connector::open(&launch(&pack, &run.command), "Feeds", "test", timeouts())
        .await
        .unwrap();
    let mut args = Map::new();
    args.insert("url".into(), json!("https://feeds.example.com/rss"));
    let read = client.action("read", &args, None).await.unwrap();
    client.close().await;
    eprintln!("one-shot action: {:?}", started.elapsed());
    assert_eq!(read["read"], "https://feeds.example.com/rss");
    assert_ne!(read["pid"], service_pid);

    // Survives a crash: the call fails, the next poll starts it again.
    let crashed = service(&lifecycle, &pack)
        .await
        .action("crash", &Map::new(), None)
        .await
        .unwrap_err();
    assert!(
        crashed.message.contains("Error: the feed went away"),
        "{crashed}"
    );
    let after_crash = service(&lifecycle, &pack)
        .await
        .poll("new-item", None, None)
        .await
        .unwrap();
    assert_ne!(after_crash["pid"], service_pid);

    // Survives a timeout: the call fails in time, the service answers the next.
    let hung = service(&lifecycle, &pack)
        .await
        .action("hang", &Map::new(), None)
        .await
        .unwrap_err();
    assert!(hung.message.contains("did not answer action/run"), "{hung}");
    service(&lifecycle, &pack)
        .await
        .poll("new-item", None, None)
        .await
        .unwrap();

    // Passes its recorded checks.
    let packed = install::pack(&pack.path, &env(), None, &[]).unwrap();
    assert_eq!(
        packed.checks.as_deref(),
        Some("manifest ok\npoll ok\n2 checks passed")
    );

    lifecycle.stop_all().await;
    installs.remove("test.feeds").unwrap();
    assert!(fs::read_dir(root.path()).unwrap().next().is_none());
}

/// Nothing behind the gate: these extensions do not ask Vorn anything.
struct Nothing;

impl World for Nothing {
    fn call(&self, _: &Caller, m: &str, _: Value) -> Answer<Value> {
        let err = ExtError::new("unknown-method", m);
        Box::pin(async move { Err(err) })
    }
    fn fetch(&self, _: &Caller, _: Request) -> Answer<Response> {
        Box::pin(async { Err(ExtError::new("network", "none")) })
    }
    fn secret(&self, _: &Caller, _: &str, _: &str) -> Answer<Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

fn router(store: PackStore, lifecycle: Arc<Lifecycle>) -> Arc<Router> {
    Router::new(
        store,
        lifecycle,
        Arc::new(Nothing),
        RouterSettings::new(env(), "test"),
        vorn_extensions::events::Bus::new(),
    )
}

fn ctx() -> CallCtx {
    CallCtx {
        caller: "test".into(),
        project: None,
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bash_extension_linked_from_a_folder_answers_an_action() {
    if !has("bash") {
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    copy_dir(&fixture("hello-bash"), folder.path());
    let root = tempfile::tempdir().unwrap();
    let store = PackStore::new(root.path());
    Installs::new(store.clone(), "git", env())
        .link(folder.path())
        .unwrap();
    let r = router(store, Lifecycle::new(Policy::default()));
    let started = Instant::now();
    let answer = r
        .call("test.hello", "greet", json!({ "name": "Vorn" }), ctx())
        .await
        .unwrap();
    eprintln!("bash one-shot: {:?}", started.elapsed());
    assert_eq!(answer, json!({ "greeting": "hello Vorn" }));
    // Linked means live: an edit is what the next call runs.
    fs::write(
        folder.path().join("greet.sh"),
        "read -r _\nprintf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"edited\"}\\n'\n",
    )
    .unwrap();
    let edited = r
        .call("test.hello", "greet", json!({}), ctx())
        .await
        .unwrap();
    assert_eq!(edited, json!("edited"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_service_stops_and_starts_again_on_the_next_event() {
    if !has("node") {
        return;
    }
    let folder = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("vorn-extension.toml"),
        "id = \"test.ticks\"\nname = \"Ticks\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\n\n[service]\ncommand = [\"node\", \"program.mjs\"]\nevents = [\"test.tick\"]\n",
    )
    .unwrap();
    fs::copy(fixture("program.mjs"), folder.path().join("program.mjs")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = PackStore::new(root.path());
    Installs::new(store.clone(), "git", env())
        .link(folder.path())
        .unwrap();
    let lifecycle = Lifecycle::new(Policy {
        idle: Duration::from_millis(300),
        ..Policy::default()
    });
    let r = router(store, Arc::clone(&lifecycle));
    r.reindex();
    assert!(
        lifecycle.running().is_empty(),
        "nothing runs until something uses it"
    );

    assert_eq!(r.emit("test.tick", json!({ "n": 1 })), 1);
    assert_eq!(r.emit("test.other", json!({})), 0);
    tokio::time::timeout(Duration::from_secs(30), r.settled("test.ticks"))
        .await
        .unwrap();
    let first = r
        .call("test.ticks", "echo", json!({}), ctx())
        .await
        .unwrap()["pid"]
        .clone();
    // Idle stops it; thirty seconds is a safety net for a loaded machine, not the expected time.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !lifecycle.running().is_empty() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(lifecycle.running().is_empty(), "it stops once idle");

    assert_eq!(r.emit("test.tick", json!({ "n": 2 })), 1);
    tokio::time::timeout(Duration::from_secs(30), r.settled("test.ticks"))
        .await
        .unwrap();
    assert_eq!(lifecycle.running().len(), 1, "the next event starts it");
    let events = r
        .call("test.ticks", "events", json!({}), ctx())
        .await
        .unwrap();
    assert_eq!(events, json!([{ "name": "test.tick", "data": { "n": 2 } }]));
    let second = r
        .call("test.ticks", "echo", json!({}), ctx())
        .await
        .unwrap()["pid"]
        .clone();
    assert_ne!(first, second);
    lifecycle.stop_all().await;
}

/// The macOS sandbox profile: the pilot connector polls, acts and recovers
/// under it unchanged, and a program under it writes only inside its own
/// folder and opens no socket of its own.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_the_sandbox_profile_the_pilot_runs_and_writes_stay_in_its_folder() {
    use vorn_extensions::sandbox;
    if sandbox::Kind::here() != sandbox::Kind::Seatbelt
        || !has("git")
        || !has("node")
        || !has("bash")
    {
        return;
    }
    let source = repository();
    let root = tempfile::tempdir().unwrap();
    let installs = Installs::new(PackStore::new(root.path()), "git", env());
    let url = format!("file://{}", source.path().display());
    let sheet = installs.inspect_git(&url, None).unwrap();
    let pack = installs.confirm(&sheet.token, "test.feeds").unwrap();
    sandbox::set(&root.path().join("test.feeds"), true).unwrap();
    let held = |argv: &[String]| {
        let mut l = launch(&pack, argv);
        let mut reach = sandbox::Reach::of(&pack.path, None, None);
        reach.work = sandbox::Confiner::default().work_dir("test.feeds");
        l.launch = sandbox::wrap(l.launch, &reach, None).expect("Seatbelt confines");
        l
    };
    let service_argv = pack.manifest.service.clone().unwrap().command;
    let lifecycle = Lifecycle::new(Policy::default());
    let key = ServiceKey::new(pack.id.clone(), "connection:c1");
    let started = Instant::now();
    let polled = lifecycle
        .get_or_start(&key, || async {
            connector::open(&held(&service_argv), "Feeds", "test", timeouts())
                .await
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap()
        .poll("new-item", None, None)
        .await
        .unwrap();
    eprintln!("sandboxed start and poll: {:?}", started.elapsed());
    assert_eq!(polled["items"][0]["title"], "First post");
    let run = pack.manifest.run("read").unwrap();
    let client = connector::open(&held(&run.command), "Feeds", "test", timeouts())
        .await
        .unwrap();
    let mut args = Map::new();
    args.insert("url".into(), json!("https://feeds.example.com/rss"));
    let read = client.action("read", &args, None).await.unwrap();
    client.close().await;
    assert_eq!(read["read"], "https://feeds.example.com/rss");
    lifecycle.stop_all().await;

    // A program extension under the profile, through the router.
    let folder = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(
        folder.path().join("vorn-extension.toml"),
        "id = \"test.held\"\nname = \"Held\"\nversion = \"1.0.0\"\napi = \"vorn:extension@1.0\"\n\n[actions.try]\ncommand = [\"bash\", \"try.sh\"]\n",
    )
    .unwrap();
    let script = format!(
        r#"read -r _
inside=denied; echo x > "$VORN_EXTENSION_DATA/inside.txt" && inside=ok
own=denied; echo x > ./own.txt 2>/dev/null && own=ok
outside=denied; echo x > '{}/outside.txt' 2>/dev/null && outside=ok
socket=$( {{ exec 3<>/dev/tcp/127.0.0.1/9; }} 2>&1 )
case "$socket" in *"not permitted"*) socket=denied ;; *) socket=allowed ;; esac
printf '{{"jsonrpc":"2.0","id":1,"result":{{"inside":"%s","own":"%s","outside":"%s","socket":"%s"}}}}\n' $inside $own $outside $socket
"#,
        outside.path().display()
    );
    fs::write(folder.path().join("try.sh"), script).unwrap();
    let packs = tempfile::tempdir().unwrap();
    let store = PackStore::new(packs.path());
    Installs::new(store.clone(), "git", env())
        .link(folder.path())
        .unwrap();
    let r = router(store, Lifecycle::new(Policy::default()));
    // On by default; turned off, the same program reaches everything.
    assert!(r.sandboxed("test.held"));
    r.set_sandboxed("test.held", false).await.unwrap();
    let free = r.call("test.held", "try", json!({}), ctx()).await.unwrap();
    assert_eq!(
        free,
        json!({ "inside": "ok", "own": "ok", "outside": "ok", "socket": "allowed" })
    );
    fs::remove_file(outside.path().join("outside.txt")).unwrap();
    r.set_sandboxed("test.held", true).await.unwrap();
    assert!(r.sandboxed("test.held"));
    let started = Instant::now();
    let held = r.call("test.held", "try", json!({}), ctx()).await.unwrap();
    eprintln!("sandboxed bash one-shot: {:?}", started.elapsed());
    assert_eq!(
        held,
        json!({ "inside": "ok", "own": "denied", "outside": "denied", "socket": "denied" })
    );
    assert!(!outside.path().join("outside.txt").exists());
}
