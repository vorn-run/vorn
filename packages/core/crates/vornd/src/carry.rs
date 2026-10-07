//! The session records vornd carries from one run to the next, with the
//! Native server switch on.
//!
//! vornd starts with the server and ends with it, so what its registry owns
//! ([`crate::registry::Registry::own_records`]) is written down here, as
//! `sessions.json` beside vornd's history: after every change the registry
//! tells, a moment later so a burst costs one write, and once more as vornd
//! stops. The next vornd reads it back ([`crate::registry::Registry::carry`])
//! before anything connects; the session holder then says which of the
//! terminals it still has, and the rest are offered to resume.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use crate::registry::{now_ms, Carried, SessionRegistry};

/// How long after a change the file is written: changes come in bursts.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);

/// Where the records are written down.
#[derive(Clone, Debug)]
pub struct CarryFile {
    path: PathBuf,
}

impl CarryFile {
    /// `sessions.json` in `dir`, vornd's own directory.
    pub fn in_dir(dir: &Path) -> CarryFile {
        CarryFile {
            path: dir.join("sessions.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the last run wrote down; `None` when nothing was, or when it
    /// cannot be read, which the log says.
    pub fn load(&self) -> Option<Carried> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(e) => {
                warn!(path = %self.path.display(), %e, "could not read the carried session records");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(carried) => Some(carried),
            Err(e) => {
                warn!(path = %self.path.display(), %e, "the carried session records do not read");
                None
            }
        }
    }

    /// Writes `carried` whole, through a file beside it, so a reader never
    /// sees half of it.
    pub fn save(&self, carried: &Carried) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec(carried).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)
    }
}

/// Writes the registry down [`SAVE_AFTER`] each change it tells, until it
/// is dropped.
pub async fn keep(registry: Arc<SessionRegistry>, file: CarryFile) {
    let mut notes = registry.subscribe();
    loop {
        match notes.recv().await {
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
        let settle = tokio::time::sleep(SAVE_AFTER);
        tokio::pin!(settle);
        loop {
            tokio::select! {
                () = &mut settle => break,
                more = notes.recv() => {
                    if matches!(more, Err(RecvError::Closed)) {
                        break;
                    }
                }
            }
        }
        save_now(&registry, &file).await;
    }
}

/// Writes the registry down now, off the runtime's threads, while vornd
/// owns the records.
pub async fn save_now(registry: &SessionRegistry, file: &CarryFile) {
    let Some(carried) = registry.carried(now_ms()) else {
        return;
    };
    let file = file.clone();
    let _ = tokio::task::spawn_blocking(move || {
        tracing::debug!(
            terminals = carried.terminals.len(),
            headless = carried.headless.len(),
            offered = carried.restored.len(),
            "session records written down"
        );
        if let Err(e) = file.save(&carried) {
            warn!(path = %file.path().display(), %e, "could not write the session records down");
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Gen, Held, Kind, Registry, TerminalSession};
    use serde_json::json;

    fn shell(id: &str) -> TerminalSession {
        serde_json::from_value(json!({
            "id": id, "agentType": "shell", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 1, "pid": 3, "displayName": "Build",
            "groupId": "g",
        }))
        .unwrap()
    }

    #[test]
    fn writes_the_records_down_and_reads_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = CarryFile::in_dir(&dir.path().join("vornd"));
        assert!(file.load().is_none());

        let mut r = Registry::new(Gen(1));
        r.own_records();
        r.decide_statuses();
        r.apply(
            crate::registry::Change::try_from(&json!({
                "op": "snapshot", "terminals": [shell("a"), shell("b")], "headless": [],
                "order": ["b", "a"],
            }))
            .unwrap(),
        );
        let carried = r.carried(1_000);
        file.save(&carried).unwrap();
        assert_eq!(file.load(), Some(carried.clone()));
        assert!(carried.terminals.iter().all(|t| t.saved_at == Some(1_000)));
        // Nothing of the registry's own revisions goes with a record.
        assert!(carried.terminals.iter().all(|t| t.rev.is_none()));

        // Read back: offered, then listed in their old order once the holder has them.
        let mut next = Registry::new(Gen(2));
        next.own_records();
        next.decide_statuses();
        assert_eq!(next.carry(file.load().unwrap(), 2_000), (2, 0));
        assert_eq!(next.restored().len(), 2);
        assert!(next.terminals().is_empty());
        let held = |id: &str| Held {
            id: id.to_owned(),
            kind: Kind::Terminal,
            pid: 7,
            epoch: 1,
        };
        assert!(!next.adopt(&held("a")).is_empty());
        assert!(!next.adopt(&held("b")).is_empty());
        assert_eq!(
            next.terminals()
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(next.restored().is_empty());
    }

    #[test]
    fn a_file_that_does_not_read_is_nothing_to_carry() {
        let dir = tempfile::tempdir().unwrap();
        let file = CarryFile::in_dir(dir.path());
        fs::write(file.path(), b"{not json").unwrap();
        assert!(file.load().is_none());
    }
}
