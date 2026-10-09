//! Size-capped log files for vornd and vorn-sessiond.
//!
//! A log that only ever appends fills the disk of a machine that stays up
//! for weeks. Each process opens its log through this crate instead: once
//! the file reaches [`Rotation::limit`] it becomes `<name>.1`, the older
//! ones move up a number, and the one past [`Rotation::files`] is deleted.
//! Old files stay plain text so they can be read and searched as they are.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// When a log rotates and how much of it is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation {
    /// The size in bytes at which the current file is set aside.
    pub limit: u64,
    /// How many files are kept, the current one included.
    pub files: usize,
}

impl Rotation {
    /// 20 MiB a file and five files: at most about 100 MiB per log.
    pub const DEFAULT: Rotation = Rotation {
        limit: 20 << 20,
        files: 5,
    };
}

impl Default for Rotation {
    fn default() -> Self {
        Rotation::DEFAULT
    }
}

/// `<path>.<n>`: the `n`th older file of the log at `path`.
pub fn numbered(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// Sets `path` aside as `<path>.1`, moving older ones up and deleting the one past `files`.
pub fn rotate(path: &Path, files: usize) -> io::Result<()> {
    if files <= 1 {
        return ignore_missing(fs::remove_file(path));
    }
    ignore_missing(fs::remove_file(numbered(path, files - 1)))?;
    for n in (1..files - 1).rev() {
        ignore_missing(fs::rename(numbered(path, n), numbered(path, n + 1)))?;
    }
    ignore_missing(fs::rename(path, numbered(path, 1)))
}

/// Rotates the log at `path` if it has reached the limit; answers whether it did.
pub fn rotate_if_full(path: &Path, rotation: Rotation) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(m) if m.len() >= rotation.limit => rotate(path, rotation.files).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn ignore_missing(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

fn open_append(path: &Path) -> io::Result<(File, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let len = file.metadata()?.len();
    Ok((file, len))
}

/// A log file that rotates itself as it is written. A write that would pass
/// the limit starts a fresh file, so a line written at once is never split.
#[derive(Debug)]
pub struct LogFile {
    path: PathBuf,
    rotation: Rotation,
    /// `None` after a rotation could not reopen the file; the next write retries.
    file: Option<File>,
    len: u64,
}

impl LogFile {
    /// Opens the log at `path` for appending, rotating it first when it is already full.
    pub fn open(path: impl Into<PathBuf>, rotation: Rotation) -> io::Result<LogFile> {
        let path = path.into();
        // A log that cannot be set aside is still appended to.
        let _ = rotate_if_full(&path, rotation);
        let (file, len) = open_append(&path)?;
        Ok(LogFile {
            path,
            rotation,
            file: Some(file),
            len,
        })
    }

    /// Sets the current file aside and starts a new one; a failed rename keeps appending.
    fn roll(&mut self) {
        // Closed first: Windows cannot rename a file this process holds open.
        self.file = None;
        let _ = rotate(&self.path, self.rotation.files);
        self.reopen();
    }

    fn reopen(&mut self) {
        if let Ok((file, len)) = open_append(&self.path) {
            self.file = Some(file);
            self.len = len;
        }
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.len > 0 && self.len.saturating_add(buf.len() as u64) > self.rotation.limit {
            self.roll();
        }
        if self.file.is_none() {
            self.reopen();
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other(format!("cannot open {}", self.path.display())))?;
        let n = file.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writing_past_the_limit_keeps_at_most_the_configured_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vornd.log");
        let rotation = Rotation {
            limit: 100,
            files: 3,
        };
        let mut log = LogFile::open(&path, rotation).unwrap();
        // One write per line, as tracing writes each event.
        for i in 0..200 {
            log.write_all(format!("line {i:04} of the log\n").as_bytes())
                .unwrap();
        }
        log.flush().unwrap();
        assert_eq!(
            files_in(dir.path()),
            ["vornd.log", "vornd.log.1", "vornd.log.2"]
        );
        for name in files_in(dir.path()) {
            let text = fs::read_to_string(dir.path().join(&name)).unwrap();
            assert!(
                text.len() as u64 <= rotation.limit,
                "{name} is {}",
                text.len()
            );
            assert!(text.ends_with('\n'), "{name} ends mid-line");
        }
        // The newest lines are in the current file, the ones before in .1.
        let current = fs::read_to_string(&path).unwrap();
        assert!(current.ends_with("line 0199 of the log\n"));
        let older = fs::read_to_string(numbered(&path, 1)).unwrap();
        let first_current = current.lines().next().unwrap();
        let last_older = older.lines().last().unwrap();
        assert!(
            last_older < first_current,
            "{last_older} then {first_current}"
        );
    }

    #[test]
    fn a_full_log_is_set_aside_when_it_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vornd.log");
        fs::write(&path, vec![b'x'; 500]).unwrap();
        let mut log = LogFile::open(
            &path,
            Rotation {
                limit: 100,
                files: 5,
            },
        )
        .unwrap();
        writeln!(log, "fresh").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "fresh\n");
        assert_eq!(fs::metadata(numbered(&path, 1)).unwrap().len(), 500);
    }

    #[test]
    fn a_log_under_the_limit_is_appended_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessiond.log");
        fs::write(&path, "before\n").unwrap();
        let rotation = Rotation::DEFAULT;
        assert!(!rotate_if_full(&path, rotation).unwrap());
        let mut log = LogFile::open(&path, rotation).unwrap();
        writeln!(log, "after").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "before\nafter\n");
        assert_eq!(files_in(dir.path()), ["sessiond.log"]);
    }

    #[test]
    fn a_line_longer_than_the_limit_lands_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vornd.log");
        let mut log = LogFile::open(
            &path,
            Rotation {
                limit: 10,
                files: 2,
            },
        )
        .unwrap();
        log.write_all(b"short\n").unwrap();
        log.write_all(&[b'y'; 40]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), vec![b'y'; 40]);
        assert_eq!(fs::read_to_string(numbered(&path, 1)).unwrap(), "short\n");
    }

    #[test]
    fn rotating_a_missing_log_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.log");
        assert!(!rotate_if_full(&path, Rotation::DEFAULT).unwrap());
        rotate(&path, 5).unwrap();
        assert!(files_in(dir.path()).is_empty());
    }

    #[test]
    fn keeping_one_file_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vornd.log");
        fs::write(&path, "old\n").unwrap();
        rotate(&path, 1).unwrap();
        assert!(files_in(dir.path()).is_empty());
    }
}
