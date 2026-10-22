//! Files written whole: the bytes go to a temp file beside the target, are
//! synced, and the temp file is renamed over (or linked to) the target, so a
//! reader (the workflow folder's watcher, another client) sees the old file,
//! the new one or none, never an empty or partial one.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What every temp file's name ends with, so a watcher can pass over them.
const TEMP_SUFFIX: &str = ".vorn-tmp";

/// Whether `path` is one of the temp files written here, half-done or left
/// by a crash: never a file of its own.
pub fn is_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') && n.ends_with(TEMP_SUFFIX))
}

/// Replaces `path` with `bytes`, making its folder if needed. A link is
/// written through, so a workflow file linked in from elsewhere stays linked.
pub fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path)?,
        _ => path.to_owned(),
    };
    let dir = folder(&target)?;
    let (tmp, file) = temp(&dir, &target)?;
    let done = fill(file, bytes)
        .and_then(|()| {
            // A rename takes the temp file's mode; keep the one the file had.
            match std::fs::metadata(&target) {
                Ok(m) => std::fs::set_permissions(&tmp, m.permissions()),
                Err(_) => Ok(()),
            }
        })
        .and_then(|()| std::fs::rename(&tmp, &target));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

/// Writes `bytes` to `path` only when nothing, not even a link, is there:
/// [`io::ErrorKind::AlreadyExists`] when something is.
pub fn create(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = folder(path)?;
    let (tmp, file) = temp(&dir, path)?;
    // A hard link never replaces what is there, as a rename would.
    let done = fill(file, bytes).and_then(|()| std::fs::hard_link(&tmp, path));
    let _ = std::fs::remove_file(&tmp);
    done
}

fn folder(path: &Path) -> io::Result<PathBuf> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_owned(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A fresh temp file in `dir` for `target`; unique in this process and
/// across processes, so two writers never share one.
fn temp(dir: &Path, target: &Path) -> io::Result<(PathBuf, File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.{}-{}{TEMP_SUFFIX}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    Ok((tmp, file))
}

fn fill(mut file: File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_reader_sees_no_file_or_a_whole_one_never_a_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.toml");
        let wholes: [Vec<u8>; 2] = [vec![b'a'; 256 * 1024], vec![b'b'; 64 * 1024 + 7]];
        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let (path, stop, wholes) = (path.clone(), Arc::clone(&stop), wholes.clone());
            std::thread::spawn(move || {
                let mut seen = 0;
                while !stop.load(Ordering::Relaxed) {
                    match std::fs::read(&path) {
                        Ok(b) => {
                            assert!(wholes.contains(&b), "read a part: {} bytes", b.len());
                            seen += 1;
                        }
                        Err(e) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
                    }
                }
                seen
            })
        };
        for i in 0..200 {
            replace(&path, &wholes[i % 2]).unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        assert_eq!(std::fs::read(&path).unwrap(), wholes[1]);
        assert_eq!(entries(dir.path()), ["w.toml"]);
    }

    #[test]
    fn create_never_replaces_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("w.toml");
        create(&path, b"first").unwrap();
        let err = create(&path, b"second").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(entries(&dir.path().join("sub")), ["w.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn create_takes_a_dangling_link_as_taken_and_replace_writes_through_one() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("w.toml");
        let real = dir.path().join("real.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(
            create(&link, b"x").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!real.exists());
        std::fs::write(&real, b"old").unwrap();
        replace(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn replace_keeps_the_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.toml");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        replace(&path, b"new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[test]
    fn a_temp_file_is_known_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let (tmp, _file) = temp(dir.path(), &dir.path().join("w.toml")).unwrap();
        assert!(is_temp(&tmp));
        assert!(!is_temp(&dir.path().join("w.toml")));
        assert!(!is_temp(&dir.path().join(".hidden.toml")));
    }
}
