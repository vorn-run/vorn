//! Every connector of the catalog installs from git as an extension and
//! passes its recorded checks: each folder of a connectors checkout
//! (`extensions/<id>`) is made a repository of its own, as it will be
//! published, installed after its sheet, checked as `vorn ext pack` checks
//! it, started and asked for its manifest. One is installed from its folder
//! of the checkout itself, as the catalog names it today.
//!
//! Runs when `VORN_CONNECTORS_CHECKOUT` names a checkout of the connectors
//! repository with its folders built; skipped otherwise, and without git or node.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use vorn_extensions::child::Launch;
use vorn_extensions::connector::{self, SdkLaunch, Source, Timeouts};
use vorn_extensions::install::{self, Installs};
use vorn_extensions::pack::PackStore;

fn has(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            ["PATH", "HOME", "SYSTEMROOT", "TMPDIR", "TEMP", "TMP"].contains(&k.as_str())
        })
        .collect()
}

fn checkout() -> Option<PathBuf> {
    let Some(root) = std::env::var_os("VORN_CONNECTORS_CHECKOUT").map(PathBuf::from) else {
        eprintln!("skipped: VORN_CONNECTORS_CHECKOUT names no connectors checkout");
        return None;
    };
    if !has("git") || !has("node") {
        eprintln!("skipped: needs git and node on PATH");
        return None;
    }
    Some(root)
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

/// `folder` as a repository of its own, committed.
fn repository_of(folder: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(folder, dir.path());
    for args in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "-A"],
        &["commit", "--quiet", "-m", "release"],
    ] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_connector_installs_from_git_and_passes_its_recorded_checks() {
    let Some(root) = checkout() else { return };
    let folders: Vec<PathBuf> = fs::read_dir(root.join("extensions"))
        .expect("the checkout has its extensions built")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("vorn-extension.toml").is_file())
        .collect();
    assert_eq!(folders.len(), 22, "{folders:?}");
    let store_root = tempfile::tempdir().unwrap();
    let installs = Installs::new(PackStore::new(store_root.path()), "git", env());
    for folder in &folders {
        let repo = repository_of(folder);
        let url = format!("file://{}", repo.path().display());
        let sheet = installs
            .inspect_git(&url, None)
            .unwrap_or_else(|e| panic!("{}: {e}", folder.display()));
        let id = sheet.id.clone();
        assert!(
            !sheet.hosts.is_empty()
                || !sheet.hosts_from.is_empty()
                || ["mysql", "postgres"].contains(&id.as_str()),
            "{id} declares no host it reaches"
        );
        let pack = installs.confirm(&sheet.token, &id).unwrap();
        assert_eq!(pack.manifest.protocol, Some(1), "{id}");

        // Its recorded checks, run as `vorn ext pack` runs them.
        let packed =
            install::pack(&pack.path, &env(), None).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(packed.checks.is_some(), "{id} records no checks");

        // It starts, greets and says what it is.
        let service = pack.manifest.service.clone().unwrap().command;
        let (program, args) = pack.resolve(&service).unwrap();
        let launch = SdkLaunch {
            launch: Launch {
                program,
                args,
                cwd: pack.path.clone(),
                env: env(),
            },
            source: Source::Pack,
            protocol: pack.manifest.protocol,
        };
        let client = connector::open(&launch, &id, "test", Timeouts::default())
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let manifest = client
            .manifest()
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(
            (manifest.id.as_str(), manifest.version.as_str()),
            (id.as_str(), pack.version.as_str())
        );
        client.close().await;
    }
    assert_eq!(installs.store().list().len(), 22);

    // As the catalog names one today: a folder of the connectors repository.
    let whole = repository_of(&root.join("extensions"));
    let other = tempfile::tempdir().unwrap();
    let installs = Installs::new(PackStore::new(other.path()), "git", env());
    let url = format!("file://{}", whole.path().display());
    let sheet = installs.inspect_git_at(&url, None, Some("rss")).unwrap();
    assert_eq!(sheet.hosts_from, ["RSS_FEEDS"]);
    installs.confirm(&sheet.token, "rss").unwrap();
}
