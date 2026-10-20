//! A theme installs from git like any extension: a repository holding only
//! a `vorn-extension.toml` with a `[theme]` section is inspected, confirmed
//! and listed to clients as a `themes` contribution.
//!
//! Skipped without git on PATH.

use std::fs;
use std::process::Command;

use vorn_extensions::install::Installs;
use vorn_extensions::pack::PackStore;
use vorn_extensions::points;

const MANIFEST: &str = r##"id = "acme.dusk"
name = "Acme Dusk"
version = "1.0.0"
api = "vorn:extension@0.9"

[theme]
appearance = "dark"

[theme.ui]
accent = "#d08770"

[theme.terminal]
foreground = "#dddddd"
background = "#111111"
ansi = [
  "#000000", "#cc3333", "#33cc33", "#cccc33", "#3333cc", "#cc33cc", "#33cccc", "#cccccc",
  "#555555", "#ff5555", "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff",
]
"##;

fn env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| {
            ["PATH", "HOME", "SYSTEMROOT", "TMPDIR", "TEMP", "TMP"].contains(&k.as_str())
        })
        .collect()
}

fn git(dir: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_theme_installs_from_git_and_is_listed() {
    let repo = tempfile::tempdir().unwrap();
    if !git(repo.path(), &["init", "--quiet", "--initial-branch=main"]) {
        eprintln!("skipped: needs git on PATH");
        return;
    }
    fs::write(repo.path().join("vorn-extension.toml"), MANIFEST).unwrap();
    assert!(git(repo.path(), &["add", "-A"]));
    assert!(git(repo.path(), &["commit", "--quiet", "-m", "release"]));

    let store = tempfile::tempdir().unwrap();
    let installs = Installs::new(PackStore::new(store.path()), "git", env());
    let url = format!("file://{}", repo.path().display());
    let sheet = installs.inspect_git(&url, None).unwrap();
    assert_eq!(sheet.id, "acme.dusk");
    let pack = installs.confirm(&sheet.token, "acme.dusk").unwrap();

    let theme = pack.manifest.theme().expect("its theme survives the install");
    assert_eq!(theme.name, "Acme Dusk");
    assert_eq!(
        theme.get(vorn_theme::Token::Accent),
        vorn_theme::Rgba8::rgb(0xd08770)
    );
    let listed = points::listed(&pack.manifest);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["kind"], "themes");
    assert_eq!(listed[0]["appearance"], "dark");
    assert_eq!(listed[0]["theme"]["terminal"]["background"], "#111111");
    assert_eq!(installs.store().list().len(), 1);
}
