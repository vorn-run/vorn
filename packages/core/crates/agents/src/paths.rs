//! Paths compared the way the server compares them (`normalizePath`): an
//! agent records the directory a session ran in in its own spelling, and a
//! project is matched to it only after both are put in one form.

/// Whether `p` is spelled as a Windows path: a drive letter or a UNC share.
fn windows_style(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
        || p.starts_with("\\\\")
}

/// `normalizePath`: `.` and `..` resolved, repeated and trailing separators
/// dropped, symlinks resolved when the path exists, and lowercased when it
/// is a Windows path or this is Windows.
pub fn normalize(p: &str) -> String {
    let win = windows_style(p);
    let mut result = normalize_lexically(p);
    if let Ok(real) = std::fs::canonicalize(&result) {
        if let Some(real) = real.to_str() {
            result = without_verbatim(real);
        }
    }
    if win || windows_style(&result) || cfg!(windows) {
        result = result.to_lowercase();
    }
    result
}

/// `path.resolve` of an absolute path: `.` and `..` resolved and trailing
/// separators dropped, without asking the file system.
pub fn normalize_lexically(p: &str) -> String {
    let (root, mut result) = if windows_style(p) {
        normalize_win32(p)
    } else {
        normalize_posix(p)
    };
    if result != root {
        let kept = result[root.len()..].trim_end_matches(['/', '\\']).len() + root.len();
        result.truncate(kept);
    }
    result
}

/// One form for comparing a project path with one an agent recorded
/// (`comparablePath`): normalized, forward slashes, lowercase.
pub fn comparable(p: &str) -> String {
    normalize(p).replace('\\', "/").to_lowercase()
}

/// `path.posix.normalize`, and its root (`/` or nothing).
fn normalize_posix(p: &str) -> (String, String) {
    if p.is_empty() {
        return (String::new(), ".".to_owned());
    }
    let absolute = p.starts_with('/');
    let trailing = p.ends_with('/');
    let body = resolve_segments(p.split('/'), absolute, "/");
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    out.push_str(&body);
    if out.is_empty() {
        out.push('.');
    }
    if trailing && !body.is_empty() {
        out.push('/');
    }
    let root = if absolute { "/" } else { "" };
    (root.to_owned(), out)
}

/// `path.win32.normalize`, close enough for comparing paths: separators
/// made backslashes, the drive or share kept as the root, and the rest
/// resolved as on POSIX.
fn normalize_win32(p: &str) -> (String, String) {
    let p = p.replace('/', "\\");
    let (root, rest) = if let Some(unc) = p.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        (format!("\\\\{server}\\{share}\\"), rest.to_owned())
    } else {
        let (drive, rest) = p.split_at(2);
        (
            format!("{drive}\\"),
            rest.trim_start_matches('\\').to_owned(),
        )
    };
    let trailing = rest.ends_with('\\');
    let body = resolve_segments(rest.split('\\'), true, "\\");
    let mut out = root.clone();
    out.push_str(&body);
    if trailing && !body.is_empty() {
        out.push('\\');
    }
    (root, out)
}

/// The segments with `.` and empty ones dropped and `..` taking the one
/// before it; above the root of an absolute path there is nothing to take.
fn resolve_segments<'a>(
    segments: impl Iterator<Item = &'a str>,
    absolute: bool,
    sep: &str,
) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for seg in segments {
        match seg {
            "" | "." => {}
            ".." => match kept.last() {
                Some(&last) if last != ".." => {
                    kept.pop();
                }
                _ if absolute => {}
                _ => kept.push(".."),
            },
            _ => kept.push(seg),
        }
    }
    kept.join(sep)
}

/// A canonical Windows path without the `\\?\` prefix `realpathSync` never
/// shows.
fn without_verbatim(p: &str) -> String {
    if let Some(unc) = p.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{unc}")
    } else if let Some(rest) = p.strip_prefix("\\\\?\\") {
        rest.to_owned()
    } else {
        p.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_dots_and_separators_as_path_normalize_does() {
        assert_eq!(normalize("/no/such/./dir//x/../y/"), "/no/such/dir/y");
        assert_eq!(normalize("/../no-such"), "/no-such");
        assert_eq!(normalize("rel/../../up"), "../up");
        // `/` exists, so it resolves; on Windows to the current drive.
        if cfg!(windows) {
            assert!(windows_style(&normalize("/")));
        } else {
            assert_eq!(normalize("/"), "/");
        }
        // Lexically: `.` exists, and resolves to the working directory.
        assert_eq!(normalize_lexically(""), ".");
        assert_eq!(normalize_lexically("./"), ".");
    }

    #[test]
    fn lowercases_windows_paths_wherever_it_runs() {
        assert_eq!(normalize("C:\\Users\\Me\\Proj\\"), "c:\\users\\me\\proj");
        assert_eq!(normalize("D:/Work/../Repo"), "d:\\repo");
        assert_eq!(
            normalize("\\\\Server\\Share\\A\\..\\B"),
            "\\\\server\\share\\b"
        );
        assert_eq!(comparable("C:\\Users\\Me"), "c:/users/me");
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlinks_of_paths_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canonical = std::fs::canonicalize(&real).unwrap();
        assert_eq!(
            normalize(&format!("{}/", link.display())),
            canonical.to_str().unwrap()
        );
    }
}
