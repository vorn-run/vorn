//! Connector credentials vornd holds, and the keychain that keeps them.
//!
//! The server stores a connection's secrets encrypted with the desktop's
//! safeStorage, and the desktop decrypts them and pushes the plaintext to the
//! server (`credentials:setDecrypted`, `credentials:clearDecrypted`) on every
//! start and every configuration change. Those pushes reach the server through
//! vornd, which keeps the same plaintext ([`Secrets::observe`]) and writes it
//! to the OS keychain, where it outlives a vornd restart that the desktop
//! would not push again for. The first push after an update moves every
//! connection's secrets into the keychain, with nobody signing in again.
//!
//! The keychain is Keychain on macOS and Credential Manager on Windows, one
//! item per connection (service [`SERVICE`], account the connection id)
//! holding its fields as JSON. Elsewhere, or with `VORND_KEYCHAIN=0`, the
//! secrets live in memory only, and a call whose secrets vornd does not know
//! goes to the server, which does.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value};
use tracing::{debug, warn};

/// The keychain service connection items are filed under.
pub const SERVICE: &str = "Vorn connection credentials";

/// What vornd knows of one connection's secrets.
#[derive(Clone, Debug, PartialEq)]
pub enum Known {
    /// The fields the desktop decrypted, by field key.
    Fields(Map<String, Value>),
    /// The desktop said there are none it can decrypt.
    None,
    /// Nothing was pushed since vornd started, and the keychain has no item.
    Unknown,
}

/// Where secrets are kept between runs.
pub trait Keychain: Send + Sync {
    /// The item for `id`; `Ok(None)` when there is none.
    fn get(&self, id: &str) -> Result<Option<String>, String>;
    fn set(&self, id: &str, value: &str) -> Result<(), String>;
    /// Removes the item for `id`; no item is not an error.
    fn delete(&self, id: &str) -> Result<(), String>;
}

enum Job {
    Set(String, String),
    Delete(String),
}

/// The secrets of every connection vornd has heard of.
pub struct Secrets {
    known: Mutex<HashMap<String, Known>>,
    keychain: Option<Arc<dyn Keychain>>,
    /// Keychain writes, one at a time in the order they were made, off the
    /// thread that heard them.
    writes: OnceLock<Mutex<mpsc::Sender<Job>>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("keychain", &self.keychain.is_some())
            .finish_non_exhaustive()
    }
}

impl Secrets {
    /// With this platform's keychain, unless `VORND_KEYCHAIN=0` or there is
    /// none.
    pub fn new() -> Secrets {
        let off = std::env::var("VORND_KEYCHAIN").is_ok_and(|v| v == "0");
        Secrets::with_keychain(if off { None } else { os_keychain() })
    }

    pub fn with_keychain(keychain: Option<Arc<dyn Keychain>>) -> Secrets {
        Secrets {
            known: Mutex::default(),
            keychain,
            writes: OnceLock::new(),
        }
    }

    /// Reads a client's call to the server that changes what the server
    /// holds: the desktop's pushes. Returns whether it was one.
    pub fn observe(&self, method: &str, params: &Value) -> bool {
        let id = params.get("connectionId").and_then(Value::as_str);
        match (method, id) {
            ("credentials:setDecrypted", Some(id)) => match params.get("fields") {
                Some(Value::Object(fields)) => {
                    self.set(id, fields.clone());
                    true
                }
                _ => false,
            },
            ("credentials:clearDecrypted", Some(id)) => {
                self.clear(id);
                true
            }
            _ => false,
        }
    }

    /// `fields` are the connection's secrets now, as `setDecryptedCreds`
    /// replaces them.
    pub fn set(&self, id: &str, fields: Map<String, Value>) {
        let known = Known::Fields(fields);
        if self.remember(id, known.clone()) {
            if let Known::Fields(fields) = known {
                self.write(Job::Set(id.to_owned(), Value::Object(fields).to_string()));
            }
        }
    }

    /// One field replaced, the others kept: `connection:rotateSecret`.
    pub fn merge(&self, id: &str, field: &str, value: &str) {
        let mut fields = match self.lookup(id) {
            Known::Fields(fields) => fields,
            Known::None | Known::Unknown => Map::new(),
        };
        fields.insert(field.to_owned(), Value::String(value.to_owned()));
        self.set(id, fields);
    }

    /// The connection has no secrets the desktop can decrypt.
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

    /// What vornd knows of `id`'s secrets, asking the keychain once when
    /// nothing was pushed since it started. Blocks while it asks.
    pub fn lookup(&self, id: &str) -> Known {
        if let Some(known) = self.lock().get(id) {
            return known.clone();
        }
        let Some(keychain) = &self.keychain else {
            return Known::Unknown;
        };
        let found = match keychain.get(id) {
            Ok(Some(text)) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(fields)) => Known::Fields(fields),
                _ => {
                    warn!("a keychain item for a connection is not what vornd wrote; ignoring it");
                    return Known::Unknown;
                }
            },
            Ok(None) => return Known::Unknown,
            Err(err) => {
                warn!(%err, "could not read a connection's secrets from the keychain");
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
        let Some(keychain) = &self.keychain else {
            return;
        };
        let tx = self.writes.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            let keychain = Arc::clone(keychain);
            std::thread::Builder::new()
                .name("vornd-keychain".into())
                .spawn(move || {
                    for job in rx {
                        let (what, done) = match &job {
                            Job::Set(id, value) => ("write", keychain.set(id, value)),
                            Job::Delete(id) => ("delete", keychain.delete(id)),
                        };
                        match done {
                            Ok(()) => debug!(what, "keychain updated"),
                            Err(err) => warn!(%err, what, "could not update the keychain"),
                        }
                    }
                })
                .expect("a thread for keychain writes");
            Mutex::new(tx)
        });
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(job);
    }
}

impl Default for Secrets {
    fn default() -> Self {
        Secrets::with_keychain(None)
    }
}

/// The values of a connection's `secretEnv` field: its plaintext, a JSON
/// object, as `parseJsonObject` reads it.
pub fn secret_env(fields: &Map<String, Value>) -> Map<String, Value> {
    super::mcp::parse_json_object(fields.get("secretEnv"))
}

#[cfg(any(target_os = "macos", windows))]
fn os_keychain() -> Option<Arc<dyn Keychain>> {
    match OsKeychain::open() {
        Ok(keychain) => Some(Arc::new(keychain)),
        Err(err) => {
            warn!(%err, "no keychain; connection secrets stay in memory");
            None
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_keychain() -> Option<Arc<dyn Keychain>> {
    None
}

/// Keychain on macOS, Credential Manager on Windows.
#[cfg(any(target_os = "macos", windows))]
pub struct OsKeychain {
    store: Arc<keyring_core::CredentialStore>,
}

#[cfg(any(target_os = "macos", windows))]
impl OsKeychain {
    pub fn open() -> Result<OsKeychain, String> {
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new();
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new();
        let store: Arc<keyring_core::CredentialStore> = store.map_err(|e| e.to_string())?;
        Ok(OsKeychain { store })
    }

    fn entry(&self, id: &str) -> Result<keyring_core::Entry, String> {
        self.store
            .build(SERVICE, id, None)
            .map_err(|e| e.to_string())
    }
}

#[cfg(any(target_os = "macos", windows))]
impl Keychain for OsKeychain {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        match self.entry(id)?.get_password() {
            Ok(text) => Ok(Some(text)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.entry(id)?
            .set_password(value)
            .map_err(|e| e.to_string())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A keychain in memory that counts its writes.
    #[derive(Default)]
    struct Memory {
        items: Mutex<HashMap<String, String>>,
        writes: Mutex<Vec<String>>,
    }

    impl Keychain for Memory {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            Ok(self.items.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, value: &str) -> Result<(), String> {
            self.writes.lock().unwrap().push(format!("set {id}"));
            self.items
                .lock()
                .unwrap()
                .insert(id.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.writes.lock().unwrap().push(format!("delete {id}"));
            self.items.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn fields(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// Waits for the keychain thread to catch up.
    fn settled(memory: &Memory, writes: usize) {
        for _ in 0..200 {
            if memory.writes.lock().unwrap().len() >= writes {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the keychain saw {:?}", memory.writes.lock().unwrap());
    }

    #[test]
    fn keeps_what_the_desktop_pushes_and_files_it_in_the_keychain() {
        let memory = Arc::new(Memory::default());
        let secrets = Secrets::with_keychain(Some(memory.clone()));
        assert_eq!(secrets.lookup("c1"), Known::Unknown);
        assert!(secrets.observe(
            "credentials:setDecrypted",
            &json!({ "connectionId": "c1", "fields": { "secretEnv": "{\"K\":\"v\"}" } })
        ));
        // The same push again, as every configuration change brings, writes nothing.
        secrets.observe(
            "credentials:setDecrypted",
            &json!({ "connectionId": "c1", "fields": { "secretEnv": "{\"K\":\"v\"}" } }),
        );
        settled(&memory, 1);
        assert_eq!(
            secrets.lookup("c1"),
            Known::Fields(fields(json!({ "secretEnv": "{\"K\":\"v\"}" })))
        );

        // A vornd started later reads them back from the keychain.
        let later = Secrets::with_keychain(Some(memory.clone()));
        assert_eq!(
            later.lookup("c1"),
            Known::Fields(fields(json!({ "secretEnv": "{\"K\":\"v\"}" })))
        );

        assert!(secrets.observe(
            "credentials:clearDecrypted",
            &json!({ "connectionId": "c1" })
        ));
        assert_eq!(secrets.lookup("c1"), Known::None);
        settled(&memory, 2);
        assert_eq!(memory.items.lock().unwrap().get("c1"), None);
        assert_eq!(*memory.writes.lock().unwrap(), ["set c1", "delete c1"]);
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
    fn ignores_what_is_not_a_push() {
        let secrets = Secrets::default();
        assert!(!secrets.observe("credentials:setDecrypted", &json!({ "connectionId": "c1" })));
        assert!(!secrets.observe("credentials:setDecrypted", &json!(null)));
        assert!(!secrets.observe("credentials:clearDecrypted", &json!({ "connectionId": 3 })));
        assert!(!secrets.observe("connection:list", &json!({ "connectionId": "c1" })));
        assert_eq!(secrets.lookup("c1"), Known::Unknown);
    }

    #[test]
    fn reads_the_secret_env_as_the_server_parses_it() {
        let env = secret_env(&fields(json!({ "secretEnv": "{\"A\":1,\"B\":\"x\"}" })));
        assert_eq!(Value::Object(env), json!({ "A": "1", "B": "x" }));
        assert!(secret_env(&fields(json!({ "secretEnv": "[1]" }))).is_empty());
        assert!(secret_env(&fields(json!({}))).is_empty());
    }

    /// The real keychain, on the platforms that have one vornd uses. Writes
    /// and removes one item of its own.
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn round_trips_an_item_through_the_os_keychain() {
        let keychain = OsKeychain::open().expect("a keychain");
        let id = format!("vornd-test-{}", std::process::id());
        keychain.set(&id, "{\"secretEnv\":\"{}\"}").unwrap();
        assert_eq!(
            keychain.get(&id).unwrap().as_deref(),
            Some("{\"secretEnv\":\"{}\"}")
        );
        keychain.delete(&id).unwrap();
        assert_eq!(keychain.get(&id).unwrap(), None);
        keychain.delete(&id).unwrap();
    }
}
