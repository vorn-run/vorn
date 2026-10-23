//! Each extension's own files: what `blobs.*` reads and writes, kept in a
//! folder per extension under one root, held to a size per file and a quota
//! per extension.
//!
//! A name is segments of letters, digits, `.`, `_` and `-` joined by `/`, so
//! it never leaves the extension's folder and reads the same on every
//! platform. A write lands whole or not at all: it goes to a temporary file
//! that is renamed over the old one. Everything here blocks on the disk; a
//! daemon calls it off its runtime.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use crate::api::types::Blob;
use crate::api::ExtError;

/// The most one file holds, as a task's image may: under the 16 MiB a stdio
/// line carries once base64 has grown it.
pub const MAX_BLOB: u64 = 10 << 20;
/// The most one extension keeps over all its files.
pub const QUOTA: u64 = 256 << 20;
/// The longest name, in bytes.
pub const MAX_NAME: usize = 200;
/// Where a write is staged before it is renamed into place; never a name a person gives.
const STAGING: &str = ".staging";

/// One `blobs.*` call, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobCall {
    Put { name: String, data: Vec<u8> },
    Get { name: String },
    List { prefix: String },
    Delete { name: String },
}

/// What a [`BlobCall`] answers.
#[derive(Debug, Clone, PartialEq)]
pub enum BlobAnswer {
    Put,
    Got(Option<Vec<u8>>),
    Listed(Vec<Blob>),
    Deleted(bool),
}

/// Every extension's files, a folder each under `root`.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    quota: u64,
    /// Extensions whose files are kept elsewhere, outside the quota.
    placed: Vec<(String, PathBuf)>,
    /// Held across a put's measure and write, so writes at once cannot pass the quota together.
    writing: Arc<Mutex<()>>,
}

fn denied(message: String) -> ExtError {
    ExtError::new("denied", message)
}

fn failed(what: &str, e: &std::io::Error) -> ExtError {
    ExtError::new("failed", format!("{what}: {e}"))
}

/// Checks `name` and answers the path it names under `dir`.
fn named(dir: &Path, name: &str) -> Result<PathBuf, ExtError> {
    let bad = |why: &str| ExtError::new("bad-request", format!("\"{name}\" {why}"));
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(bad(&format!("is not a file name: 1 to {MAX_NAME} bytes")));
    }
    let mut path = dir.to_path_buf();
    for segment in name.split('/') {
        let allowed = segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if segment.is_empty() || segment.starts_with('.') || !allowed {
            return Err(bad(
                "is not a file name: letters, digits, '.', '_' and '-', in segments joined by '/' that do not start with '.'",
            ));
        }
        path.push(segment);
    }
    Ok(path)
}

/// Every file under `dir` (staging left out) as `(name, path, size, modified)`, by name.
fn walk(dir: &Path) -> std::io::Result<Vec<(String, PathBuf, u64, u64)>> {
    let mut out = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, prefix)) = pending.pop() {
        let entries = match fs::read_dir(&at) {
            Ok(e) => e,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let Ok(file) = entry.file_name().into_string() else {
                continue;
            };
            if file.starts_with('.') {
                continue;
            }
            let name = if prefix.is_empty() {
                file
            } else {
                format!("{prefix}/{file}")
            };
            let meta = entry.metadata()?;
            if meta.is_dir() {
                pending.push((entry.path(), name));
            } else if meta.is_file() {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
                out.push((name, entry.path(), meta.len(), modified));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

impl BlobStore {
    /// The files of every extension, under `root`.
    pub fn new(root: impl Into<PathBuf>) -> BlobStore {
        BlobStore::with_quota(root, QUOTA)
    }

    /// As [`BlobStore::new`], each extension held to `quota` bytes.
    pub fn with_quota(root: impl Into<PathBuf>, quota: u64) -> BlobStore {
        BlobStore {
            root: root.into(),
            quota,
            placed: Vec::new(),
            writing: Arc::default(),
        }
    }

    /// Keeps extension `id`'s files in `folder`, with no quota: for an
    /// extension Vorn ships whose files were there before the store, as
    /// Tasks' images are, so a path handed out before still reads.
    pub fn place(mut self, id: &str, folder: impl Into<PathBuf>) -> BlobStore {
        self.placed.push((id.to_owned(), folder.into()));
        self
    }

    /// Where extension `id`'s file `name` is on disk, once the name checks.
    pub fn path(&self, id: &str, name: &str) -> Result<PathBuf, ExtError> {
        named(&self.dir(id), name)
    }

    fn quota_of(&self, id: &str) -> Option<u64> {
        (!self.placed.iter().any(|(p, _)| p == id)).then_some(self.quota)
    }

    /// Extension `id`'s folder; its name escapes the id as the store's tables do.
    pub fn dir(&self, id: &str) -> PathBuf {
        if let Some((_, folder)) = self.placed.iter().find(|(p, _)| p == id) {
            return folder.clone();
        }
        let mut name = String::with_capacity(id.len() + 4);
        for c in id.chars() {
            match c {
                'a'..='z' | '0'..='9' => name.push(c),
                'A'..='Z' => {
                    name.push('_');
                    name.push(c.to_ascii_lowercase());
                }
                '.' => name.push_str("_0"),
                '-' => name.push_str("_1"),
                _ => name.push_str("_2"),
            }
        }
        self.root.join(name)
    }

    /// Runs `call` on extension `id`'s files.
    pub fn run(&self, id: &str, call: BlobCall) -> Result<BlobAnswer, ExtError> {
        let dir = self.dir(id);
        match call {
            BlobCall::Put { name, data } => self
                .put(&dir, &name, &data, self.quota_of(id))
                .map(|()| BlobAnswer::Put),
            BlobCall::Get { name } => {
                let path = named(&dir, &name)?;
                match fs::read(&path) {
                    Ok(bytes) => Ok(BlobAnswer::Got(Some(bytes))),
                    Err(e)
                        if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
                    {
                        Ok(BlobAnswer::Got(None))
                    }
                    Err(e) => Err(failed(&format!("reading {name}"), &e)),
                }
            }
            BlobCall::List { prefix } => {
                let all = walk(&dir).map_err(|e| failed("listing its files", &e))?;
                Ok(BlobAnswer::Listed(
                    all.into_iter()
                        .filter(|(n, ..)| n.starts_with(&prefix))
                        .map(|(name, _, size, modified)| Blob {
                            name,
                            size,
                            modified,
                        })
                        .collect(),
                ))
            }
            BlobCall::Delete { name } => {
                let path = named(&dir, &name)?;
                match fs::remove_file(&path) {
                    Ok(()) => {
                        prune_empty(&dir, &path);
                        Ok(BlobAnswer::Deleted(true))
                    }
                    Err(e)
                        if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
                    {
                        Ok(BlobAnswer::Deleted(false))
                    }
                    Err(e) => Err(failed(&format!("removing {name}"), &e)),
                }
            }
        }
    }

    fn put(&self, dir: &Path, name: &str, data: &[u8], quota: Option<u64>) -> Result<(), ExtError> {
        let path = named(dir, name)?;
        let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
        if size > MAX_BLOB {
            return Err(denied(format!(
                "{name} is {size} bytes; a file holds {MAX_BLOB} at most"
            )));
        }
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(quota) = quota {
            let kept: u64 = walk(dir)
                .map_err(|e| failed("measuring its files", &e))?
                .iter()
                .filter(|(n, ..)| n != name)
                .map(|(.., size, _)| size)
                .sum();
            if kept.saturating_add(size) > quota {
                return Err(denied(format!(
                    "writing {name} would keep {} bytes; an extension keeps {quota} at most",
                    kept.saturating_add(size),
                )));
            }
        }
        let parent = path.parent().unwrap_or(dir);
        fs::create_dir_all(parent).map_err(|e| failed(&format!("writing {name}"), &e))?;
        let staging = dir.join(STAGING);
        fs::create_dir_all(&staging).map_err(|e| failed(&format!("writing {name}"), &e))?;
        let part = staging.join(format!("{}.part", unique()));
        let written = fs::write(&part, data).and_then(|()| fs::rename(&part, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&part);
            return Err(failed(&format!("writing {name}"), &e));
        }
        Ok(())
    }

    /// Removes every file extension `id` keeps; files placed elsewhere stay.
    pub fn forget(&self, id: &str) -> std::io::Result<()> {
        if self.quota_of(id).is_none() {
            return Ok(());
        }
        match fs::remove_dir_all(self.dir(id)) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// A name no other write in this process uses at the same time.
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Removes the folders a delete left empty, up to the extension's own.
fn prune_empty(dir: &Path, removed: &Path) {
    let mut at = removed.parent();
    while let Some(folder) = at.filter(|f| *f != dir && f.starts_with(dir)) {
        if fs::remove_dir(folder).is_err() {
            break;
        }
        at = folder.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(quota: u64) -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::with_quota(dir.path().join("blobs"), quota);
        (dir, store)
    }

    fn put(s: &BlobStore, id: &str, name: &str, data: &[u8]) -> Result<BlobAnswer, ExtError> {
        s.run(
            id,
            BlobCall::Put {
                name: name.into(),
                data: data.to_vec(),
            },
        )
    }

    #[test]
    fn a_placed_folder_keeps_its_files_where_they_were_with_no_quota() {
        let dir = tempfile::tempdir().unwrap();
        let images = dir.path().join("task-images");
        let s = BlobStore::with_quota(dir.path().join("blobs"), 4).place("vorn.tasks", &images);
        assert!(matches!(
            put(&s, "vorn.tasks", "t1/a.png", b"123456"),
            Ok(BlobAnswer::Put)
        ));
        assert_eq!(fs::read(images.join("t1/a.png")).unwrap(), b"123456");
        assert_eq!(
            s.path("vorn.tasks", "t1/a.png").unwrap(),
            images.join("t1/a.png")
        );
        assert!(s.path("vorn.tasks", "../x").is_err());
        // Another extension is still held to the quota.
        assert!(put(&s, "a.b", "big", b"123456").is_err());
        s.forget("vorn.tasks").unwrap();
        assert!(
            images.join("t1/a.png").exists(),
            "a placed folder is never removed"
        );
    }

    fn get(s: &BlobStore, id: &str, name: &str) -> Option<Vec<u8>> {
        match s.run(id, BlobCall::Get { name: name.into() }).unwrap() {
            BlobAnswer::Got(b) => b,
            other => panic!("{other:?}"),
        }
    }

    fn names(s: &BlobStore, id: &str, prefix: &str) -> Vec<String> {
        match s
            .run(
                id,
                BlobCall::List {
                    prefix: prefix.into(),
                },
            )
            .unwrap()
        {
            BlobAnswer::Listed(l) => l.into_iter().map(|b| b.name).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn writes_reads_lists_and_deletes_an_extensions_own_files() {
        let (_d, s) = store(QUOTA);
        put(&s, "a.b", "images/t1/one.png", b"\x89PNG").unwrap();
        put(&s, "a.b", "notes.txt", b"hi").unwrap();
        put(&s, "a.b", "notes.txt", b"hello").unwrap();
        assert_eq!(get(&s, "a.b", "notes.txt").as_deref(), Some(&b"hello"[..]));
        assert_eq!(get(&s, "a.b", "missing"), None);
        assert_eq!(names(&s, "a.b", ""), ["images/t1/one.png", "notes.txt"]);
        assert_eq!(names(&s, "a.b", "images/"), ["images/t1/one.png"]);
        // Another extension sees none of them.
        assert!(names(&s, "a.c", "").is_empty());
        assert_eq!(get(&s, "a.c", "notes.txt"), None);
        let deleted = |name: &str| s.run("a.b", BlobCall::Delete { name: name.into() });
        assert_eq!(
            deleted("images/t1/one.png").unwrap(),
            BlobAnswer::Deleted(true)
        );
        assert_eq!(
            deleted("images/t1/one.png").unwrap(),
            BlobAnswer::Deleted(false)
        );
        assert!(!s.dir("a.b").join("images").exists(), "empty folders go");
        s.forget("a.b").unwrap();
        assert!(names(&s, "a.b", "").is_empty());
    }

    #[test]
    fn refuses_names_that_could_leave_the_folder() {
        let (_d, s) = store(QUOTA);
        for name in [
            "",
            "../x",
            "a/../../x",
            "/etc/passwd",
            "a//b",
            ".hidden",
            "a/.staging/x",
            "a\\b",
            "c:x",
            "ü",
            &"x".repeat(MAX_NAME + 1),
        ] {
            let err = put(&s, "a", name, b"x").unwrap_err();
            assert_eq!(err.code, "bad-request", "{name:?}: {}", err.message);
        }
    }

    #[test]
    fn holds_each_file_and_each_extension_to_its_size() {
        let (_d, s) = store(10);
        put(&s, "a", "one", b"123456").unwrap();
        let err = put(&s, "a", "two", b"12345").unwrap_err();
        assert_eq!(err.code, "denied");
        assert!(err.message.contains("keeps 10 at most"), "{}", err.message);
        // Replacing a file counts its new size, not both.
        put(&s, "a", "one", b"1234567890").unwrap();
        put(&s, "b", "one", b"1234567890").unwrap();
        let (_d, s) = store(QUOTA);
        let big = vec![0u8; usize::try_from(MAX_BLOB).unwrap() + 1];
        assert_eq!(put(&s, "a", "big", &big).unwrap_err().code, "denied");
    }

    #[test]
    fn ids_that_differ_keep_folders_apart() {
        let (_d, s) = store(QUOTA);
        assert_ne!(s.dir("a.b"), s.dir("a_b"));
        assert_ne!(s.dir("a-b"), s.dir("a.b"));
        assert_ne!(s.dir("A"), s.dir("a"));
    }
}
