//! What vornd as the server owns in its data directory: the lock that makes
//! it the one server there, and the two files anything else on this machine
//! finds and reaches it by, `ws-port` (`{port, pid}`) and `local-token` (the
//! credential, readable by this user only).
//!
//! The lock is taken before anything is published and held for as long as
//! vornd runs; a second vornd on the same directory finds it taken and exits
//! with [`EXIT_TAKEN`], which whoever started it reads as "adopt the one
//! running". A file is only ever removed by the process it names.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tracing::warn;

/// The exit code of a vornd that found another one serving its directory.
pub const EXIT_TAKEN: u8 = 3;

pub const WS_PORT_FILE: &str = "ws-port";
pub const LOCAL_TOKEN_FILE: &str = "local-token";
const LOCK_FILE: &str = "vornd.lock";

/// The data directory, held: dropped, the lock goes with the process.
#[derive(Debug)]
pub struct Held {
    dir: PathBuf,
    _lock: File,
}

/// Another process holds the directory.
#[derive(Debug)]
pub struct Taken;

impl Held {
    /// Takes `dir`, creating it (for this user only) if it is not there.
    pub fn take(dir: &Path) -> Result<Held, Taken> {
        let _ = make_private_dir(dir);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))
            .map_err(|err| {
                warn!(%err, "could not open the lock in the data directory");
                Taken
            })?;
        match lock.try_lock() {
            Ok(()) => Ok(Held {
                dir: dir.to_owned(),
                _lock: lock,
            }),
            Err(_) => Err(Taken),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Publishes where this vornd listens.
    pub fn publish_port(&self, port: u16) {
        let record = json!({ "port": port, "pid": std::process::id() });
        if let Err(err) = std::fs::write(self.dir.join(WS_PORT_FILE), record.to_string()) {
            warn!(%err, "could not write the port file; nothing else can find this server");
        }
    }

    /// Publishes the credential, readable by this user only.
    pub fn publish_credential(&self, secret: &[u8]) {
        let path = self.dir.join(LOCAL_TOKEN_FILE);
        let _ = std::fs::remove_file(&path);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let written = options.open(&path).and_then(|mut f| f.write_all(secret));
        if let Err(err) = written {
            warn!(%err, "could not publish the local credential; tools on this machine cannot connect");
        }
    }

    /// Takes back what this vornd published, where it is still its own.
    pub fn withdraw(&self, secret: &[u8]) {
        let port = self.dir.join(WS_PORT_FILE);
        let ours = std::fs::read(&port)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|v| v["pid"] == json!(std::process::id()));
        if ours {
            let _ = std::fs::remove_file(port);
        }
        let token = self.dir.join(LOCAL_TOKEN_FILE);
        if std::fs::read(&token).is_ok_and(|t| t == secret) {
            let _ = std::fs::remove_file(token);
        }
    }
}

/// `dir` and its parents, private to this user where the platform says so.
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Lets a debug build use the default data directory.
pub const ALLOW_DEFAULT_VAR: &str = "VORN_ALLOW_DEFAULT_DATA_DIR";

/// Refuses `dir` when it is the default data directory, `~/.vorn`, and this is
/// a debug build not told otherwise: a test that forgets its own directory
/// must never reach a person's data.
pub fn refuse_default(dir: &Path, home: Option<&Path>, allowed: bool) -> Result<(), String> {
    if !cfg!(debug_assertions) || allowed {
        return Ok(());
    }
    let Some(home) = home else {
        return Ok(());
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    if canon(dir) == canon(&home.join(".vorn")) {
        return Err(format!(
            "a debug build will not use the default data directory {}; set {ALLOW_DEFAULT_VAR}=1 to allow it",
            dir.display()
        ));
    }
    Ok(())
}

/// The credential clients on this machine authenticate with: the one the app
/// that started vornd chose, else a new one.
pub fn credential(supplied: Option<Vec<u8>>) -> Vec<u8> {
    supplied.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let id = uuid::Uuid::new_v4();
        let more = uuid::Uuid::new_v4();
        format!("{}{}", id.simple(), more.simple()).into_bytes()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_vornd_holds_a_directory_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let held = Held::take(dir.path()).unwrap();
        assert!(Held::take(dir.path()).is_err());
        drop(held);
        assert!(Held::take(dir.path()).is_ok());
    }

    #[test]
    fn publishes_and_takes_back_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let held = Held::take(dir.path()).unwrap();
        held.publish_port(50091);
        held.publish_credential(b"secret");
        let port: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(WS_PORT_FILE)).unwrap()).unwrap();
        assert_eq!(port, json!({ "port": 50091, "pid": std::process::id() }));
        assert_eq!(
            std::fs::read(dir.path().join(LOCAL_TOKEN_FILE)).unwrap(),
            b"secret"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(LOCAL_TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Someone else's credential stays.
        std::fs::write(dir.path().join(LOCAL_TOKEN_FILE), "theirs").unwrap();
        held.withdraw(b"secret");
        assert!(!dir.path().join(WS_PORT_FILE).exists());
        assert!(dir.path().join(LOCAL_TOKEN_FILE).exists());
    }

    #[test]
    fn a_debug_build_keeps_off_the_default_data_directory() {
        let home = tempfile::tempdir().unwrap();
        let default = home.path().join(".vorn");
        std::fs::create_dir(&default).unwrap();
        let refused = refuse_default(&default, Some(home.path()), false);
        assert_eq!(refused.is_err(), cfg!(debug_assertions));
        assert!(refuse_default(&default, Some(home.path()), true).is_ok());
        assert!(refuse_default(&home.path().join("other"), Some(home.path()), false).is_ok());
        assert!(refuse_default(&default, None, false).is_ok());
    }

    #[test]
    fn keeps_the_credential_it_was_given_or_makes_one() {
        assert_eq!(credential(Some(b"given".to_vec())), b"given");
        let made = credential(None);
        assert_eq!(made.len(), 64);
        assert_ne!(credential(Some(Vec::new())), made);
    }
}
