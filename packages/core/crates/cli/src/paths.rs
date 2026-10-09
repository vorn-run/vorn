//! Paths as the TypeScript command handles them: `path.resolve`,
//! `path.basename`, and the server's `normalizePath` for telling whether two
//! paths name one project.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// `path.resolve(p)`: absolute against the working directory, `.` and `..`
/// folded away lexically, no trailing separator.
pub fn resolve(p: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let joined = cwd.join(p);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// `path.basename(p)`: the last component, trailing separators ignored.
pub fn basename(p: &str) -> &str {
    let trimmed = p.trim_end_matches(|c| c == '/' || (cfg!(windows) && c == '\\'));
    trimmed
        .rsplit(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .next()
        .unwrap_or("")
}

fn windows_style(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
        || p.starts_with("\\\\")
}

/// A path for comparison: separators trimmed, symlinks resolved when it
/// exists, and folded to lower case where the file system ignores case.
pub fn normalize(p: &str) -> String {
    let mut result = p.to_owned();
    while result.len() > 1 && (result.ends_with('/') || result.ends_with('\\')) {
        result.pop();
    }
    if let Ok(real) = std::fs::canonicalize(&result) {
        let real = real.to_string_lossy().into_owned();
        // Windows answers with a verbatim prefix nobody wrote.
        result = real
            .strip_prefix(r"\\?\")
            .map(str::to_owned)
            .unwrap_or(real);
    }
    if windows_style(&result) || windows_style(p) || cfg!(windows) {
        result = result.to_lowercase();
    }
    result
}

/// The top of the git repository `cwd` is in, as `git rev-parse
/// --show-toplevel` says it; `None` outside one, or when git does not answer
/// within three seconds.
pub async fn repo_root(cwd: &Path) -> Option<String> {
    let run = vorn_spawn::tokio_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!root.is_empty()).then_some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_last_component() {
        assert_eq!(basename("/a/b/website/"), "website");
        assert_eq!(basename("website"), "website");
    }

    #[cfg(unix)]
    #[test]
    fn resolves_lexically() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(resolve("/a/./b/../c/"), "/a/c");
        assert_eq!(resolve("x"), cwd.join("x").to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn normalizes_for_comparison() {
        assert_eq!(normalize("/no/such/dir///"), "/no/such/dir");
        assert_eq!(normalize("C:\\Work\\"), "c:\\work");
    }
}
