//! Items in a file only this user can read, where there is no keychain.
//!
//! One JSON object by service then id, rewritten whole through a temporary
//! file on every change, so a crash leaves the old file or the new one.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::{Error, Keychain, Kind, Op, Result, Secret};

/// The file's name in the directory it is kept in.
pub const FILE_NAME: &str = "credentials.json";

#[derive(Debug)]
pub struct FileKeychain {
    path: PathBuf,
    /// Held across each read-modify-write.
    lock: Mutex<()>,
}

impl FileKeychain {
    pub fn new(path: PathBuf) -> FileKeychain {
        FileKeychain {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self, op: Op, kind: Kind) -> Result<Map<String, Value>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(Value::Object(map)) => Ok(map),
                _ => Err(Error::Corrupt { kind }),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
            Err(err) => Err(failed(op, kind, &err)),
        }
    }

    fn write(&self, kind: Kind, op: Op, all: &Map<String, Value>) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed(op, kind, &e))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        write_private(&tmp, Value::Object(all.clone()).to_string().as_bytes())
            .map_err(|e| failed(op, kind, &e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| failed(op, kind, &e))
    }

    fn change(&self, kind: Kind, op: Op, f: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.read(op, kind)?;
        let mut service = match all.remove(kind.service()) {
            Some(Value::Object(items)) => items,
            _ => Map::new(),
        };
        f(&mut service);
        if !service.is_empty() {
            all.insert(kind.service().to_owned(), Value::Object(service));
        }
        self.write(kind, op, &all)
    }
}

fn failed(op: Op, kind: Kind, err: &std::io::Error) -> Error {
    Error::Failed {
        op,
        kind,
        reason: err.to_string(),
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

impl Keychain for FileKeychain {
    fn get(&self, kind: Kind, id: &str) -> Result<Option<Secret>> {
        let _held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let all = self.read(Op::Read, kind)?;
        match all.get(kind.service()).and_then(|items| items.get(id)) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(Secret::new(text.clone()))),
            Some(_) => Err(Error::Corrupt { kind }),
        }
    }

    fn set(&self, kind: Kind, id: &str, secret: &Secret) -> Result<()> {
        self.change(kind, Op::Write, |items| {
            items.insert(id.to_owned(), Value::String(secret.expose().to_owned()));
        })
    }

    fn delete(&self, kind: Kind, id: &str) -> Result<()> {
        self.change(kind, Op::Delete, |items| {
            items.remove(id);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_items_per_kind_in_a_file_only_this_user_reads() {
        let dir = tempfile::tempdir().unwrap();
        let file = FileKeychain::new(dir.path().join("vornd").join(FILE_NAME));
        assert_eq!(file.get(Kind::SshKey, "k").unwrap(), None);
        file.set(Kind::SshKey, "k", &Secret::from("PRIVATE")).unwrap();
        file.set(Kind::HostPassword, "k", &Secret::from("pw")).unwrap();
        assert_eq!(file.get(Kind::SshKey, "k").unwrap().unwrap().expose(), "PRIVATE");
        assert_eq!(file.get(Kind::HostPassword, "k").unwrap().unwrap().expose(), "pw");
        file.delete(Kind::SshKey, "k").unwrap();
        file.delete(Kind::SshKey, "k").unwrap();
        assert_eq!(file.get(Kind::SshKey, "k").unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_file_that_is_not_its_own_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "[]").unwrap();
        let file = FileKeychain::new(path.clone());
        assert_eq!(
            file.set(Kind::SshKey, "k", &Secret::from("x")).unwrap_err(),
            Error::Corrupt { kind: Kind::SshKey }
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "[]");
    }
}
