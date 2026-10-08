//! What a terminal on a remote host is given once its local shell is up:
//! the ssh line, the password and stored key read from the vault for this
//! one login, and the command to run there ([`Remote`]). The engine types
//! them as [`vorn_agents::launch::ssh::Login`] reads the output.
//!
//! The credentials leave vornd only
//! over the session holder's local socket, as keystrokes, and as a key file
//! only its owner can read, removed once the login is over. Neither is ever
//! in an argv, an environment, a record, a plan, an answer or a log line:
//! [`Secret`] prints as redacted.

use std::fmt;
use std::io::Write;
use std::path::PathBuf;

/// A credential: never printed, compared only in tests.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Secret {
        Secret(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A stored key, written for the login to a file of its own and removed after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFile {
    pub path: PathBuf,
    content: Secret,
}

impl KeyFile {
    /// A key to be written under the temporary directory, named as the server names its own.
    pub fn new(content: Secret) -> KeyFile {
        let path = std::env::temp_dir().join(format!("vorn-key-{}", uuid::Uuid::new_v4()));
        KeyFile { path, content }
    }

    /// Writes the key, readable by its owner only.
    pub fn write(&self) -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options
            .open(&self.path)?
            .write_all(self.content.expose().as_bytes())
    }

    /// Removes the key; one already gone is not an error.
    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// What the engine types into a remote terminal's local shell, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    /// The ssh line, typed once the shell has drawn its prompt.
    pub line: String,
    /// What the remote shell prints once it is up.
    pub marker: String,
    /// The agent's launch in the project there, typed once it is up.
    pub command: String,
    pub password: Option<Secret>,
    pub key: Option<KeyFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_prints_as_redacted_wherever_it_is() {
        let remote = Remote {
            line: "ssh -t me@box".into(),
            marker: "M".into(),
            command: "cd /p && claude".into(),
            password: Some(Secret::new("hunter2".into())),
            key: Some(KeyFile::new(Secret::new("-----BEGIN KEY-----".into()))),
        };
        let shown = format!("{remote:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("BEGIN KEY"),
            "{shown}"
        );
        assert!(shown.contains("Secret(<redacted>)"));
    }

    #[test]
    fn writes_the_key_for_its_owner_only_and_removes_it() {
        let key = KeyFile::new(Secret::new("key text".into()));
        key.write().unwrap();
        assert_eq!(std::fs::read_to_string(&key.path).unwrap(), "key text");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Never written over: a name taken is a failed write.
        assert!(key.write().is_err());
        key.remove();
        assert!(!key.path.exists());
        key.remove();
    }
}
