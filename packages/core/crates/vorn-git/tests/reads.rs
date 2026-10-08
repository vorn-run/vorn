//! The plain reads an extension asks for: the diff, the status and the origin.

use std::path::Path;
use std::process::Command;

use vorn_git::repo::Git;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

fn tool() -> Git {
    Git {
        bin: "git".into(),
        env: std::env::vars().collect(),
    }
}

#[test]
fn reads_the_diff_status_and_origin_of_a_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@vorn.invalid"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "one"]);
    let g = tool();
    assert_eq!(g.diff_text(dir).unwrap(), "");
    assert_eq!(g.status_porcelain(dir).unwrap(), "");
    assert_eq!(g.origin_url(dir), None);

    std::fs::write(dir.join("a.txt"), "b\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    let diff = g.diff_text(dir).unwrap();
    assert!(diff.starts_with("diff --git a/a.txt b/a.txt"), "{diff}");
    assert!(diff.ends_with("+b"), "trimmed as gitExec trims: {diff:?}");
    assert_eq!(g.status_porcelain(dir).unwrap(), "M a.txt\n?? new.txt");

    git(dir, &["remote", "add", "origin", "git@github.com:vorn-run/vorn.git"]);
    assert_eq!(
        g.origin_url(dir).as_deref(),
        Some("git@github.com:vorn-run/vorn.git")
    );
}

#[test]
fn a_long_diff_is_cut_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "t@vorn.invalid"]);
    git(dir, &["config", "user.name", "t"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("big.txt"), "").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "one"]);
    std::fs::write(dir.join("big.txt"), "x".repeat(80).repeat(8000) + "\n").unwrap();
    let diff = tool().diff_text(dir).unwrap();
    assert!(diff.ends_with("\n\n... diff truncated (too large) ...\n"));
    assert!(diff.len() <= vorn_git::repo::MAX_DIFF_TEXT_BYTES + 64);
}

#[test]
fn a_directory_that_is_no_repository_fails() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(tool().status_porcelain(tmp.path()).is_err());
    assert!(tool().diff_text(tmp.path()).is_err());
}
