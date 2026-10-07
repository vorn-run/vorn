//! What the worktree manager reads of the configuration, by a second process.
//!
//! The projects with their names, which hosts they are on, and the person's
//! retention preferences, read from the server's file as
//! [`crate::ProjectHosts`] reads the projects: read-only, fresh on every call,
//! creating nothing.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

use crate::hosts::{ProjectHost, ProjectHosts};
use crate::Result;

/// The projects, by name, and `defaults.worktreeRetention`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorktreeSettings {
    /// Each project's name, in the order of `hosts.projects`.
    pub names: Vec<String>,
    pub hosts: ProjectHosts,
    /// As stored; `None` when the person never set one.
    pub retention: Option<Value>,
}

impl WorktreeSettings {
    /// Reads them from the database at `path`. `None` when there is no such
    /// file or the server has not created its tables yet; an error when a
    /// row does not parse, so the caller cannot mistake it for no projects.
    pub fn read(path: &Path) -> Result<Option<WorktreeSettings>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('projects', 'remote_hosts', 'defaults')",
            [],
            |row| row.get(0),
        )?;
        if tables < 3 {
            return Ok(None);
        }
        let mut names = Vec::new();
        let mut projects = Vec::new();
        let mut stmt = conn.prepare("SELECT name, path, host_ids FROM projects ORDER BY rowid")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let host_ids = match row.get::<_, Option<String>>(2)? {
                Some(text) => Some(serde_json::from_str::<Vec<String>>(&text)?),
                None => None,
            };
            names.push(row.get(0)?);
            projects.push(ProjectHost {
                path: row.get(1)?,
                host_ids,
            });
        }
        let remote_hosts = conn
            .prepare("SELECT id FROM remote_hosts")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let retention = conn
            .query_row(
                "SELECT value FROM defaults WHERE key = 'worktreeRetention'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|text| serde_json::from_str::<Value>(&text))
            .transpose()?
            .filter(|v| !v.is_null());
        Ok(Some(WorktreeSettings {
            names,
            hosts: ProjectHosts {
                projects,
                remote_hosts,
            },
            retention,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Placement;

    #[test]
    fn reads_the_projects_by_name_and_the_retention() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        assert_eq!(WorktreeSettings::read(&path).unwrap(), None);
        let (store, _) =
            crate::Store::open(&path, crate::test_support::options()).expect("a store opens");
        let read = WorktreeSettings::read(&path).unwrap().unwrap();
        assert!(read.names.is_empty() && read.retention.is_none());
        store
            .conn()
            .execute_batch(
                r#"INSERT INTO projects (name, path, host_ids) VALUES ('a', '/src/a', NULL);
                   INSERT INTO projects (name, path, host_ids) VALUES ('b', '/src/b', '["local","h1"]');
                   INSERT INTO remote_hosts (id, label, hostname, user) VALUES ('h1', 'H', 'h', 'u');
                   INSERT INTO defaults VALUES ('worktreeRetention', '{"idleDaysThreshold":3}');"#,
            )
            .unwrap();
        let read = WorktreeSettings::read(&path).unwrap().unwrap();
        assert_eq!(read.names, ["a", "b"]);
        assert_eq!(
            read.hosts.placement(&read.hosts.projects[1]),
            Placement::Remote("h1".into())
        );
        assert_eq!(
            read.retention,
            Some(serde_json::json!({ "idleDaysThreshold": 3 }))
        );
        store
            .conn()
            .execute(
                "UPDATE projects SET host_ids = 'not json' WHERE name = 'a'",
                [],
            )
            .unwrap();
        assert!(WorktreeSettings::read(&path).is_err());
    }
}
