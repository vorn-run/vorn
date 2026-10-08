//! `git:renameWorktreeBranch`, `git:renameWorktree` and `git:checkoutBranch`, answered by vornd
//! once it holds the session records: the git is `vorn_git::repo`, and the
//! sessions in the worktree take the new branch, or path and name, in the
//! copy of the records ([`crate::registry::WorktreeMove`]), which tells the
//! server and so its clients.
//!
//! The server keeps both calls while it holds the records itself (vornd
//! neither creates its terminals nor starts its headless agents), and for a
//! worktree in a project on a remote host.

use std::path::Path;

use serde_json::{json, Value};
use vorn_git::repo::{Done, Git, MovedWorktree};

use super::{absolute_str, Answer, Native};
use crate::registry::WorktreeMove;

/// Whether `method` is one of the calls this module answers.
pub fn foresees(method: &str) -> bool {
    matches!(
        method,
        "git:renameWorktreeBranch" | "git:renameWorktree" | "git:checkoutBranch"
    )
}

/// A call as its handler reads it.
enum Asked<'a> {
    Branch { worktree: &'a str, branch: &'a str },
    Move { worktree: &'a str, name: &'a str },
    Checkout { worktree: &'a str, branch: &'a str },
}

impl<'a> Asked<'a> {
    fn read(native: &Native, method: &str, params: &'a Value) -> Option<Self> {
        let text = |key| params.get(key)?.as_str();
        if method == "git:checkoutBranch" {
            let worktree = absolute_str(params.get("cwd")?)?;
            let branch = text("branch")?;
            return native
                .local_path(worktree)
                .then_some(Asked::Checkout { worktree, branch });
        }
        let worktree = absolute_str(params.get("worktreePath")?)?;
        let asked = match method {
            "git:renameWorktreeBranch" => Asked::Branch {
                worktree,
                branch: text("newBranch")?,
            },
            "git:renameWorktree" => Asked::Move {
                worktree,
                name: text("newName")?,
            },
            _ => return None,
        };
        native.local_path(worktree).then_some(asked)
    }
}

fn git(native: &Native) -> Git {
    Git {
        bin: native.env.git_bin(),
        env: native.env.get(),
    }
}

fn moved_json(moved: Option<MovedWorktree>) -> Value {
    moved.map_or(
        Value::Null,
        |m| json!({ "newPath": m.path, "name": m.name }),
    )
}

/// Whether vornd holds the records the server would change: it decides
/// them, and the server follows vornd's terminals and headless agents.
fn holds_records(native: &Native) -> bool {
    let follows = native
        .link
        .get()
        .is_some_and(|l| l.creates_terminals() && l.creates_headless());
    follows && native.registry.get().is_some_and(|r| r.decides())
}

/// Answers `method`, renaming or moving the worktree and its sessions.
pub fn call(native: &Native, method: &str, params: &Value) -> Answer {
    if !holds_records(native) {
        return Answer::Forward;
    }
    let Some(asked) = Asked::read(native, method, params) else {
        return Answer::Forward;
    };
    let git = git(native);
    let (worktree, answer, moved) = match asked {
        Asked::Branch { worktree, branch } => {
            let done = native.turns.take(Path::new(worktree), || {
                git.rename_branch(Path::new(worktree), branch)
            });
            (
                worktree,
                json!(done),
                done.then(|| WorktreeMove::Branch(branch.to_owned())),
            )
        }
        Asked::Move { worktree, name } => {
            let done = native
                .turns
                .take(Path::new(worktree), || git.move_worktree(worktree, name));
            let moved = done.as_ref().map(|m| WorktreeMove::Path {
                path: m.path.clone(),
                name: m.name.clone(),
            });
            (worktree, moved_json(done), moved)
        }
        Asked::Checkout { worktree, branch } => {
            let done = native.turns.take(Path::new(worktree), || {
                git.checkout(Path::new(worktree), branch)
            });
            match done {
                Done::Ok => (
                    worktree,
                    json!({ "ok": true }),
                    Some(WorktreeMove::Branch(branch.to_owned())),
                ),
                Done::Failed(error) => (worktree, json!({ "ok": false, "error": error }), None),
            }
        }
    };
    if let (Some(moved), Some(registry)) = (moved, native.registry.get()) {
        registry.change(|r| ((), r.move_worktree(worktree, &moved).unwrap_or_default()));
    }
    Answer::Result(answer)
}

/// What vornd would answer `method`, worked out without changing the
/// repository, for the comparison with the server's answer in shadow mode.
/// `None` when the server's answer is its own: a remote worktree, params of
/// another shape, or a branch name vornd cannot judge without git.
pub fn foresee(native: &Native, method: &str, params: &Value) -> Option<Answer> {
    let git = git(native);
    let answer = match Asked::read(native, method, params)? {
        Asked::Branch { worktree, branch } => {
            json!(git.foresee_branch_rename(Path::new(worktree), branch)?)
        }
        Asked::Move { worktree, name } => moved_json(git.foresee_worktree_move(worktree, name)),
        // Whether git takes the checkout is git's to say.
        Asked::Checkout { .. } => return None,
    };
    Some(Answer::Result(answer))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;

    use super::*;
    use crate::applink::AppLink;
    use crate::registry::SessionRegistry;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A repository with a linked worktree on `feature`, and its path.
    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir(&main).unwrap();
        sh(&main, &["init", "-q", "-b", "main"]);
        sh(&main, &["commit", "-q", "--allow-empty", "-m", "one"]);
        let wt = dir.path().join("old-1a2b3c4d");
        sh(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        (dir, wt.to_string_lossy().into_owned())
    }

    /// vornd holding the records of a terminal and a headless agent in
    /// `worktree`, with the server following both when `follows`.
    fn holding(worktree: &str, follows: bool) -> (Arc<Native>, Arc<SessionRegistry>) {
        let native = Native::new();
        let registry = SessionRegistry::new();
        native.set_registry(Arc::clone(&registry));
        registry.decide_statuses();
        let snapshot = json!({
            "op": "snapshot", "order": ["t"],
            "terminals": [{
                "id": "t", "agentType": "claude", "projectName": "p", "projectPath": "/p",
                "status": "running", "createdAt": 1, "pid": 9,
                "worktreePath": worktree, "worktreeName": "old", "branch": "feature",
            }],
            "headless": [{
                "id": "h", "pid": 7, "agentType": "claude", "projectName": "p",
                "projectPath": "/p", "status": "running", "startedAt": 1,
                "worktreePath": worktree,
            }],
        });
        registry.feed(1, &snapshot).unwrap();
        let link = Arc::new(AppLink::default());
        if follows {
            link.set_creates_terminals();
            link.set_creates_headless();
        }
        native.set_link(link);
        native.set_database(
            std::env::temp_dir()
                .join("vornd-no-such-db")
                .join("vorn.db"),
        );
        (native, registry)
    }

    type Fields = (Option<String>, Option<String>, Option<String>);

    fn terminal(registry: &SessionRegistry) -> Fields {
        registry
            .read(|r| {
                let t = r.terminals()[0];
                (
                    t.branch.clone(),
                    t.worktree_path.clone(),
                    t.worktree_name.clone(),
                )
            })
            .unwrap()
    }

    /// The branch rename foreseen to succeed, unless gix declines to judge
    /// it here (the environment carries git config it does not read).
    fn foreseen_true(native: &Native, params: &Value) -> bool {
        matches!(
            foresee(native, "git:renameWorktreeBranch", params),
            None | Some(Answer::Result(Value::Bool(true)))
        )
    }

    #[test]
    fn renames_the_branch_and_moves_the_worktree_with_its_sessions() {
        let (_dir, wt) = repo();
        let (native, registry) = holding(&wt, true);
        let rename = json!({ "worktreePath": wt, "newBranch": " renamed " });
        assert!(foreseen_true(&native, &rename));
        assert_eq!(
            call(&native, "git:renameWorktreeBranch", &rename),
            Answer::Result(json!(true))
        );
        // As the server keeps it: the name as the client sent it.
        assert_eq!(terminal(&registry).0.as_deref(), Some(" renamed "));
        let taken = json!({ "worktreePath": wt, "newBranch": "main" });
        assert_eq!(
            call(&native, "git:renameWorktreeBranch", &taken),
            Answer::Result(json!(false))
        );
        assert_eq!(terminal(&registry).0.as_deref(), Some(" renamed "));

        let moved = json!({ "worktreePath": wt, "newName": "New Name" });
        let target = Path::new(&wt).with_file_name("New-Name-1a2b3c4d");
        let expected = json!({ "newPath": target.to_str().unwrap(), "name": "New-Name" });
        assert_eq!(
            foresee(&native, "git:renameWorktree", &moved),
            Some(Answer::Result(expected.clone()))
        );
        assert_eq!(
            call(&native, "git:renameWorktree", &moved),
            Answer::Result(expected)
        );
        assert!(target.is_dir());
        let path = target.to_string_lossy().into_owned();
        assert_eq!(
            terminal(&registry),
            (
                Some(" renamed ".into()),
                Some(path.clone()),
                Some("New-Name".into())
            )
        );
        let headless = registry
            .read(|r| r.headless_record("h").unwrap().worktree_path.clone())
            .unwrap();
        assert_eq!(headless, Some(path));
        // The worktree is no longer where the client says.
        assert_eq!(
            call(&native, "git:renameWorktree", &moved),
            Answer::Result(Value::Null)
        );
    }

    #[test]
    fn checks_out_a_branch_and_moves_the_sessions_on_it() {
        let (dir, wt) = repo();
        sh(&dir.path().join("main"), &["branch", "other"]);
        let (native, registry) = holding(&wt, true);
        let checkout = json!({ "cwd": wt, "branch": "other" });
        assert_eq!(foresee(&native, "git:checkoutBranch", &checkout), None);
        assert_eq!(
            call(&native, "git:checkoutBranch", &checkout),
            Answer::Result(json!({ "ok": true }))
        );
        assert_eq!(terminal(&registry).0.as_deref(), Some("other"));
        let missing = json!({ "cwd": wt, "branch": "no-such-branch" });
        let Answer::Result(refused) = call(&native, "git:checkoutBranch", &missing) else {
            panic!("an answer")
        };
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().is_some_and(|e| !e.is_empty()));
        assert_eq!(terminal(&registry).0.as_deref(), Some("other"));
    }

    #[test]
    fn leaves_the_call_to_the_server_while_it_holds_the_records() {
        let (_dir, wt) = repo();
        let (native, registry) = holding(&wt, false);
        let rename = json!({ "worktreePath": wt, "newBranch": "renamed" });
        assert_eq!(
            call(&native, "git:renameWorktreeBranch", &rename),
            Answer::Forward
        );
        assert_eq!(terminal(&registry).0.as_deref(), Some("feature"));
        // Foreseen all the same: the server answers what vornd would.
        assert!(foreseen_true(&native, &rename));
    }

    #[test]
    fn leaves_params_of_another_shape_to_the_server() {
        let (_dir, wt) = repo();
        let (native, _registry) = holding(&wt, true);
        for (method, params) in [
            ("git:renameWorktreeBranch", json!({ "worktreePath": wt })),
            (
                "git:renameWorktreeBranch",
                json!({ "worktreePath": "rel", "newBranch": "x" }),
            ),
            (
                "git:renameWorktree",
                json!({ "worktreePath": wt, "newName": 3 }),
            ),
            ("git:renameWorktree", json!(wt)),
        ] {
            assert_eq!(call(&native, method, &params), Answer::Forward, "{params}");
            assert_eq!(foresee(&native, method, &params), None, "{params}");
        }
        // Without the database vornd cannot tell a remote worktree.
        let bare = Native::new();
        let params = json!({ "worktreePath": wt, "newName": "x" });
        assert_eq!(foresee(&bare, "git:renameWorktree", &params), None);
    }
}
