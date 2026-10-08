//! Connection secrets, kept in the vault ([`vorn_vault`]).
//!
//! One item per connection, holding its secret fields as JSON. With
//! `VORND_KEYCHAIN=0` the OS keychain is not used and items go to the
//! vault's private file; until vornd knows its data directory there is no
//! vault, and nothing is kept between runs.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value};
use tracing::{debug, warn};
use vorn_vault::{Keychain, Secret};

/// A connection's secret fields, by field key.
pub type Fields = BTreeMap<String, Secret>;

/// What vornd knows of one connection's secrets.
#[derive(Clone, Debug, PartialEq)]
pub enum Known {
    /// Its secret fields.
    Fields(Fields),
    /// It has none.
    None,
    /// Nothing was set since vornd started, and the vault has no item.
    Unknown,
}

enum Job {
    Set(String, Fields),
    Delete(String),
}

/// The secrets of every connection vornd has heard of.
pub struct Secrets {
    known: Mutex<HashMap<String, Known>>,
    keychain: OnceLock<Arc<dyn Keychain>>,
    /// Vault writes, one at a time in the order they were made, off the
    /// thread that heard them.
    writes: OnceLock<Mutex<mpsc::Sender<Job>>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("vault", &self.keychain.get().is_some())
            .finish_non_exhaustive()
    }
}

impl Secrets {
    /// No vault until [`Secrets::settle`] names the data directory.
    pub fn new() -> Secrets {
        Secrets {
            known: Mutex::default(),
            keychain: OnceLock::new(),
            writes: OnceLock::new(),
        }
    }

    pub fn with_keychain(keychain: Option<Arc<dyn Keychain>>) -> Secrets {
        let secrets = Secrets::new();
        if let Some(keychain) = keychain {
            let _ = secrets.keychain.set(keychain);
        }
        secrets
    }

    /// Opens the vault: the OS keychain unless `VORND_KEYCHAIN=0` or there
    /// is none, else a private file in `dir`. Only the first call counts.
    pub fn settle(&self, dir: &Path) {
        let use_os = !std::env::var("VORND_KEYCHAIN").is_ok_and(|v| v == "0");
        self.settle_in(dir, use_os);
    }

    fn settle_in(&self, dir: &Path, use_os: bool) {
        if self.keychain.get().is_some() {
            return;
        }
        let (keychain, backing) = vorn_vault::open(use_os, Some(&dir.join("vornd")));
        debug!(?backing, "connection secrets are kept");
        let _ = self.keychain.set(keychain);
    }

    /// `fields` are the connection's secrets now.
    pub fn set(&self, id: &str, fields: Fields) {
        if self.remember(id, Known::Fields(fields.clone())) {
            self.write(Job::Set(id.to_owned(), fields));
        }
    }

    /// One field replaced, the others kept: `connection:rotateSecret`.
    pub fn merge(&self, id: &str, field: &str, value: &str) {
        let mut fields = match self.lookup(id) {
            Known::Fields(fields) => fields,
            Known::None | Known::Unknown => Fields::new(),
        };
        fields.insert(field.to_owned(), Secret::from(value));
        self.set(id, fields);
    }

    /// The connection has no secrets.
    pub fn clear(&self, id: &str) {
        if self.remember(id, Known::None) {
            self.write(Job::Delete(id.to_owned()));
        }
    }

    /// The connection is gone.
    pub fn forget(&self, id: &str) {
        self.lock().remove(id);
        self.write(Job::Delete(id.to_owned()));
    }

    /// What vornd knows of `id`'s secrets, asking the vault once when
    /// nothing was pushed since it started. Blocks while it asks.
    pub fn lookup(&self, id: &str) -> Known {
        if let Some(known) = self.lock().get(id) {
            return known.clone();
        }
        let Some(keychain) = self.keychain.get() else {
            return Known::Unknown;
        };
        let found = match vorn_vault::connection_fields(keychain.as_ref(), id) {
            Ok(Some(fields)) => Known::Fields(fields),
            Ok(None) => return Known::Unknown,
            Err(err) => {
                warn!(%err, "could not read a connection's secrets");
                return Known::Unknown;
            }
        };
        // A push that came in meanwhile is newer than the item.
        self.lock().entry(id.to_owned()).or_insert(found).clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Known>> {
        self.known.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records `known`; whether it differs from what was known before.
    fn remember(&self, id: &str, known: Known) -> bool {
        let mut map = self.lock();
        if map.get(id) == Some(&known) {
            return false;
        }
        map.insert(id.to_owned(), known);
        true
    }

    fn write(&self, job: Job) {
        let Some(keychain) = self.keychain.get() else {
            return;
        };
        let tx = self.writes.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            let keychain = Arc::clone(keychain);
            let logs = tracing::dispatcher::get_default(Clone::clone);
            std::thread::Builder::new()
                .name("vornd-keychain".into())
                .spawn(move || {
                    let _logs = tracing::dispatcher::set_default(&logs);
                    for job in rx {
                        let (what, done) = match &job {
                            Job::Set(id, fields) => (
                                "write",
                                vorn_vault::set_connection_fields(keychain.as_ref(), id, fields),
                            ),
                            Job::Delete(id) => {
                                ("delete", keychain.delete(vorn_vault::Kind::Connection, id))
                            }
                        };
                        match done {
                            Ok(()) => debug!(what, "vault updated"),
                            Err(err) => warn!(%err, what, "could not update the vault"),
                        }
                    }
                })
                .expect("a thread for vault writes");
            Mutex::new(tx)
        });
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(job);
    }
}

impl Default for Secrets {
    fn default() -> Self {
        Secrets::new()
    }
}

/// Fields from JSON: a string as it is, anything else as its JSON text.
#[cfg(test)]
fn fields_of(map: &Map<String, Value>) -> Fields {
    map.iter()
        .map(|(k, v)| {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), Secret::new(text))
        })
        .collect()
}

/// The values of a connection's `secretEnv` field: its plaintext, a JSON
/// object, as `parseJsonObject` reads it.
pub fn secret_env(fields: &Fields) -> Map<String, Value> {
    let raw = fields
        .get("secretEnv")
        .map(|s| Value::String(s.expose().to_owned()));
    super::mcp::parse_json_object(raw.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vorn_vault::{Kind, Memory};

    fn fields(v: Value) -> Fields {
        fields_of(v.as_object().unwrap())
    }

    /// Waits for the vault thread to catch up.
    fn settled(memory: &Memory, writes: usize) {
        for _ in 0..200 {
            if memory.writes().len() >= writes {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the vault saw {:?}", memory.writes());
    }

    #[test]
    fn files_what_it_is_given_in_the_vault() {
        let memory = Arc::new(Memory::default());
        let secrets = Secrets::with_keychain(Some(memory.clone()));
        assert_eq!(secrets.lookup("c1"), Known::Unknown);
        let given = fields(json!({ "secretEnv": "{\"K\":\"v\"}" }));
        secrets.set("c1", given.clone());
        // The same fields again write nothing.
        secrets.set("c1", given.clone());
        settled(&memory, 1);
        assert_eq!(secrets.lookup("c1"), Known::Fields(given.clone()));

        // A vornd started later reads them back from the vault.
        let later = Secrets::with_keychain(Some(memory.clone()));
        assert_eq!(later.lookup("c1"), Known::Fields(given));

        secrets.clear("c1");
        assert_eq!(secrets.lookup("c1"), Known::None);
        settled(&memory, 2);
        assert_eq!(memory.get(Kind::Connection, "c1").unwrap(), None);
        assert_eq!(memory.writes(), ["set c1", "delete c1"]);
    }

    #[test]
    fn a_rotated_secret_keeps_the_other_fields() {
        let secrets = Secrets::default();
        secrets.set("c1", fields(json!({ "token": "a", "secretEnv": "{}" })));
        secrets.merge("c1", "secretEnv", "{\"K\":\"new\"}");
        assert_eq!(
            secrets.lookup("c1"),
            Known::Fields(fields(
                json!({ "token": "a", "secretEnv": "{\"K\":\"new\"}" })
            ))
        );
        secrets.forget("c1");
        assert_eq!(secrets.lookup("c1"), Known::Unknown);
    }

    #[test]
    fn reads_the_secret_env_as_the_server_parses_it() {
        let env = secret_env(&fields(json!({ "secretEnv": "{\"A\":1,\"B\":\"x\"}" })));
        assert_eq!(Value::Object(env), json!({ "A": "1", "B": "x" }));
        assert!(secret_env(&fields(json!({ "secretEnv": "[1]" }))).is_empty());
        assert!(secret_env(&fields(json!({}))).is_empty());
    }

    #[test]
    fn settles_on_a_private_file_when_told_not_to_use_the_os_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new();
        secrets.settle_in(dir.path(), false);
        secrets.set("c1", fields(json!({ "token": "t" })));
        let file = dir.path().join("vornd").join(vorn_vault::file::FILE_NAME);
        for _ in 0..200 {
            if file.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let later = Secrets::new();
        later.settle_in(dir.path(), false);
        assert_eq!(
            later.lookup("c1"),
            Known::Fields(fields(json!({ "token": "t" })))
        );
    }

    /// Everything logged, into one buffer.
    #[derive(Clone, Default)]
    struct Logged(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logged {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn no_secret_reaches_the_log_or_a_debug_print() {
        let logged = Logged::default();
        let writer = logged.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let memory = Arc::new(Memory::default());
            let secrets = Secrets::with_keychain(Some(memory.clone()));
            secrets.set("c1", fields(json!({ "token": "hunter2-secret" })));
            settled(&memory, 1);
            // A write the keychain refuses, and an item that is not what vornd wrote.
            let locked = Arc::new(Memory::default());
            locked.fail_writes(true);
            Secrets::with_keychain(Some(locked)).merge("c3", "token", "hunter3-secret");
            memory
                .set(Kind::Connection, "c2", &Secret::from("hunter4-secret"))
                .unwrap();
            assert_eq!(secrets.lookup("c2"), Known::Unknown);
            let shown = format!("{:?} {:?}", secrets.lookup("c1"), secrets);
            assert!(!shown.contains("hunter"), "{shown}");
        });
        // The refused write is logged from the vault's thread, after it fails.
        for _ in 0..200 {
            if String::from_utf8_lossy(&logged.0.lock().unwrap()).contains("could not update") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let text = String::from_utf8_lossy(&logged.0.lock().unwrap()).into_owned();
        assert!(text.contains("could not update the vault"), "{text}");
        assert!(text.contains("could not read a connection"), "{text}");
        assert!(!text.contains("hunter"), "{text}");
    }
}
