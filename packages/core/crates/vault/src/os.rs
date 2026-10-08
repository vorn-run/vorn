//! The platform's keychain: Keychain on macOS, Credential Manager on Windows,
//! Secret Service on Linux. One item per secret, service [`Kind::service`]
//! and account the owner's id.

use std::sync::Arc;

use crate::{Error, Keychain, Result};
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use crate::{Kind, Op, Secret};

/// This platform's keychain, once it has answered a read.
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub fn open() -> Result<Arc<dyn Keychain>> {
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new();
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    let store: Arc<keyring_core::CredentialStore> =
        store.map_err(|e| Error::Unavailable(e.to_string()))?;
    let keychain = OsKeychain { store };
    // A Linux machine without a session bus only says so when asked.
    keychain
        .get(Kind::Connection, "vorn-probe")
        .map_err(|e| Error::Unavailable(e.to_string()))?;
    Ok(Arc::new(keychain))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub fn open() -> Result<Arc<dyn Keychain>> {
    Err(Error::Unavailable("this platform has no keychain Vorn uses".into()))
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
struct OsKeychain {
    store: Arc<keyring_core::CredentialStore>,
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
impl std::fmt::Debug for OsKeychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OsKeychain")
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
impl OsKeychain {
    fn entry(&self, op: Op, kind: Kind, id: &str) -> Result<keyring_core::Entry> {
        self.store
            .build(kind.service(), id, None)
            .map_err(|e| failed(op, kind, &e))
    }
}

/// The keychain's error without what it may carry of an item's bytes.
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn failed(op: Op, kind: Kind, err: &keyring_core::Error) -> Error {
    use keyring_core::Error as K;
    match err {
        K::BadEncoding(_) | K::BadDataFormat(..) => Error::Corrupt { kind },
        other => Error::Failed {
            op,
            kind,
            reason: other.to_string(),
        },
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
impl Keychain for OsKeychain {
    fn get(&self, kind: Kind, id: &str) -> Result<Option<Secret>> {
        match self.entry(Op::Read, kind, id)?.get_password() {
            Ok(text) => Ok(Some(Secret::new(text))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(failed(Op::Read, kind, &err)),
        }
    }

    fn set(&self, kind: Kind, id: &str, secret: &Secret) -> Result<()> {
        self.entry(Op::Write, kind, id)?
            .set_password(secret.expose())
            .map_err(|e| failed(Op::Write, kind, &e))
    }

    fn delete(&self, kind: Kind, id: &str) -> Result<()> {
        match self.entry(Op::Delete, kind, id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(err) => Err(failed(Op::Delete, kind, &err)),
        }
    }
}

#[cfg(test)]
mod tests {
    /// The real keychain where there is one; skipped, and says so, where not.
    #[test]
    fn round_trips_an_item_through_the_os_keychain() {
        use crate::{Kind, Secret};
        let keychain = match super::open() {
            Ok(k) => k,
            Err(err) => {
                eprintln!("skipped: {err}");
                return;
            }
        };
        let id = format!("vorn-vault-test-{}", std::process::id());
        keychain
            .set(Kind::SshKey, &id, &Secret::from("PRIVATE"))
            .unwrap();
        assert_eq!(
            keychain.get(Kind::SshKey, &id).unwrap().unwrap().expose(),
            "PRIVATE"
        );
        keychain.delete(Kind::SshKey, &id).unwrap();
        assert_eq!(keychain.get(Kind::SshKey, &id).unwrap(), None);
        keychain.delete(Kind::SshKey, &id).unwrap();
    }
}
