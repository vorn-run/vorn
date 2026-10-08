//! Where Vorn keeps secrets: a connection's credentials, an SSH key, a remote
//! host's password.
//!
//! Each secret is one item in the OS keychain ([`os`]): Keychain on macOS,
//! Credential Manager on Windows, Secret Service on Linux. Where there is no
//! keychain to reach (a Linux machine with no session bus), items go to a
//! file only this user can read ([`file`]), which is what the desktop's own
//! sealing amounted to there. [`open`] picks the first that works.
//!
//! A secret's text is a [`Secret`], which never prints: its `Debug` is
//! redacted and it has no `Display`, so a log line or an error that names one
//! by mistake shows nothing. No [`Error`] carries an item's contents.
//!
//! The desktop used to seal secrets with its own encryption before they were
//! stored in the database. [`import`] files what it decrypted once, so that
//! from then on vornd holds them and no process has to hand them over again.

pub mod file;
pub mod memory;
pub mod os;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

pub use file::FileKeychain;
pub use memory::Memory;

/// A secret's text. Never printed: see the module docs.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Secret {
        Secret(text.into())
    }

    /// The text, for the one place that hands it to what needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

impl From<String> for Secret {
    fn from(text: String) -> Secret {
        Secret(text)
    }
}

impl From<&str> for Secret {
    fn from(text: &str) -> Secret {
        Secret(text.to_owned())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

/// What a secret belongs to, which decides the keychain service it is filed
/// under. The account is the owner's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A connection's secret fields, as one JSON object by field key.
    Connection,
    /// An SSH private key the user stored.
    SshKey,
    /// A remote host's password.
    HostPassword,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Connection, Kind::SshKey, Kind::HostPassword];

    /// The keychain service. `Connection`'s is the one vornd has filed
    /// connection items under since it first kept them.
    pub fn service(self) -> &'static str {
        match self {
            Kind::Connection => "Vorn connection credentials",
            Kind::SshKey => "Vorn SSH keys",
            Kind::HostPassword => "Vorn remote host passwords",
        }
    }
}

/// What was being done when the keychain failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
    Delete,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Delete => "delete",
        })
    }
}

/// Why the vault could not do something. Never holds an item's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// There is no keychain here, or it cannot be reached.
    Unavailable(String),
    /// The keychain refused or failed one operation.
    Failed { op: Op, kind: Kind, reason: String },
    /// An item is there but is not text, or not what vornd wrote.
    Corrupt { kind: Kind },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable(why) => write!(f, "no keychain: {why}"),
            Error::Failed { op, kind, reason } => {
                write!(f, "could not {op} {} in the keychain: {reason}", kind.service())
            }
            Error::Corrupt { kind } => {
                write!(f, "an item of {} is not what Vorn wrote", kind.service())
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Where items are kept between runs. Every call may block on the platform,
/// so none is made on an async task.
pub trait Keychain: Send + Sync + fmt::Debug {
    /// The item for `id`; `Ok(None)` when there is none.
    fn get(&self, kind: Kind, id: &str) -> Result<Option<Secret>>;
    fn set(&self, kind: Kind, id: &str, secret: &Secret) -> Result<()>;
    /// Removes the item for `id`; no item is not an error.
    fn delete(&self, kind: Kind, id: &str) -> Result<()>;
}

/// Which store [`open`] settled on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backing {
    /// The platform's keychain.
    Os,
    /// A private file, because there is no keychain here.
    File(std::path::PathBuf),
    /// Memory only: nothing outlives the process.
    Memory,
}

/// The keychain to keep secrets in: the platform's, unless `use_os` is false
/// or it cannot be reached; then a private file in `dir`; with no `dir`,
/// memory only.
pub fn open(use_os: bool, dir: Option<&Path>) -> (Arc<dyn Keychain>, Backing) {
    if use_os {
        match os::open() {
            Ok(keychain) => return (keychain, Backing::Os),
            Err(err) => tracing::warn!(%err, "no OS keychain; secrets go to a private file"),
        }
    }
    match dir {
        Some(dir) => {
            let file = FileKeychain::new(dir.join(file::FILE_NAME));
            let path = file.path().to_owned();
            (Arc::new(file), Backing::File(path))
        }
        None => (Arc::new(Memory::default()), Backing::Memory),
    }
}

/// What the desktop decrypted of the secrets it sealed, to be filed once.
#[derive(Debug, Default)]
pub struct Import {
    /// Each connection's secret fields, by connection id then field key.
    pub connections: BTreeMap<String, BTreeMap<String, Secret>>,
    /// Private keys, by SSH key id.
    pub ssh_keys: BTreeMap<String, Secret>,
    /// Passwords, by remote host id.
    pub host_passwords: BTreeMap<String, Secret>,
}

/// How many items [`import`] filed, per kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub connections: usize,
    pub ssh_keys: usize,
    pub host_passwords: usize,
}

/// Files everything in `import`. A connection's fields are merged over any
/// it already has, so a field vornd already holds and the import lacks is
/// kept. The first failure stops it: what was filed stays filed, and the
/// caller keeps its copy to try again.
pub fn import(keychain: &dyn Keychain, import: &Import) -> Result<Imported> {
    let mut done = Imported::default();
    for (id, fields) in &import.connections {
        let mut merged = connection_fields(keychain, id)?.unwrap_or_default();
        for (key, value) in fields {
            merged.insert(key.clone(), value.clone());
        }
        set_connection_fields(keychain, id, &merged)?;
        done.connections += 1;
    }
    for (id, key) in &import.ssh_keys {
        keychain.set(Kind::SshKey, id, key)?;
        done.ssh_keys += 1;
    }
    for (id, password) in &import.host_passwords {
        keychain.set(Kind::HostPassword, id, password)?;
        done.host_passwords += 1;
    }
    Ok(done)
}

/// A connection's secret fields, as filed: one JSON object of strings.
pub fn connection_fields(
    keychain: &dyn Keychain,
    id: &str,
) -> Result<Option<BTreeMap<String, Secret>>> {
    let Some(item) = keychain.get(Kind::Connection, id)? else {
        return Ok(None);
    };
    parse_fields(item.expose())
        .map(Some)
        .ok_or(Error::Corrupt {
            kind: Kind::Connection,
        })
}

/// Files a connection's secret fields as one item.
pub fn set_connection_fields(
    keychain: &dyn Keychain,
    id: &str,
    fields: &BTreeMap<String, Secret>,
) -> Result<()> {
    keychain.set(Kind::Connection, id, &Secret::new(fields_json(fields)))
}

/// The fields of a connection item; `None` when it is not a JSON object.
/// A non-string value is kept as its JSON text, as the server's reader did.
pub fn parse_fields(text: &str) -> Option<BTreeMap<String, Secret>> {
    let serde_json::Value::Object(map) = serde_json::from_str(text).ok()? else {
        return None;
    };
    Some(
        map.into_iter()
            .map(|(k, v)| {
                let text = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, Secret::new(text))
            })
            .collect(),
    )
}

/// One item's text for `fields`.
pub fn fields_json(fields: &BTreeMap<String, Secret>) -> String {
    let map: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.expose().to_owned())))
        .collect();
    serde_json::Value::Object(map).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, Secret> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Secret::from(*v)))
            .collect()
    }

    #[test]
    fn a_secret_never_prints() {
        let secret = Secret::from("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(redacted)");
        let import = Import {
            ssh_keys: [("k".to_owned(), secret.clone())].into(),
            ..Import::default()
        };
        assert!(!format!("{import:?}").contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn imports_every_kind_and_keeps_fields_already_held() {
        let memory = Memory::default();
        set_connection_fields(&memory, "c1", &fields(&[("token", "old"), ("extra", "kept")]))
            .unwrap();
        let import = Import {
            connections: [("c1".to_owned(), fields(&[("token", "new")]))].into(),
            ssh_keys: [("k1".to_owned(), Secret::from("PRIVATE"))].into(),
            host_passwords: [("h1".to_owned(), Secret::from("pw"))].into(),
        };
        let done = super::import(&memory, &import).unwrap();
        assert_eq!(
            done,
            Imported {
                connections: 1,
                ssh_keys: 1,
                host_passwords: 1
            }
        );
        assert_eq!(
            connection_fields(&memory, "c1").unwrap().unwrap(),
            fields(&[("extra", "kept"), ("token", "new")])
        );
        assert_eq!(
            memory.get(Kind::SshKey, "k1").unwrap().unwrap().expose(),
            "PRIVATE"
        );
        assert_eq!(
            memory.get(Kind::HostPassword, "h1").unwrap().unwrap().expose(),
            "pw"
        );
        // Run again, as a desktop that did not hear the answer would: the same items.
        super::import(&memory, &import).unwrap();
        assert_eq!(memory.len(), 3);
    }

    #[test]
    fn an_import_stops_at_the_first_failure_and_says_nothing_secret() {
        let memory = Memory::default();
        memory.fail_writes(true);
        let import = Import {
            ssh_keys: [("k1".to_owned(), Secret::from("PRIVATE"))].into(),
            ..Import::default()
        };
        let err = super::import(&memory, &import).unwrap_err();
        assert!(!err.to_string().contains("PRIVATE"), "{err}");
        assert!(!format!("{err:?}").contains("PRIVATE"));
    }

    #[test]
    fn reads_connection_items_as_the_server_wrote_them() {
        let parsed = parse_fields(r#"{"secretEnv":"{\"K\":\"v\"}","n":3}"#).unwrap();
        assert_eq!(parsed["secretEnv"].expose(), r#"{"K":"v"}"#);
        assert_eq!(parsed["n"].expose(), "3");
        assert!(parse_fields("[1]").is_none());
        assert!(parse_fields("not json").is_none());
        let memory = Memory::default();
        memory
            .set(Kind::Connection, "c", &Secret::from("[1]"))
            .unwrap();
        assert_eq!(
            connection_fields(&memory, "c").unwrap_err(),
            Error::Corrupt {
                kind: Kind::Connection
            }
        );
        assert_eq!(connection_fields(&memory, "none").unwrap(), None);
    }

    #[test]
    fn falls_back_to_a_private_file_and_then_to_memory() {
        let dir = tempfile::tempdir().unwrap();
        let (keychain, backing) = open(false, Some(dir.path()));
        assert_eq!(backing, Backing::File(dir.path().join(file::FILE_NAME)));
        keychain
            .set(Kind::HostPassword, "h", &Secret::from("pw"))
            .unwrap();
        let (again, _) = open(false, Some(dir.path()));
        assert_eq!(
            again.get(Kind::HostPassword, "h").unwrap().unwrap().expose(),
            "pw"
        );
        let (_, backing) = open(false, None);
        assert_eq!(backing, Backing::Memory);
    }
}
