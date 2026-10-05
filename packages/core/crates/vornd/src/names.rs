//! The names sessions go by outside vornd.
//!
//! sessiond numbers the sessions it starts. The app has its own id for each
//! terminal, which its panes, its database and every client already key by,
//! and which a resumed session keeps across a new process. A session the app
//! starts through vornd is therefore given the app's id as its name, and
//! that name is the only id anything outside vornd sees: clients attach by
//! it, frames and effects carry it, and the engine's actors run under it.
//! sessiond's own id is used only on the wire to sessiond, and the engine's
//! driver translates at that edge.
//!
//! The table is kept in a file beside vornd's history, so a vornd that
//! restarts serves each session under the same name before the app has
//! reconnected. A session with no name goes by sessiond's id.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use tracing::warn;

/// The longest name: what a bytes frame can carry as its id.
pub const MAX_NAME: usize = 128;

/// Why a name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Empty, too long, or with a character outside `A-Z a-z 0-9 . _ : -`.
    Malformed,
    /// Another session goes by it.
    Taken,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Malformed => write!(
                f,
                "a session name is 1 to {MAX_NAME} of A-Z a-z 0-9 . _ : -"
            ),
            Refused::Taken => f.write_str("another session goes by that name"),
        }
    }
}

/// sessiond's ids and the names they go by, both ways.
#[derive(Debug, Default)]
pub struct Names {
    /// sessiond's id to the session's name.
    by_held: HashMap<String, String>,
    /// The name to sessiond's id.
    by_name: HashMap<String, String>,
    /// Where the table is kept; none keeps it in memory only.
    file: Option<PathBuf>,
}

/// Whether `name` may name a session.
pub fn well_formed(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

impl Names {
    /// The table kept in `file`, as the last vornd left it. A file that is
    /// missing or unreadable starts an empty table: sessions then go by
    /// sessiond's ids until they are named again.
    pub fn load(file: Option<PathBuf>) -> Names {
        let mut names = Names {
            file,
            ..Names::default()
        };
        let Some(path) = names.file.clone() else {
            return names;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return names,
            Err(e) => {
                warn!(file = %path.display(), %e, "session names unreadable; starting without");
                return names;
            }
        };
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) else {
            warn!(file = %path.display(), "session names malformed; starting without");
            return names;
        };
        for (held, name) in map {
            if let Some(name) = name.as_str() {
                if well_formed(name) && !names.by_name.contains_key(name) {
                    names.by_name.insert(name.to_owned(), held.clone());
                    names.by_held.insert(held, name.to_owned());
                }
            }
        }
        names
    }

    /// The id session `held` goes by outside vornd.
    pub fn public<'a>(&'a self, held: &'a str) -> &'a str {
        self.by_held.get(held).map_or(held, String::as_str)
    }

    /// sessiond's id for the session that goes by `public`.
    pub fn held<'a>(&'a self, public: &'a str) -> &'a str {
        self.by_name.get(public).map_or(public, String::as_str)
    }

    /// Whether a new session may go by `name`: well formed, and neither
    /// another session's name nor an unnamed session's id in `unnamed`.
    pub fn check(&self, name: &str, unnamed: &dyn Fn(&str) -> bool) -> Result<(), Refused> {
        if !well_formed(name) {
            return Err(Refused::Malformed);
        }
        if self.by_name.contains_key(name) || (!self.by_held.contains_key(name) && unnamed(name)) {
            return Err(Refused::Taken);
        }
        Ok(())
    }

    /// Session `held` goes by `name` from now on.
    pub fn name(&mut self, held: &str, name: &str) {
        if let Some(old) = self.by_held.insert(held.to_owned(), name.to_owned()) {
            self.by_name.remove(&old);
        }
        self.by_name.insert(name.to_owned(), held.to_owned());
        self.save();
    }

    /// Session `held` is gone, and its name with it.
    pub fn forget(&mut self, held: &str) {
        if let Some(name) = self.by_held.remove(held) {
            self.by_name.remove(&name);
            self.save();
        }
    }

    /// Forgets the names of every session sessiond no longer holds.
    pub fn keep_only<'a>(&mut self, held: impl IntoIterator<Item = &'a str>) {
        let keep: std::collections::HashSet<&str> = held.into_iter().collect();
        let gone: Vec<String> = self
            .by_held
            .keys()
            .filter(|h| !keep.contains(h.as_str()))
            .cloned()
            .collect();
        if gone.is_empty() {
            return;
        }
        for h in gone {
            if let Some(name) = self.by_held.remove(&h) {
                self.by_name.remove(&name);
            }
        }
        self.save();
    }

    /// Writes the table where it is kept, whole, through a file of its own
    /// renamed over the last, so a crash leaves one table or the other.
    fn save(&self) {
        let Some(path) = &self.file else { return };
        let map: Map<String, Value> = self
            .by_held
            .iter()
            .map(|(h, n)| (h.clone(), Value::String(n.clone())))
            .collect();
        if let Err(e) = write_atomically(path, Value::Object(map).to_string().as_bytes()) {
            warn!(file = %path.display(), %e, "could not keep session names");
        }
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn a_session_goes_by_its_name_or_its_own_id() {
        let mut n = Names::load(None);
        assert_eq!(n.public("0a-1"), "0a-1");
        n.name("0a-1", "pane-7");
        assert_eq!(n.public("0a-1"), "pane-7");
        assert_eq!(n.held("pane-7"), "0a-1");
        // An unnamed session's id goes through both ways unchanged.
        assert_eq!(n.held("0a-2"), "0a-2");
        n.forget("0a-1");
        assert_eq!(n.public("0a-1"), "0a-1");
        assert_eq!(n.held("pane-7"), "pane-7");
    }

    #[test]
    fn refuses_a_name_that_is_taken_or_malformed() {
        let mut n = Names::load(None);
        n.name("0a-1", "pane-7");
        assert_eq!(n.check("pane-7", &none), Err(Refused::Taken));
        // An unnamed session's own id is taken too.
        assert_eq!(n.check("0a-2", &|id| id == "0a-2"), Err(Refused::Taken));
        for bad in ["", "has space", "slash/", &"x".repeat(MAX_NAME + 1)] {
            assert_eq!(n.check(bad, &none), Err(Refused::Malformed), "{bad:?}");
        }
        assert_eq!(
            n.check("5f0c8a52-6b1e-4f4e-9d2a-1b2c3d4e5f60", &none),
            Ok(())
        );
    }

    #[test]
    fn the_table_outlives_the_process_and_forgets_what_sessiond_let_go() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vornd").join("names.json");
        {
            let mut n = Names::load(Some(file.clone()));
            n.name("0a-1", "pane-1");
            n.name("0a-2", "pane-2");
        }
        let mut n = Names::load(Some(file.clone()));
        assert_eq!(n.public("0a-2"), "pane-2");
        n.keep_only(["0a-2"]);
        let n = Names::load(Some(file));
        assert_eq!(n.public("0a-1"), "0a-1");
        assert_eq!(n.public("0a-2"), "pane-2");
    }

    #[test]
    fn a_damaged_table_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("names.json");
        std::fs::write(&file, "{not json").unwrap();
        let n = Names::load(Some(file));
        assert_eq!(n.public("0a-1"), "0a-1");
    }
}
