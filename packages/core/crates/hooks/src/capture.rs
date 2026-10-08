//! The conversation an agent took, read from its own database, for agents
//! that can be sent back to a conversation but cannot be told one when they
//! start (`captureAgentSessionId`): Codex and OpenCode. The newest
//! unarchived conversation in the session's directory is the one.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// The agent CLIs whose database is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Codex,
    OpenCode,
}

impl Agent {
    pub fn from_id(id: &str) -> Option<Agent> {
        match id {
            "codex" => Some(Agent::Codex),
            "opencode" => Some(Agent::OpenCode),
            _ => None,
        }
    }

    /// Where the agent keeps its database, for a user whose home is `home`.
    pub fn database(self, home: &Path, data_home: Option<&Path>) -> PathBuf {
        match self {
            Agent::Codex => home.join(".codex").join("state_5.sqlite"),
            Agent::OpenCode => data_home
                .map_or_else(|| home.join(".local").join("share"), Path::to_path_buf)
                .join("opencode")
                .join("opencode.db"),
        }
    }

    fn query(self) -> &'static str {
        match self {
            Agent::Codex => {
                "SELECT id FROM threads WHERE archived = 0 \
                 AND rtrim(lower(replace(cwd, '\\', '/')), '/') = ?1 \
                 ORDER BY updated_at DESC LIMIT 1"
            }
            Agent::OpenCode => {
                "SELECT id FROM session WHERE time_archived IS NULL \
                 AND rtrim(lower(replace(directory, '\\', '/')), '/') = ?1 \
                 ORDER BY time_updated DESC LIMIT 1"
            }
        }
    }
}

/// A directory as the query compares it: lower case, forward slashes, no
/// trailing slash but for the root.
fn comparable(cwd: &str) -> String {
    let lowered = cwd.replace('\\', "/").to_lowercase();
    if lowered == "/" {
        return lowered;
    }
    lowered.trim_end_matches('/').to_owned()
}

/// The newest conversation `agent` has in `cwd`, read from `db`; `None`
/// when there is no database or no conversation there.
pub fn capture(agent: Agent, cwd: &str, db: &Path) -> Option<String> {
    if !db.is_file() {
        return None;
    }
    let read = || -> rusqlite::Result<Option<String>> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.query_row(agent.query(), [comparable(cwd)], |row| row.get(0))
            .optional()
    };
    match read() {
        Ok(found) => found,
        Err(err) => {
            tracing::warn!(%err, db = %db.display(), "could not read an agent's database");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_newest_unarchived_conversation_in_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let db = Agent::Codex.database(home, None);
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT, cwd TEXT, archived INT, updated_at INT);
             INSERT INTO threads VALUES ('old', 'C:\\Work\\App\\', 0, 1);
             INSERT INTO threads VALUES ('new', 'c:/work/app', 0, 2);
             INSERT INTO threads VALUES ('gone', 'c:/work/app', 1, 3);",
        )
        .unwrap();
        assert_eq!(
            capture(Agent::Codex, "C:\\work\\app", &db).as_deref(),
            Some("new")
        );
        assert_eq!(capture(Agent::Codex, "/elsewhere", &db), None);
        assert_eq!(
            capture(Agent::OpenCode, "/x", &Agent::OpenCode.database(home, None)),
            None
        );
        assert_eq!(
            Agent::OpenCode.database(home, Some(Path::new("/d"))),
            PathBuf::from("/d/opencode/opencode.db")
        );
        assert_eq!(Agent::from_id("claude"), None);
        assert_eq!(comparable("/"), "/");
    }
}
