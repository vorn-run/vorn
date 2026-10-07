//! Which projects are on which machine, read by a second process.
//!
//! A path in a call names a project on this machine or one on a remote host,
//! and only the server reaches remote hosts. A process standing beside the
//! server (vornd) reads the same rows to tell the two apart, from the same
//! file, without opening it for writing: no schema created, no migration, no
//! seeding, all of which stay the server's. A file it cannot read, or rows it
//! cannot make sense of, are an answer of "cannot tell", never "local".

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use vorn_protocol::RemoteHost;

use crate::config::row_to_remote_host;
use crate::Result;

/// The id every project has for this machine.
const LOCAL: &str = "local";

/// One project's path and the hosts it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectHost {
    pub path: String,
    /// `host_ids` as stored; `None` is this machine only.
    pub host_ids: Option<Vec<String>>,
}

/// The projects, in the order the server lists them, and the remote hosts
/// that exist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectHosts {
    pub projects: Vec<ProjectHost>,
    pub remote_hosts: Vec<String>,
}

/// Where a path's project is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// No project, or a project on this machine.
    Local,
    /// A project on this remote host.
    Remote(String),
}

impl ProjectHosts {
    /// Reads the rows from the database at `path`. `None` when there is no
    /// such file or it has no projects yet; an error when a row does not
    /// parse, so the caller cannot mistake it for a local project.
    pub fn read(path: &Path) -> Result<Option<ProjectHosts>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('projects', 'remote_hosts')",
            [],
            |row| row.get(0),
        )?;
        if tables < 2 {
            return Ok(None);
        }
        let mut projects = Vec::new();
        let mut stmt = conn.prepare("SELECT path, host_ids FROM projects ORDER BY rowid")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let host_ids = match row.get::<_, Option<String>>(1)? {
                Some(text) => Some(serde_json::from_str::<Vec<String>>(&text)?),
                None => None,
            };
            projects.push(ProjectHost {
                path: row.get(0)?,
                host_ids,
            });
        }
        let mut stmt = conn.prepare("SELECT id FROM remote_hosts")?;
        let remote_hosts = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(Some(ProjectHosts {
            projects,
            remote_hosts,
        }))
    }

    /// Where the project at exactly `project_path` is, as the server's
    /// `resolveRemoteHost` decides it.
    pub fn for_project(&self, project_path: &str) -> Placement {
        self.projects
            .iter()
            .find(|p| p.path == project_path)
            .map_or(Placement::Local, |p| self.placement(p))
    }

    /// Where the project a path is in, or a worktree of it, is: the first
    /// project the path is, sits under, or has a worktree under, as the
    /// server's `resolveRemoteHostByPath` decides it.
    pub fn for_path(&self, any_path: &str) -> Placement {
        for project in &self.projects {
            let path = project.path.as_str();
            let under = any_path
                .strip_prefix(path)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
            if under {
                return self.placement(project);
            }
            let worktrees = format!("{}/.vorn-worktrees/", parent_of(path));
            if any_path.starts_with(&worktrees) {
                return self.placement(project);
            }
        }
        Placement::Local
    }

    /// The first host other than this machine, when it is one that exists.
    pub fn placement(&self, project: &ProjectHost) -> Placement {
        let remote = project
            .host_ids
            .as_deref()
            .filter(|ids| !ids.is_empty())
            .and_then(|ids| ids.iter().find(|id| *id != LOCAL));
        match remote {
            Some(id) if self.remote_hosts.contains(id) => Placement::Remote(id.clone()),
            _ => Placement::Local,
        }
    }
}

/// Remote host `id` as the server keeps it, read beside it; `None` when there is no such file, table or host.
pub fn remote_host(path: &Path, id: &str) -> Result<Option<RemoteHost>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'remote_hosts'",
        [],
        |row| row.get(0),
    )?;
    if tables == 0 {
        return Ok(None);
    }
    let mut stmt = conn.prepare("SELECT * FROM remote_hosts WHERE id = ?1")?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_remote_host(row)?)),
        None => Ok(None),
    }
}

/// `path` without its last `/segment`, or all of it when it ends in `/`:
/// the server's `path.replace(/\/[^/]+$/, '')`.
fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(at) if at + 1 < path.len() => &path[..at],
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> ProjectHosts {
        ProjectHosts {
            projects: vec![
                ProjectHost {
                    path: "/src/app".into(),
                    host_ids: None,
                },
                ProjectHost {
                    path: "/srv/far".into(),
                    host_ids: Some(vec!["local".into(), "box".into()]),
                },
                ProjectHost {
                    path: "/srv/gone".into(),
                    host_ids: Some(vec!["deleted-host".into()]),
                },
                ProjectHost {
                    path: "/srv/far".into(),
                    host_ids: None,
                },
            ],
            remote_hosts: vec!["box".into()],
        }
    }

    #[test]
    fn a_project_is_remote_only_on_a_host_that_exists() {
        let h = hosts();
        assert_eq!(h.for_project("/src/app"), Placement::Local);
        // The first project with the path decides.
        assert_eq!(h.for_project("/srv/far"), Placement::Remote("box".into()));
        assert_eq!(h.for_project("/srv/gone"), Placement::Local);
        assert_eq!(h.for_project("/nowhere"), Placement::Local);
    }

    #[test]
    fn a_path_belongs_to_the_project_it_is_in_or_has_a_worktree_of() {
        let h = hosts();
        assert_eq!(
            h.for_path("/srv/far/src/x"),
            Placement::Remote("box".into())
        );
        assert_eq!(
            h.for_path("/srv/.vorn-worktrees/far/wt-1234abcd"),
            // `/src/app`'s worktrees live under `/src`, so `/srv` is far's.
            Placement::Remote("box".into())
        );
        assert_eq!(h.for_path("/srv/farther"), Placement::Local);
        assert_eq!(h.for_path("/src/app"), Placement::Local);
        assert_eq!(h.for_path("/elsewhere"), Placement::Local);
    }

    #[test]
    fn the_parent_is_the_path_less_its_last_segment() {
        assert_eq!(parent_of("/a/b"), "/a");
        assert_eq!(parent_of("/a/b/"), "/a/b/");
        assert_eq!(parent_of("/a"), "");
        assert_eq!(parent_of("a"), "a");
    }

    #[test]
    fn reads_the_rows_from_a_file_another_store_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        assert_eq!(ProjectHosts::read(&path).unwrap(), None);
        let (store, _) =
            crate::Store::open(&path, crate::test_support::options()).expect("a store opens");
        store
            .conn()
            .execute_batch(
                "INSERT INTO projects (name, path, host_ids) VALUES ('a', '/src/a', NULL);
                 INSERT INTO projects (name, path, host_ids) VALUES ('b', '/src/b', '[\"local\",\"h1\"]');
                 INSERT INTO remote_hosts (id, label, hostname, user) VALUES ('h1', 'H', 'h', 'u');",
            )
            .unwrap();
        let read = ProjectHosts::read(&path).unwrap().unwrap();
        assert_eq!(read.for_project("/src/a"), Placement::Local);
        assert_eq!(read.for_project("/src/b"), Placement::Remote("h1".into()));

        store
            .conn()
            .execute(
                "UPDATE projects SET host_ids = 'not json' WHERE name = 'a'",
                [],
            )
            .unwrap();
        assert!(ProjectHosts::read(&path).is_err());
    }

    #[test]
    fn reads_one_remote_host_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        assert_eq!(remote_host(&path, "h1").unwrap(), None);
        let (store, _) =
            crate::Store::open(&path, crate::test_support::options()).expect("a store opens");
        store
            .conn()
            .execute_batch(
                "INSERT INTO remote_hosts (id, label, hostname, user, port, auth_method, ssh_options)
                 VALUES ('h1', 'Box', 'box.example', 'me', 2222, 'password', '-A');",
            )
            .unwrap();
        let host = remote_host(&path, "h1").unwrap().expect("the host");
        assert_eq!(
            (host.label.as_str(), host.hostname.as_str(), host.user.as_str()),
            ("Box", "box.example", "me")
        );
        assert_eq!(host.port, 2222.0);
        assert_eq!(host.auth_method.map(|m| m.0).as_deref(), Some("password"));
        assert_eq!(host.ssh_options.as_deref(), Some("-A"));
        assert_eq!(remote_host(&path, "h2").unwrap(), None);
    }
}
