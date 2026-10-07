//! The cleanup actions: remove worktrees, sweep their build output, delete
//! directories git has forgotten. Each goes on past a failure and reports
//! every path on one side or the other, in the server's words.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use vorn_git::repo::{Git, WorktreeEntry};

use crate::guard::{assert_inside_worktree, assert_removable_path, canonical};
use crate::scan::Project;
use crate::size::{du_bytes, find_artifact_dirs, walk_bytes, Sizes};

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
}

/// A project's worktrees, listed once per action.
#[derive(Default)]
struct Listed(HashMap<String, Vec<WorktreeEntry>>);

/// The project and worktree git says `target` is, by resolved path, so a
/// worktree made by hand outside `.vorn-worktrees` counts as much as one vorn
/// made. Never the project's own checkout.
fn find_owning_worktree(
    target: &str,
    ctx: &Cleanup<'_>,
    listed: &mut Listed,
) -> Result<(String, String), String> {
    let wanted = canonical(target);
    for project in ctx.projects {
        let worktrees = listed
            .0
            .entry(project.path.clone())
            .or_insert_with(|| ctx.git.list_worktrees(Path::new(&project.path)));
        if let Some(wt) = worktrees
            .iter()
            .find(|wt| !wt.is_main && canonical(&wt.path) == wanted)
        {
            return Ok((project.path.clone(), wt.path.clone()));
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
            let (project, _) = find_owning_worktree(path, ctx, &mut listed)?;
            let bytes = size_of(path, ctx);
            if item.delete_branch {
                branch = ctx.git.branch(Path::new(path));
            }
            // Again, after the git above, and once more inside the turn.
            ctx.guard.assert_idle(path)?;
            ctx.guard.turn(Path::new(&project), &mut || {
                ctx.guard.assert_idle(path)?;
                if ctx.git.remove_worktree(
                    Path::new(&project),
                    path,
                    item.force,
                    item.delete_branch,
                ) {
                    Ok(())
                } else {
                    Err("git worktree remove failed".into())
                }
            })?;
            ctx.sizes.invalidate(path);
            Ok((project, bytes))
        })();
        let freed = done.map(|(project, bytes)| {
            if let Some(b) = branch.take() {
                if !ctx.git.list_branches(Path::new(&project)).contains(&b) {
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
            let (_, worktree) = find_owning_worktree(path, ctx, &mut listed)?;
            let dirs = find_artifact_dirs(path, ctx.artifact_dirs, &ctx.git.env);
            if dirs.is_empty() {
                return Ok(0);
            }
            let before = if cfg!(unix) {
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
                assert_inside_worktree(dir, &worktree)?;
                remove_dir(dir)?;
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
            assert_removable_path(path)?;
            if ctx.git.absolute_git_dir(Path::new(path)).is_some() {
                return Err("still registered with git — remove it as a worktree instead".into());
            }
            let bytes = size_of(path, ctx);
            ctx.guard.assert_idle(path)?;
            remove_dir(path)?;
            ctx.sizes.invalidate(path);
            Ok(bytes)
        })();
        result.settle(path, done);
    }
    result
}

fn size_of(path: &str, ctx: &Cleanup<'_>) -> u64 {
    ctx.sizes
        .measure(path, ctx.artifact_dirs, false, &ctx.git.env)
        .size_bytes
}

/// `fs.rmSync(.., {recursive, force})`: gone already is done.
fn remove_dir(path: &str) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}
