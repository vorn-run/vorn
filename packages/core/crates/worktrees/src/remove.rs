//! The cleanup actions: remove worktrees, sweep their build output, delete
//! directories git has forgotten. Each goes on past a failure and reports
//! every path on one side or the other, in the server's words.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use vorn_git::repo::{Git, WorktreeEntry};

use crate::guard::{
    assert_inside_remote_worktree, assert_inside_worktree, assert_removable_path,
    assert_removable_remote_path, canonical, canonical_remote,
};
use crate::scan::{git_on, Project, Remote};
use crate::size::{
    du_bytes, du_bytes_remote, find_artifact_dirs, find_artifact_dirs_remote, walk_bytes, Sizes,
};

/// What the host promises around a deletion.
pub trait Guard {
    /// An error, in the words the person sees, when a session runs in `path`
    /// or is starting there.
    fn assert_idle(&self, path: &str) -> Result<(), String>;

    /// Runs `f` with the turn of `project`'s repository, so no other change
    /// to it comes between `f`'s check and its removal.
    fn turn(&self, project: &Path, f: &mut dyn FnMut() -> Result<(), String>)
        -> Result<(), String>;
}

/// A path that could not be cleaned, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

/// What an action did (`WorktreeActionResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<Failure>,
    pub freed_bytes: u64,
    /// Only the branches that are really gone afterwards.
    pub deleted_branches: Vec<String>,
}

impl ActionResult {
    fn settle(&mut self, path: &str, done: Result<u64, String>) {
        match done {
            Ok(freed) => {
                self.freed_bytes += freed;
                self.succeeded.push(path.to_owned());
            }
            Err(error) => self.failed.push(Failure {
                path: path.to_owned(),
                error,
            }),
        }
    }
}

/// One worktree to remove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveItem {
    pub worktree_path: String,
    pub force: bool,
    pub delete_branch: bool,
}

/// What the actions work with.
pub struct Cleanup<'a> {
    pub git: &'a Git,
    pub sizes: &'a Sizes,
    pub artifact_dirs: &'a [String],
    pub projects: &'a [Project],
    pub guard: &'a dyn Guard,
    /// The remote host any path is on (`resolveRemoteHostByPath`), for an orphan directory.
    pub remote_of: &'a dyn Fn(&str) -> Option<Remote>,
}

/// Where an owned worktree is: its project, its path, and the host it is on.
struct Owner<'a> {
    project: String,
    worktree: String,
    remote: Option<&'a Remote>,
}

/// A project's worktrees, listed once per action.
#[derive(Default)]
struct Listed(HashMap<String, Vec<WorktreeEntry>>);

/// The project and worktree git says `target` is, by resolved path, so a
/// worktree made by hand outside `.vorn-worktrees` counts as much as one vorn
/// made. Never the project's own checkout.
fn find_owning_worktree<'a>(
    target: &str,
    ctx: &Cleanup<'a>,
    listed: &mut Listed,
) -> Result<Owner<'a>, String> {
    for project in ctx.projects {
        let remote = project.remote.as_ref();
        let canon = |p: &str| match remote {
            Some(_) => canonical_remote(p),
            None => canonical(p),
        };
        let wanted = canon(target);
        let worktrees = listed
            .0
            .entry(project.path.clone())
            .or_insert_with(|| git_on(ctx.git, remote).list_worktrees(Path::new(&project.path)));
        if let Some(wt) = worktrees
            .iter()
            .find(|wt| !wt.is_main && canon(&wt.path) == wanted)
        {
            return Ok(Owner {
                project: project.path.clone(),
                worktree: wt.path.clone(),
                remote,
            });
        }
    }
    Err("not a worktree of any known project".into())
}

/// Removes registered worktrees, and their branches when asked. `git worktree
/// remove` is the guard: it refuses what it does not own, and uncommitted
/// work unless forced.
pub fn remove_worktrees(items: &[RemoveItem], ctx: &Cleanup<'_>) -> ActionResult {
    let mut result = ActionResult::default();
    let mut listed = Listed::default();
    for item in items {
        let path = item.worktree_path.as_str();
        let mut branch = None;
        let done = (|| {
            let owner = find_owning_worktree(path, ctx, &mut listed)?;
            let (project, remote) = (owner.project, owner.remote);
            let git = git_on(ctx.git, remote);
            let bytes = size_of(path, ctx, remote);
            if item.delete_branch {
                branch = git.branch(Path::new(path));
            }
            // Again, after the git above, and once more inside the turn.
            ctx.guard.assert_idle(path)?;
            ctx.guard.turn(Path::new(&project), &mut || {
                ctx.guard.assert_idle(path)?;
                if git.remove_worktree(Path::new(&project), path, item.force, item.delete_branch) {
                    Ok(())
                } else {
                    Err("git worktree remove failed".into())
                }
            })?;
            ctx.sizes.invalidate(path);
            Ok((project, bytes, git))
        })();
        let freed = done.map(|(project, bytes, git)| {
            if let Some(b) = branch.take() {
                if !git.list_branches(Path::new(&project)).contains(&b) {
                    result.deleted_branches.push(b);
                }
            }
            bytes
        });
        result.settle(path, freed);
    }
    result
}

/// Deletes the build output inside worktrees, leaving git alone: the one
/// action that cannot lose work.
pub fn reclaim_artifacts(paths: &[String], ctx: &Cleanup<'_>) -> ActionResult {
    let mut result = ActionResult::default();
    let mut listed = Listed::default();
    for path in paths {
        let done = (|| {
            let owner = find_owning_worktree(path, ctx, &mut listed)?;
            let (worktree, remote) = (owner.worktree, owner.remote);
            let dirs = match remote {
                Some(r) => find_artifact_dirs_remote(path, ctx.artifact_dirs, r),
                None => find_artifact_dirs(path, ctx.artifact_dirs, &ctx.git.env),
            };
            if dirs.is_empty() {
                return Ok(0);
            }
            let before = if let Some(r) = remote {
                du_bytes_remote(&dirs, r).ok_or("could not measure the build output")?
            } else if cfg!(unix) {
                du_bytes(&dirs, &ctx.git.env).ok_or("could not measure the build output")?
            } else {
                dirs.iter()
                    .map(|d| walk_bytes(Path::new(d), &Default::default()))
                    .sum()
            };
            ctx.guard.assert_idle(path)?;
            for dir in &dirs {
                // Each on its own: a symlinked build directory must not lead
                // out of the worktree.
                match remote {
                    Some(_) => assert_inside_remote_worktree(dir, &worktree)?,
                    None => assert_inside_worktree(dir, &worktree)?,
                }
                remove_dir(dir, remote)?;
            }
            ctx.sizes.invalidate(path);
            Ok(before)
        })();
        result.settle(path, done);
    }
    result
}

/// Deletes directories git has forgotten, which `git worktree remove` cannot
/// reach. Never one git still lists.
pub fn prune_orphan_dirs(paths: &[String], ctx: &Cleanup<'_>) -> ActionResult {
    let mut result = ActionResult::default();
    for path in paths {
        let done = (|| {
            let remote = (ctx.remote_of)(path);
            let remote = remote.as_ref();
            match remote {
                Some(_) => assert_removable_remote_path(path)?,
                None => assert_removable_path(path)?,
            }
            if git_on(ctx.git, remote)
                .absolute_git_dir(Path::new(path))
                .is_some()
            {
                return Err("still registered with git — remove it as a worktree instead".into());
            }
            let bytes = size_of(path, ctx, remote);
            ctx.guard.assert_idle(path)?;
            remove_dir(path, remote)?;
            ctx.sizes.invalidate(path);
            Ok(bytes)
        })();
        result.settle(path, done);
    }
    result
}

fn size_of(path: &str, ctx: &Cleanup<'_>, remote: Option<&Remote>) -> u64 {
    ctx.sizes
        .measure(path, ctx.artifact_dirs, false, &ctx.git.env, remote)
        .size_bytes
}

/// `fs.rmSync(.., {recursive, force})`: gone already is done; `rm -rf` on a remote host.
fn remove_dir(path: &str, remote: Option<&Remote>) -> Result<(), String> {
    if let Some(r) = remote {
        let cmd = format!("rm -rf {}", vorn_remote::quote(path));
        return r
            .shell(&cmd, std::time::Duration::from_secs(60))
            .map(|_| ());
    }
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}
