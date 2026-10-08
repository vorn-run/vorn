//! What a raw delete may touch.
//!
//! `git worktree remove` only touches a path git claims as a worktree, and
//! keeps uncommitted work unless forced, so a removal needs no more than git's
//! word. A delete of a directory has no such backstop: its target must sit
//! inside a worktree git named, or, for a directory git has forgotten, inside
//! `.vorn-worktrees/<project>/`. Symlinks are resolved first, so a link planted
//! in a worktree cannot reach the rest of the file system.

use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

use crate::WORKTREE_ROOT_SEGMENT;

/// `path.resolve`: absolute, `.` and `..` resolved, without the file system.
pub(crate) fn resolve(p: &str) -> PathBuf {
    let absolute = std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p));
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The path with symlinks resolved, or only resolved when it does not exist,
/// so two spellings of one directory compare equal.
pub(crate) fn canonical(p: &str) -> String {
    let resolved = resolve(p);
    std::fs::canonicalize(&resolved)
        .unwrap_or(resolved)
        .to_string_lossy()
        .into_owned()
}

/// [`canonical`] for a remote host's path: only trailing slashes go.
pub(crate) fn canonical_remote(p: &str) -> String {
    p.trim_end_matches('/').to_owned()
}

/// [`assert_removable_path`] for a remote host's POSIX path, taken as written.
pub fn assert_removable_remote_path(target: &str) -> Result<(), String> {
    refuse_empty(target)?;
    let segments: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    check_segments(target, &segments)
}

/// [`assert_inside_worktree`] for a remote host's POSIX paths.
pub fn assert_inside_remote_worktree(target: &str, worktree: &str) -> Result<(), String> {
    refuse_empty(target)?;
    let root = canonical_remote(worktree);
    let resolved = canonical_remote(target);
    let beneath = resolved
        .strip_prefix(&root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
    if !beneath {
        return Err(format!(
            "Refusing to delete {target}: outside the worktree at {worktree}"
        ));
    }
    Ok(())
}

fn refuse_empty(target: &str) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("Refusing to delete an empty path".into());
    }
    Ok(())
}

/// Refuses anything that is not a directory inside
/// `.vorn-worktrees/<project>/`: never the root itself, nor a whole project's
/// worktrees at once. For directories git has forgotten, which have no record
/// to check against.
pub fn assert_removable_path(target: &str) -> Result<(), String> {
    refuse_empty(target)?;
    let resolved = canonical(target);
    let segments: Vec<&str> = resolved
        .split(MAIN_SEPARATOR)
        .filter(|s| !s.is_empty())
        .collect();
    check_segments(target, &segments)
}

/// At least `.vorn-worktrees/<project>/<worktree>`, never the root or a whole project's folder.
fn check_segments(target: &str, segments: &[&str]) -> Result<(), String> {
    let Some(root) = segments.iter().rposition(|s| *s == WORKTREE_ROOT_SEGMENT) else {
        return Err(format!(
            "Refusing to delete a path outside {WORKTREE_ROOT_SEGMENT}: {target}"
        ));
    };
    if segments.len() < root + 3 {
        return Err(format!(
            "Refusing to delete {target}: not a worktree directory"
        ));
    }
    Ok(())
}

/// Refuses a target that is not the worktree itself or beneath it.
pub fn assert_inside_worktree(target: &str, worktree: &str) -> Result<(), String> {
    refuse_empty(target)?;
    let root = canonical(worktree);
    let resolved = canonical(target);
    let beneath = resolved
        .strip_prefix(&root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(MAIN_SEPARATOR));
    if !beneath {
        return Err(format!(
            "Refusing to delete {target}: outside the worktree at {worktree}"
        ));
    }
    Ok(())
}

/// Whether `p` names an existing directory, not a link to one.
pub(crate) fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn resolves_dots_without_the_file_system() {
        assert_eq!(resolve("/a/./b/../c/"), PathBuf::from("/a/c"));
        assert_eq!(resolve("/.."), PathBuf::from("/"));
    }

    #[test]
    fn removes_only_a_worktree_directory_under_the_root() {
        let ok = "/nowhere/.vorn-worktrees/proj/wt";
        assert_eq!(assert_removable_path(ok), Ok(()));
        assert_eq!(
            assert_removable_path("  "),
            Err("Refusing to delete an empty path".into())
        );
        assert_eq!(
            assert_removable_path("/nowhere/proj"),
            Err("Refusing to delete a path outside .vorn-worktrees: /nowhere/proj".into())
        );
        for whole in ["/nowhere/.vorn-worktrees", "/nowhere/.vorn-worktrees/proj"] {
            assert_eq!(
                assert_removable_path(whole),
                Err(format!(
                    "Refusing to delete {whole}: not a worktree directory"
                ))
            );
        }
        // `..` is resolved before the check, not trusted.
        assert!(assert_removable_path("/nowhere/.vorn-worktrees/proj/wt/../..").is_err());
    }

    #[test]
    fn a_link_out_of_the_worktree_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let wt = tmp.path().join("wt");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(wt.join("dist")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, wt.join("out")).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let wt_s = s(&wt);
        assert_eq!(assert_inside_worktree(&s(&wt.join("dist")), &wt_s), Ok(()));
        assert_eq!(assert_inside_worktree(&wt_s, &wt_s), Ok(()));
        let link = s(&wt.join("out"));
        assert_eq!(
            assert_inside_worktree(&link, &wt_s),
            Err(format!(
                "Refusing to delete {link}: outside the worktree at {wt_s}"
            ))
        );
        // A sibling sharing the prefix is not beneath it.
        let sibling = format!("{wt_s}x");
        assert!(assert_inside_worktree(&sibling, &wt_s).is_err());
        assert!(assert_inside_worktree("", &wt_s).is_err());
        assert!(is_real_dir(&wt));
        assert!(!is_real_dir(&wt.join("out")));
    }
}
