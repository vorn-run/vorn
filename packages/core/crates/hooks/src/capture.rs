//! The conversation an agent took, read from its own database, for agents
//! that can be sent back to a conversation but cannot be told one when they
//! start: the adapter's `[agent.resume.capture]` query, the newest
//! conversation in the session's directory.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// A directory as the query compares it: lower case, forward slashes, no
/// trailing slash but for the root.
pub fn comparable(cwd: &str) -> String {
    let lowered = cwd.replace('\\', "/").to_lowercase();
    if lowered == "/" {
        return lowered;
    }
    lowered.trim_end_matches('/').to_owned()
}

/// The conversation `query` finds for `cwd` in `db` (`?1` is the directory,
/// as [`comparable`] writes it); `None` when there is no database or no
/// conversation there.
pub fn capture(query: &str, cwd: &str, db: &Path) -> Option<String> {
    if !db.is_file() {
        return None;
    }
    let read = || -> rusqlite::Result<Option<String>> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.query_row(query, [comparable(cwd)], |row| row.get(0))
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

    const QUERY: &str = "SELECT id FROM threads WHERE archived = 0 \
        AND rtrim(lower(replace(cwd, '\\', '/')), '/') = ?1 \
        ORDER BY updated_at DESC LIMIT 1";

    #[test]
    fn finds_the_newest_unarchived_conversation_in_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT, cwd TEXT, archived INT, updated_at INT);
             INSERT INTO threads VALUES ('old', 'C:\\Work\\App\\', 0, 1);
             INSERT INTO threads VALUES ('new', 'c:/work/app', 0, 2);
             INSERT INTO threads VALUES ('gone', 'c:/work/app', 1, 3);",
        )
        .unwrap();
        assert_eq!(capture(QUERY, "C:\\work\\app", &db).as_deref(), Some("new"));
        assert_eq!(capture(QUERY, "/elsewhere", &db), None);
        assert_eq!(capture(QUERY, "/x", &dir.path().join("absent.db")), None);
        assert_eq!(comparable("/"), "/");
    }
}
