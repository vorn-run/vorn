//! The inventory and the cleanup actions against real repositories: a
//! worktree a session runs in is never removed, uncommitted work is reported
//! and kept, and every path ends up on one side of the result or the other.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde_json::{json, Value};
use vorn_git::repo::Git;
use vorn_worktrees::{
    prune_orphan_dirs, reclaim_artifacts, remove_worktrees, scan, Cleanup, Guard, Inventory,
    Project, RemoveItem, Retention, Scan, Sizes,
};

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

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `p` resolved, without the `\\?\` prefix Windows adds, which git cannot read.
fn real(p: &Path) -> PathBuf {
    let real = std::fs::canonicalize(p).unwrap();
    match s(&real).strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => real,
    }
}

/// A project on `main` with worktrees: `gilded-fresco` merged and clean,
/// `amber-muse` with an unmerged commit, `royal-chapel` with uncommitted
/// work, `quiet-loom` where a session runs, and `stray`, a directory git
/// has forgotten.
struct Fixture {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    base: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        // Git reports resolved paths; macOS's temp directory is a symlink.
        let root = real(tmp.path());
        let project = root.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["config", "user.email", "t@vorn.invalid"]);
        git(&project, &["config", "user.name", "t"]);
        git(&project, &["config", "commit.gpgsign", "false"]);
        std::fs::write(project.join("a.txt"), "a\n").unwrap();
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-q", "-m", "one"]);
        let base = root.join(".vorn-worktrees/proj");
        for name in ["gilded-fresco", "amber-muse", "royal-chapel", "quiet-loom"] {
            let at = s(&base.join(name));
            git(&project, &["worktree", "add", "-q", "-b", name, &at]);
            std::fs::create_dir_all(base.join(name).join("node_modules/pkg")).unwrap();
            std::fs::write(
                base.join(name).join("node_modules/pkg/i.js"),
                vec![b'x'; 8192],
            )
            .unwrap();
        }
        let amber = base.join("amber-muse");
        std::fs::write(amber.join("b.txt"), "b\n").unwrap();
        git(&amber, &["add", "b.txt"]);
        git(&amber, &["commit", "-q", "-m", "two"]);
        std::fs::write(base.join("royal-chapel/a.txt"), "changed\n").unwrap();
        std::fs::create_dir_all(base.join("stray/src")).unwrap();
        std::fs::write(base.join("stray/src/c.txt"), vec![b'c'; 4096]).unwrap();
        // node_modules is ignored, as a project would have it.
        std::fs::write(project.join(".git/info/exclude"), "node_modules\n").unwrap();
        Fixture {
            _tmp: tmp,
            project,
            base,
        }
    }

    /// A worktree's path as git lists it and so as a client sends it back:
    /// with forward slashes, on Windows too.
    fn wt(&self, name: &str) -> String {
        s(&self.base.join(name)).replace(std::path::MAIN_SEPARATOR, "/")
    }

    fn projects(&self) -> Vec<Project> {
        vec![Project {
            name: "proj".into(),
            path: s(&self.project),
            host_ids: vec!["local".into()],
        }]
    }
}

fn local_git() -> Git {
    Git {
        bin: "git".into(),
        env: std::env::vars().collect(),
    }
}

/// Sessions run in `live`; every turn taken is recorded.
struct Sessions {
    live: HashSet<String>,
    turns: RefCell<Vec<PathBuf>>,
}

impl Sessions {
    fn running_in(paths: &[String]) -> Sessions {
        Sessions {
            live: paths.iter().cloned().collect(),
            turns: RefCell::default(),
        }
    }
}

impl Guard for Sessions {
    fn assert_idle(&self, path: &str) -> Result<(), String> {
        if self.live.contains(path) {
            return Err(format!("{path} has 1 active session — close them first"));
        }
        Ok(())
    }

    fn turn(
        &self,
        project: &Path,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        self.turns.borrow_mut().push(project.to_path_buf());
        f()
    }
}

fn inventory(fx: &Fixture, live: &[String]) -> Inventory {
    let git = local_git();
    let sizes = Sizes::default();
    let retention = Retention::from_config(Some(&json!({ "pinnedPaths": [fx.wt("amber-muse")] })));
    let active = |p: &str| {
        if live.iter().any(|l| l == p) {
            vec!["t-1".to_owned()]
        } else {
            Vec::new()
        }
    };
    scan(
        &fx.projects(),
        &[],
        &Scan {
            git: &git,
            sizes: &sizes,
            retention: &retention,
            refresh: false,
            now: Utc::now(),
            active: &active,
        },
    )
}

fn entry<'a>(inv: &'a Value, path: &str) -> &'a Value {
    inv["projects"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"].as_str().map(Path::new) == Some(Path::new(path)))
        .unwrap_or_else(|| panic!("no entry for {path}"))
}

#[test]
fn the_inventory_judges_every_worktree_and_forgotten_directory() {
    let fx = Fixture::new();
    let live = vec![fx.wt("quiet-loom")];
    let inv = serde_json::to_value(inventory(&fx, &live)).unwrap();
    let project = &inv["projects"][0];
    assert_eq!(project["defaultBranch"], "main");
    assert_eq!(project["remoteHostId"], Value::Null);
    assert!(project.get("error").is_none());
    let keys: Vec<&str> = project["entries"][1]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "path",
            "name",
            "projectPath",
            "projectName",
            "kind",
            "branch",
            "isMain",
            "sizeBytes",
            "artifactBytes",
            "sizeMeasured",
            "lastCommitAt",
            "lastTouchedAt",
            "idleDays",
            "isDirty",
            "isMerged",
            "hasUpstream",
            "activeSessionIds",
            "verdict",
        ]
    );

    let main = &project["entries"][0];
    assert_eq!(main["isMain"], true);
    assert_eq!(main["verdict"]["level"], "keep");
    assert_eq!(main["verdict"]["reasons"], json!(["main worktree"]));

    let merged = entry(&inv, &fx.wt("gilded-fresco"));
    assert_eq!(merged["isMerged"], true);
    assert_eq!(merged["isDirty"], false);
    assert_eq!(merged["kind"], "registered");
    assert_eq!(merged["idleDays"], 0);
    assert!(merged["sizeBytes"].as_u64().unwrap() >= merged["artifactBytes"].as_u64().unwrap());
    assert!(merged["artifactBytes"].as_u64().unwrap() >= 8192);

    let dirty = entry(&inv, &fx.wt("royal-chapel"));
    assert_eq!(dirty["isDirty"], true);
    assert_eq!(dirty["verdict"]["level"], "review");
    assert!(dirty["verdict"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("uncommitted changes")));

    let pinned = entry(&inv, &fx.wt("amber-muse"));
    assert_eq!(pinned["isMerged"], false);
    assert_eq!(pinned["verdict"]["level"], "keep");

    let used = entry(&inv, &fx.wt("quiet-loom"));
    assert_eq!(used["activeSessionIds"], json!(["t-1"]));
    assert_eq!(used["verdict"]["level"], "keep");
    assert_eq!(used["verdict"]["autoSelect"], false);

    let stray = entry(&inv, &fx.wt("stray"));
    assert_eq!(stray["kind"], "orphan-dir");
    assert_eq!(stray["name"], "stray");
    assert_eq!(stray["branch"], Value::Null);
    assert_eq!(project["entries"].as_array().unwrap().len(), 6);
    assert_eq!(project["staleBranches"], json!([]));
}

#[test]
fn a_project_that_is_not_a_repository_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let git = local_git();
    let sizes = Sizes::default();
    let retention = Retention::from_config(None);
    let projects = vec![
        Project {
            name: "plain".into(),
            path: s(tmp.path()),
            host_ids: vec!["local".into()],
        },
        Project {
            name: "again".into(),
            path: s(tmp.path()),
            host_ids: vec!["local".into()],
        },
    ];
    let inv = scan(
        &projects,
        &[],
        &Scan {
            git: &git,
            sizes: &sizes,
            retention: &retention,
            refresh: true,
            now: Utc::now(),
            active: &|_| Vec::new(),
        },
    );
    assert_eq!(inv.projects.len(), 1, "one scan per project and hosts");
    assert_eq!(
        inv.projects[0].error.as_deref(),
        Some("not a git repository")
    );
    let wanted = scan(
        &projects,
        &["/elsewhere".into()],
        &Scan {
            git: &git,
            sizes: &sizes,
            retention: &retention,
            refresh: false,
            now: Utc::now(),
            active: &|_| Vec::new(),
        },
    );
    assert!(wanted.projects.is_empty());
}

#[test]
fn a_worktree_a_live_session_uses_is_never_removed() {
    let fx = Fixture::new();
    let used = fx.wt("quiet-loom");
    let guard = Sessions::running_in(std::slice::from_ref(&used));
    let git = local_git();
    let sizes = Sizes::default();
    let dirs = vec!["node_modules".to_owned()];
    let projects = fx.projects();
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &dirs,
        projects: &projects,
        guard: &guard,
    };
    let refused = format!("{used} has 1 active session — close them first");

    let removed = remove_worktrees(
        &[RemoveItem {
            worktree_path: used.clone(),
            force: true,
            delete_branch: true,
        }],
        &ctx,
    );
    assert!(removed.succeeded.is_empty());
    assert_eq!(removed.failed[0].error, refused);
    assert!(
        guard.turns.borrow().is_empty(),
        "refused before taking a turn"
    );

    let reclaimed = reclaim_artifacts(std::slice::from_ref(&used), &ctx);
    assert_eq!(reclaimed.failed[0].error, refused);
    assert!(Path::new(&used).join("node_modules/pkg/i.js").exists());

    // Gone from git but a session still runs there: the directory stays.
    git_worktree_forget(&fx, "quiet-loom");
    let pruned = prune_orphan_dirs(std::slice::from_ref(&used), &ctx);
    assert_eq!(pruned.failed[0].error, refused);
    assert_eq!(pruned.freed_bytes, 0);
    assert!(Path::new(&used).join("a.txt").exists());
    assert!(crate::git(&fx.project, &["branch", "--list", "quiet-loom"]).contains("quiet-loom"));
}

/// A session that starts while the action runs is caught by the check
/// inside the repository's turn.
#[test]
fn a_session_that_starts_during_the_removal_stops_it() {
    struct StartsLate(RefCell<u32>);
    impl Guard for StartsLate {
        fn assert_idle(&self, path: &str) -> Result<(), String> {
            *self.0.borrow_mut() += 1;
            if *self.0.borrow() > 1 {
                return Err(format!("{path} has a session starting — close it first"));
            }
            Ok(())
        }
        fn turn(&self, _: &Path, f: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String> {
            f()
        }
    }
    let fx = Fixture::new();
    let path = fx.wt("gilded-fresco");
    let guard = StartsLate(RefCell::new(0));
    let git = local_git();
    let sizes = Sizes::default();
    let projects = fx.projects();
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &[],
        projects: &projects,
        guard: &guard,
    };
    let item = RemoveItem {
        worktree_path: path.clone(),
        force: false,
        delete_branch: false,
    };
    let result = remove_worktrees(&[item], &ctx);
    assert_eq!(
        result.failed[0].error,
        format!("{path} has a session starting — close it first")
    );
    assert!(Path::new(&path).exists());
}

#[test]
fn uncommitted_work_is_reported_not_removed() {
    let fx = Fixture::new();
    let dirty = fx.wt("royal-chapel");
    let inv = serde_json::to_value(inventory(&fx, &[])).unwrap();
    assert_eq!(entry(&inv, &dirty)["isDirty"], true);
    assert_eq!(entry(&inv, &dirty)["verdict"]["autoSelect"], false);

    let guard = Sessions::running_in(&[]);
    let git = local_git();
    let sizes = Sizes::default();
    let projects = fx.projects();
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &[],
        projects: &projects,
        guard: &guard,
    };
    let result = remove_worktrees(
        &[RemoveItem {
            worktree_path: dirty.clone(),
            force: false,
            delete_branch: true,
        }],
        &ctx,
    );
    assert!(result.succeeded.is_empty());
    assert_eq!(result.failed[0].error, "git worktree remove failed");
    assert!(result.deleted_branches.is_empty());
    assert_eq!(
        std::fs::read_to_string(Path::new(&dirty).join("a.txt")).unwrap(),
        "changed\n"
    );
    assert_eq!(
        guard.turns.borrow().as_slice(),
        std::slice::from_ref(&fx.project)
    );
}

#[test]
fn removing_a_merged_worktree_takes_its_branch_and_reports_the_bytes() {
    let fx = Fixture::new();
    let merged = fx.wt("gilded-fresco");
    let unmerged = fx.wt("amber-muse");
    let guard = Sessions::running_in(&[]);
    let git = local_git();
    let sizes = Sizes::default();
    let projects = fx.projects();
    let dirs = vec!["node_modules".to_owned()];
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &dirs,
        projects: &projects,
        guard: &guard,
    };
    let item = |path: &str| RemoveItem {
        worktree_path: path.to_owned(),
        force: true,
        delete_branch: true,
    };
    let result = remove_worktrees(
        &[
            item(&merged),
            item(&unmerged),
            item(&s(&fx.project)),
            item("/nowhere"),
        ],
        &ctx,
    );
    assert_eq!(result.succeeded, vec![merged.clone(), unmerged.clone()]);
    assert!(result.freed_bytes >= 2 * 8192);
    // `-d` keeps a branch with commits nowhere else.
    assert_eq!(result.deleted_branches, vec!["gilded-fresco".to_owned()]);
    let errors: Vec<(&str, &str)> = result
        .failed
        .iter()
        .map(|f| (f.path.as_str(), f.error.as_str()))
        .collect();
    let project = s(&fx.project);
    assert_eq!(
        errors,
        [
            (project.as_str(), "not a worktree of any known project"),
            ("/nowhere", "not a worktree of any known project"),
        ]
    );
    assert!(!Path::new(&merged).exists());
    assert!(crate::git(&fx.project, &["branch", "--list", "amber-muse"]).contains("amber-muse"));

    // The left branch now shows as stale.
    let inv = inventory(&fx, &[]);
    let stale: Vec<&str> = inv.projects[0]
        .stale_branches
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(stale, ["amber-muse"]);
}

#[test]
fn reclaiming_sweeps_build_output_and_keeps_the_work() {
    let fx = Fixture::new();
    let path = fx.wt("royal-chapel");
    let guard = Sessions::running_in(&[]);
    let git = local_git();
    let sizes = Sizes::default();
    let projects = fx.projects();
    let dirs = vec!["node_modules".to_owned()];
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &dirs,
        projects: &projects,
        guard: &guard,
    };
    let clean = fx.wt("gilded-fresco");
    std::fs::remove_dir_all(Path::new(&clean).join("node_modules")).unwrap();
    let result = reclaim_artifacts(&[path.clone(), clean.clone(), fx.wt("stray")], &ctx);
    assert_eq!(result.succeeded, vec![path.clone(), clean]);
    assert!(result.freed_bytes >= 8192);
    assert_eq!(
        result.failed[0].error,
        "not a worktree of any known project"
    );
    assert!(!Path::new(&path).join("node_modules").exists());
    assert_eq!(
        std::fs::read_to_string(Path::new(&path).join("a.txt")).unwrap(),
        "changed\n"
    );
}

#[test]
fn pruning_deletes_only_what_git_has_forgotten() {
    let fx = Fixture::new();
    let guard = Sessions::running_in(&[]);
    let git = local_git();
    let sizes = Sizes::default();
    let projects = fx.projects();
    let ctx = Cleanup {
        git: &git,
        sizes: &sizes,
        artifact_dirs: &[],
        projects: &projects,
        guard: &guard,
    };
    let stray = fx.wt("stray");
    let listed = fx.wt("amber-muse");
    let result = prune_orphan_dirs(&[stray.clone(), listed.clone(), s(&fx.project)], &ctx);
    assert_eq!(result.succeeded, vec![stray.clone()]);
    assert!(result.freed_bytes >= 4096);
    assert_eq!(
        result.failed[0].error,
        "still registered with git — remove it as a worktree instead"
    );
    assert_eq!(result.failed[1].path, s(&fx.project));
    assert!(!Path::new(&stray).exists());
    assert!(Path::new(&listed).exists());
}

/// Makes git forget a worktree while its directory stays.
fn git_worktree_forget(fx: &Fixture, name: &str) {
    let dir = fx.base.join(name);
    std::fs::remove_file(dir.join(".git")).unwrap();
    git(&fx.project, &["worktree", "prune"]);
}
