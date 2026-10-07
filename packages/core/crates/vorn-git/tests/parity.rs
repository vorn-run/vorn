//! Every command gix answers, against git itself, in the repositories that
//! make either of them think twice. An answer from gix must be what git prints,
//! byte for byte; where gix declines, git's own answer (or failure) is what the
//! caller gets, so declining is always allowed and answering wrong never is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use vorn_git::{run, run_git, Engine, Request};

const FAST: &[&[&str]] = &[
    &["rev-parse", "--is-inside-work-tree"],
    &["rev-parse", "--show-toplevel"],
    &["rev-parse", "--absolute-git-dir"],
    &["rev-parse", "HEAD"],
    &["rev-parse", "--abbrev-ref", "HEAD"],
];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {dir:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn request(cwd: &Path, args: &[&str]) -> Request {
    Request {
        bin: "git".into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: cwd.to_path_buf(),
        env: std::env::vars().collect(),
        timeout: Duration::from_secs(10),
        max_buffer: 1024 * 1024,
    }
}

/// A repository with one commit on `main`.
fn repo(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@vorn.invalid"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("src/deep/a.txt"), "a\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "one"]);
    dir
}

/// Asserts gix and git agree on every fast command in `cwd`, and returns which
/// of them gix answered itself.
fn agree(cwd: &Path) -> Vec<String> {
    let mut answered = Vec::new();
    for args in FAST {
        let req = request(cwd, args);
        let git_says = run_git(&req).map_err(|e| e.to_string());
        let ours = run(&req);
        match ours {
            Ok(reply) if reply.engine == Engine::Gix => {
                assert_eq!(
                    Ok(reply.stdout.clone()),
                    git_says,
                    "gix answered `git {}` in {cwd:?} differently from git",
                    args.join(" ")
                );
                answered.push(args.join(" "));
            }
            Ok(reply) => assert_eq!(Ok(reply.stdout), git_says, "{args:?} in {cwd:?}"),
            Err(err) => assert!(git_says.is_err(), "{args:?} in {cwd:?}: {err}"),
        }
    }
    answered
}

/// All of them, where gix answers at all. In an environment that steers git
/// (CI sandboxes inject config through `GIT_CONFIG_COUNT`) gix declines
/// everything, and the comparisons above still hold.
fn all() -> usize {
    if vorn_git::gix_answers_here() {
        FAST.len()
    } else {
        eprintln!("gix declines in this environment; checking git's answers only");
        0
    }
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_plain_repository_from_its_root_and_below() {
    let t = tmp();
    let dir = repo(t.path(), "plain");
    assert_eq!(
        agree(&dir).len(),
        all(),
        "gix should answer all of them here"
    );
    assert_eq!(agree(&dir.join("src/deep")).len(), all());
}

#[cfg(unix)]
#[test]
fn a_repository_reached_through_a_symlink() {
    let t = tmp();
    let dir = repo(t.path(), "real");
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    assert_eq!(agree(&link).len(), all());
    assert_eq!(agree(&link.join("src")).len(), all());
}

#[test]
fn a_linked_worktree_on_a_branch_with_a_slash() {
    let t = tmp();
    let dir = repo(t.path(), "main");
    let wt = t.path().join("wt");
    git(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/x",
            wt.to_str().unwrap(),
        ],
    );
    assert_eq!(agree(&wt).len(), all());
    assert_eq!(agree(&wt.join("src/deep")).len(), all());
    assert_eq!(
        run(&request(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]))
            .unwrap()
            .stdout,
        "feature/x\n"
    );
}

#[test]
fn a_detached_head() {
    let t = tmp();
    let dir = repo(t.path(), "detached");
    git(&dir, &["checkout", "-q", "--detach"]);
    assert_eq!(agree(&dir).len(), all());
}

#[test]
fn a_repository_with_no_commits_yet() {
    let t = tmp();
    let dir = t.path().join("unborn");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let answered = agree(&dir);
    assert!(!answered.contains(&"rev-parse HEAD".to_string()));
    assert!(!answered.contains(&"rev-parse --abbrev-ref HEAD".to_string()));
}

#[test]
fn a_bare_repository_and_the_inside_of_a_git_directory() {
    let t = tmp();
    let bare = t.path().join("bare.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare"]);
    assert!(agree(&bare).is_empty());
    let dir = repo(t.path(), "inside");
    assert!(agree(&dir.join(".git")).is_empty());
    assert!(agree(&dir.join(".git/refs")).is_empty());
}

#[test]
fn a_branch_that_shares_its_name_with_a_tag() {
    let t = tmp();
    let dir = repo(t.path(), "ambiguous");
    git(&dir, &["checkout", "-q", "-b", "dup"]);
    git(&dir, &["tag", "dup"]);
    let answered = agree(&dir);
    assert!(!answered.contains(&"rev-parse --abbrev-ref HEAD".to_string()));
    // And git's own answer comes back, disambiguated.
    assert_eq!(
        run(&request(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]))
            .unwrap()
            .stdout,
        "heads/dup\n"
    );
}

#[test]
fn a_directory_in_no_repository() {
    let t = tmp();
    let plain = t.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    // A ceiling, so the search cannot walk up into a repository around the temp dir.
    let mut req = request(&plain, &["rev-parse", "--is-inside-work-tree"]);
    req.env.push((
        "GIT_CEILING_DIRECTORIES".into(),
        t.path().display().to_string(),
    ));
    assert!(run(&req).is_err());
    assert!(run_git(&req).is_err());
}

#[test]
fn a_submodule_which_sets_core_worktree() {
    let t = tmp();
    let inner = repo(t.path(), "inner");
    let outer = repo(t.path(), "outer");
    git(
        &outer,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            inner.to_str().unwrap(),
            "sub",
        ],
    );
    agree(&outer.join("sub"));
    assert_eq!(agree(&outer).len(), all());
}

#[test]
fn an_environment_that_steers_git_leaves_it_to_git() {
    let t = tmp();
    let dir = repo(t.path(), "steered");
    let other = repo(t.path(), "other");
    let mut req = request(&dir, &["rev-parse", "--show-toplevel"]);
    req.env
        .push(("GIT_DIR".into(), other.join(".git").display().to_string()));
    let reply = run(&req).unwrap();
    assert_eq!(reply.engine, Engine::Git);
}

#[test]
fn a_command_gix_does_not_know_runs_git() {
    let t = tmp();
    let dir = repo(t.path(), "other-cmds");
    let reply = run(&request(&dir, &["log", "-1", "--format=%s"])).unwrap();
    assert_eq!(reply.engine, Engine::Git);
    assert_eq!(reply.stdout, "one\n");
}

#[test]
fn git_failures_read_as_node_words_them() {
    let t = tmp();
    let dir = repo(t.path(), "fails");
    let err = run(&request(&dir, &["checkout", "no-such-branch"])).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.starts_with("Command failed: git checkout no-such-branch\n"),
        "{msg}"
    );
    assert!(msg.contains("no-such-branch"), "{msg}");
}

#[test]
fn output_past_the_limit_is_an_error_not_a_truncation() {
    let t = tmp();
    let dir = repo(t.path(), "big");
    std::fs::write(dir.join("big.txt"), "x\n".repeat(100_000)).unwrap();
    let mut req = request(&dir, &["diff", "--no-index", "/dev/null", "big.txt"]);
    req.max_buffer = 1000;
    assert!(matches!(run(&req), Err(vorn_git::Error::TooLarge { .. })));
}

#[cfg(unix)]
#[test]
fn a_command_past_its_timeout_is_stopped() {
    let t = tmp();
    let mut req = request(t.path(), &[]);
    req.bin = "sleep".into();
    req.args = vec!["5".into()];
    req.timeout = Duration::from_millis(100);
    let started = std::time::Instant::now();
    assert!(matches!(run(&req), Err(vorn_git::Error::TimedOut { .. })));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn stderr_past_the_limit_is_an_error_too() {
    let t = tmp();
    let dir = repo(t.path(), "noisy");
    // git names every missing path on stderr, and fails.
    let mut req = request(&dir, &["rm", "--cached", "-q"]);
    for i in 0..400 {
        req.args
            .push(format!("missing-path-with-a-long-enough-name-{i}"));
    }
    req.max_buffer = 64;
    assert!(
        matches!(run(&req), Err(vorn_git::Error::TooLarge { .. })),
        "{:?}",
        run(&req)
    );
}

#[cfg(unix)]
#[test]
fn a_git_wrapper_on_path_is_run_not_bypassed() {
    use std::os::unix::fs::PermissionsExt;
    let t = tmp();
    let dir = repo(t.path(), "wrapped");
    let bin = t.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let wrapper = bin.join("git");
    std::fs::write(&wrapper, "#!/bin/sh\necho wrapped\n").unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    for bin_name in [wrapper.display().to_string(), "git".to_string()] {
        let mut req = request(&dir, &["rev-parse", "--show-toplevel"]);
        req.bin = bin_name;
        req.env.retain(|(key, _)| key != "PATH");
        req.env.push(("PATH".into(), bin.display().to_string()));
        // A test spawning on another thread can hold the script's write
        // descriptor across its fork for a moment, so exec may see it busy.
        let mut tries = 0;
        let reply = loop {
            match run(&req) {
                Err(vorn_git::Error::Spawn { error, .. })
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 50 =>
                {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => break other.unwrap(),
            }
        };
        assert_eq!(reply.stdout, "wrapped\n");
        assert_eq!(reply.engine, Engine::Git);
    }
}

#[test]
fn checks_out_a_branch_reads_head_and_names_a_worktree_before_making_it() {
    use vorn_git::repo::{Done, Git};
    let t = tmp();
    let dir = repo(t.path(), "checkout");
    git(&dir, &["branch", "side"]);
    let g = Git {
        bin: "git".into(),
        env: std::env::vars().collect(),
    };
    let head = git(&dir, &["rev-parse", "HEAD"]);
    assert_eq!(g.head(&dir).as_deref(), Some(head.trim()));
    assert_eq!(g.head(t.path()), None);

    assert_eq!(g.checkout(&dir, "side"), Done::Ok);
    assert_eq!(g.branch(&dir).as_deref(), Some("side"));
    assert!(matches!(
        g.checkout(&dir, "no-such-branch"),
        Done::Failed(_)
    ));

    let mut told = None;
    let made = g
        .create_worktree_at(dir.to_str().unwrap(), "fresh", Some("my tree"), |p| {
            // Told before git has made anything there.
            assert!(!Path::new(p).exists());
            told = Some(p.to_owned());
        })
        .unwrap();
    assert_eq!(told.as_deref(), Some(made.worktree_path.as_str()));
    assert_eq!(
        (made.branch.as_str(), made.name.as_str()),
        ("fresh", "my-tree")
    );
    assert_eq!(
        g.branch(Path::new(&made.worktree_path)).as_deref(),
        Some("fresh")
    );
}

#[test]
fn renames_a_worktree_branch_and_moves_a_worktree_as_foreseen() {
    use vorn_git::repo::{Git, MovedWorktree};
    let t = tmp();
    let dir = repo(t.path(), "rename");
    git(&dir, &["branch", "taken"]);
    git(&dir, &["branch", "nest/inner"]);
    let g = Git {
        bin: "git".into(),
        env: std::env::vars().collect(),
    };
    let made = g
        .create_worktree(dir.to_str().unwrap(), "first", Some("tree"))
        .unwrap();
    let wt = PathBuf::from(&made.worktree_path);
    // What gix foresees, when it does, is what the rename then does.
    let rename = |name: &str| {
        let foreseen = g.foresee_branch_rename(&wt, name);
        assert!(
            foreseen.is_some() || !vorn_git::gix_answers_here(),
            "{name:?}"
        );
        let done = g.rename_branch(&wt, name);
        if let Some(foreseen) = foreseen {
            assert_eq!(foreseen, done, "{name:?}");
        }
        done
    };
    assert!(!rename("  "));
    assert!(!rename("-x"));
    assert!(!rename("taken"));
    assert!(!rename("nest"));
    assert!(!rename("bad..name"));
    assert!(rename("  second  "));
    assert_eq!(g.branch(&wt).as_deref(), Some("second"));
    assert!(rename("second"));
    // Detached: a branch is started there instead.
    git(&wt, &["checkout", "-q", "--detach"]);
    assert!(!rename("taken"));
    assert!(rename("third"));
    assert_eq!(g.branch(&wt).as_deref(), Some("third"));

    let path = made.worktree_path.as_str();
    let parent = &path[..path.rfind('/').unwrap()];
    let id = &path[path.len() - 8..];
    let moved = |p: &str, name: &str| {
        let foreseen = g.foresee_worktree_move(p, name);
        let done = g.move_worktree(p, name);
        assert_eq!(foreseen, done, "{p} {name:?}");
        done
    };
    assert_eq!(moved(path, "!!"), None);
    assert_eq!(moved(path, "tree"), None);
    assert_eq!(moved(dir.to_str().unwrap(), "x"), None);
    std::fs::create_dir(format!("{parent}/blocked-{id}")).unwrap();
    assert_eq!(moved(path, "blocked"), None);
    let to = moved(path, " New  Name! ").unwrap();
    assert_eq!(
        to,
        MovedWorktree {
            path: format!("{parent}/New-Name-{id}"),
            name: "New-Name".into()
        }
    );
    assert!(!wt.exists());
    assert_eq!(g.branch(Path::new(&to.path)).as_deref(), Some("third"));
}
