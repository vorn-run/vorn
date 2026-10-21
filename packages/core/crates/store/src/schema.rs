//! The database's schema: one baseline that a new file is created with, and
//! the steps that bring a version 27 or 28 file up to it.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::sql::{json_text, now_iso, num, opt_num, opt_text, random_uuid};
use crate::{Error, Result, Store};

/// The version [`BASELINE_DDL`] is.
pub(crate) const SCHEMA_VERSION: i64 = 29;

/// The oldest version that is upgraded rather than refused.
const UPGRADES_FROM: i64 = 27;

pub(crate) const CONFIG_REVISION_KEY: &str = "config_revision";

/// Every table and index, as a new file gets them.
const BASELINE_DDL: &str = r#"
    CREATE TABLE schema_meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE defaults (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE users (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      role TEXT NOT NULL DEFAULT 'owner',
      created_at TEXT NOT NULL
    );

    -- Only the hash is stored. The plaintext is shown once at creation and is
    -- not recoverable afterwards, so a leaked database yields no usable
    -- credential.
    CREATE TABLE device_tokens (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      token_hash TEXT NOT NULL,
      created_at TEXT NOT NULL,
      last_seen_at TEXT,
      revoked_at TEXT,
      last_address TEXT
    );

    CREATE INDEX idx_device_tokens_user ON device_tokens(user_id);

    CREATE TABLE workspaces (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0,
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE projects (
      name TEXT PRIMARY KEY,
      path TEXT NOT NULL,
      preferred_agents TEXT NOT NULL DEFAULT '[]',
      icon TEXT,
      icon_color TEXT,
      host_ids TEXT,
      workspace_id TEXT NOT NULL DEFAULT 'personal',
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE workflows (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT NOT NULL,
      icon_color TEXT NOT NULL,
      nodes TEXT NOT NULL DEFAULT '[]',
      edges TEXT NOT NULL DEFAULT '[]',
      enabled INTEGER NOT NULL DEFAULT 1,
      last_run_at TEXT,
      last_run_status TEXT,
      stagger_delay_ms INTEGER,
      workspace_id TEXT NOT NULL DEFAULT 'personal'
    );

    CREATE TABLE agent_commands (
      agent_type TEXT PRIMARY KEY,
      command TEXT NOT NULL,
      args TEXT NOT NULL DEFAULT '[]',
      headless_args TEXT,
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE remote_hosts (
      id TEXT PRIMARY KEY,
      label TEXT NOT NULL,
      hostname TEXT NOT NULL,
      user TEXT NOT NULL,
      port INTEGER NOT NULL DEFAULT 22,
      auth_method TEXT DEFAULT 'agent',
      ssh_key_path TEXT,
      credential_id TEXT,
      encrypted_password TEXT,
      ssh_options TEXT,
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE ssh_keys (
      id TEXT PRIMARY KEY,
      label TEXT NOT NULL,
      encrypted_private_key TEXT NOT NULL,
      public_key TEXT,
      certificate TEXT,
      key_type TEXT,
      created_at TEXT NOT NULL
    );

    CREATE TABLE tasks (
      id TEXT PRIMARY KEY,
      project_name TEXT NOT NULL,
      title TEXT NOT NULL,
      description TEXT NOT NULL DEFAULT '',
      status TEXT NOT NULL DEFAULT 'todo',
      "order" INTEGER NOT NULL DEFAULT 0,
      assigned_session_id TEXT,
      assigned_agent TEXT,
      agent_session_id TEXT,
      branch TEXT,
      use_worktree INTEGER DEFAULT 0,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      completed_at TEXT,
      source_connector_id TEXT,
      source_external_url TEXT,
      source_external_id TEXT,
      archived_at TEXT,
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE INDEX idx_tasks_project ON tasks(project_name, status);

    CREATE TABLE sessions (
      id TEXT PRIMARY KEY,
      agent_type TEXT NOT NULL,
      project_name TEXT NOT NULL,
      project_path TEXT NOT NULL,
      status TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      pid INTEGER NOT NULL,
      display_name TEXT,
      branch TEXT,
      worktree_path TEXT,
      is_worktree INTEGER DEFAULT 0,
      remote_host_id TEXT,
      remote_host_label TEXT,
      hook_session_id TEXT,
      status_source TEXT,
      saved_at INTEGER,
      sort_order INTEGER NOT NULL DEFAULT 0,
      worktree_name TEXT,
      agent_session_id TEXT,
      shell_cwd TEXT,
      head_commit TEXT,
      group_id TEXT,
      renamed_by_person INTEGER
    );

    CREATE TABLE session_groups (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0,
      workspace_id TEXT NOT NULL DEFAULT 'personal',
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE archived_sessions (
      id TEXT PRIMARY KEY,
      agent_type TEXT NOT NULL,
      project_name TEXT NOT NULL,
      project_path TEXT NOT NULL,
      display_name TEXT,
      branch TEXT,
      agent_session_id TEXT,
      archived_at INTEGER NOT NULL
    );

    CREATE TABLE session_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id TEXT NOT NULL,
      event_type TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      metadata TEXT
    );

    CREATE INDEX idx_session_events_session ON session_events(session_id, timestamp DESC);
    CREATE INDEX idx_session_events_type ON session_events(event_type, timestamp DESC);

    CREATE TABLE schedule_log (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      workflow_id TEXT NOT NULL,
      workflow_name TEXT NOT NULL,
      executed_at TEXT NOT NULL,
      status TEXT NOT NULL,
      sessions_launched INTEGER NOT NULL DEFAULT 0,
      error TEXT
    );

    CREATE INDEX idx_schedule_log_workflow_id ON schedule_log(workflow_id);

    CREATE TABLE workflow_runs (
      id TEXT PRIMARY KEY,
      workflow_id TEXT NOT NULL,
      started_at TEXT NOT NULL,
      completed_at TEXT,
      status TEXT NOT NULL DEFAULT 'running',
      trigger_task_id TEXT,
      inputs TEXT,
      connector_inbox_id INTEGER,
      connector_item TEXT,
      connector_inbox_lease_token TEXT,
      connector_inbox_disposition TEXT,
      definition TEXT
    );

    CREATE INDEX idx_workflow_runs_workflow ON workflow_runs(workflow_id);
    CREATE INDEX idx_workflow_runs_task ON workflow_runs(trigger_task_id);

    -- A gate node also keeps what it asked (message), its review page token,
    -- which round it is on, what the reviewer wrote, when a person rejected
    -- it, its editable text as the steps produced it and the reviewer's
    -- rewrite of that text.
    CREATE TABLE workflow_run_nodes (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
      node_id TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      started_at TEXT,
      completed_at TEXT,
      session_id TEXT,
      error TEXT,
      logs TEXT,
      task_id TEXT,
      agent_session_id TEXT,
      approved_at TEXT,
      agent_type TEXT,
      project_name TEXT,
      project_path TEXT,
      diagnostics TEXT,
      output TEXT,
      structured_output TEXT,
      iteration INTEGER,
      worktree_path TEXT,
      worktree_name TEXT,
      worktree_origin TEXT,
      waiting_for TEXT,
      message TEXT,
      view_token TEXT,
      round INTEGER,
      feedback TEXT,
      rejected_at TEXT,
      editable_text TEXT,
      edited_text TEXT
    );

    CREATE INDEX idx_workflow_run_nodes_run ON workflow_run_nodes(run_id);
    CREATE INDEX idx_workflow_run_nodes_task ON workflow_run_nodes(task_id);

    CREATE TABLE source_connections (
      id TEXT PRIMARY KEY,
      connector_id TEXT NOT NULL,
      name TEXT NOT NULL,
      filters TEXT NOT NULL DEFAULT '{}',
      sync_interval_minutes INTEGER NOT NULL DEFAULT 5,
      status_mapping TEXT NOT NULL DEFAULT '{}',
      execution_project TEXT,
      last_sync_at TEXT,
      last_sync_error TEXT,
      sync_cursor TEXT,
      created_at TEXT NOT NULL,
      signed_in_as TEXT,
      signed_in_at TEXT
    );

    CREATE TABLE task_source_links (
      task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      connection_id TEXT NOT NULL REFERENCES source_connections(id) ON DELETE CASCADE,
      connector_id TEXT NOT NULL,
      external_id TEXT NOT NULL,
      external_url TEXT NOT NULL,
      source_status_raw TEXT NOT NULL,
      source_updated_at TEXT NOT NULL,
      last_synced_at TEXT NOT NULL,
      conflict_state TEXT NOT NULL DEFAULT 'none',
      PRIMARY KEY (task_id),
      UNIQUE (connection_id, external_id)
    );

    CREATE TABLE connector_poll_state (
      workflow_id TEXT PRIMARY KEY REFERENCES workflows(id) ON DELETE CASCADE,
      connection_id TEXT NOT NULL REFERENCES source_connections(id) ON DELETE CASCADE,
      cursor TEXT,
      last_polled_at TEXT,
      last_error TEXT
    );

    CREATE TABLE connector_inbox (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
      connection_id TEXT NOT NULL REFERENCES source_connections(id) ON DELETE CASCADE,
      connector_id TEXT NOT NULL,
      event_id TEXT NOT NULL,
      event_type TEXT NOT NULL,
      event_timestamp TEXT NOT NULL,
      payload TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      attempts INTEGER NOT NULL DEFAULT 0,
      available_at TEXT NOT NULL,
      lease_until TEXT,
      lease_token TEXT,
      last_error TEXT,
      created_at TEXT NOT NULL,
      processed_at TEXT,
      UNIQUE (workflow_id, connection_id, event_type, event_id)
    );

    CREATE INDEX idx_connector_inbox_ready
      ON connector_inbox(status, available_at, lease_until);
    CREATE INDEX idx_connector_inbox_connection
      ON connector_inbox(connection_id, created_at);

    -- Published artifacts, every version they have had, and the comments
    -- written on them.
    CREATE TABLE artifacts (
      id TEXT PRIMARY KEY,
      kind TEXT NOT NULL,
      title TEXT NOT NULL,
      session_id TEXT,
      project_name TEXT,
      token TEXT NOT NULL,
      latest_version INTEGER NOT NULL DEFAULT 0,
      gate_run_id TEXT,
      gate_node_id TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE INDEX idx_artifacts_session ON artifacts(session_id, updated_at DESC);
    CREATE INDEX idx_artifacts_project ON artifacts(project_name, updated_at DESC);

    CREATE TABLE artifact_versions (
      artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
      version INTEGER NOT NULL,
      author TEXT NOT NULL,
      answers_batch_id TEXT,
      created_at TEXT NOT NULL,
      PRIMARY KEY (artifact_id, version)
    );

    CREATE TABLE artifact_comments (
      id TEXT PRIMARY KEY,
      artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
      version INTEGER NOT NULL,
      anchor TEXT,
      body TEXT NOT NULL,
      state TEXT NOT NULL DEFAULT 'draft',
      batch_id TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      sent_at TEXT
    );

    CREATE INDEX idx_artifact_comments_artifact
      ON artifact_comments(artifact_id, created_at);

    -- The effects of a session's output that have been acted on, by the id
    -- the daemon gives each one.
    CREATE TABLE effect_receipts (
      effect_id TEXT PRIMARY KEY,
      kind TEXT NOT NULL,
      received_at INTEGER NOT NULL
    );

    CREATE INDEX idx_effect_receipts_received ON effect_receipts(received_at);
"#;

/// The columns version 27 had that nothing reads any more.
const DROPPED_IN_28: [(&str, &str); 5] = [
    ("agent_commands", "fallback_command"),
    ("agent_commands", "fallback_args"),
    ("sessions", "claude_session_id"),
    ("tasks", "source_import_mode"),
    ("source_connections", "import_mode"),
];

/// Creates the schema in a new file, upgrades a version 27 file (adding
/// `device_tokens.last_address` when missing) and a version 28 one, and
/// refuses one older than that. A current file is left alone.
pub(crate) fn create(store: &mut Store) -> Result<()> {
    let workspace = store.options()?.default_workspace.clone();
    let owner = store.options()?.owner_name.clone();
    let tx = crate::write_transaction(store.conn_mut())?;
    match version(&tx)? {
        0 => {
            tx.execute_batch(BASELINE_DDL)?;
            tx.execute(
                r#"INSERT INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)"#,
                params![
                    workspace.id,
                    workspace.name,
                    opt_text(workspace.icon.as_deref()),
                    opt_text(workspace.icon_color.as_deref()),
                    num(workspace.order),
                ],
            )?;
            let name = if owner.is_empty() { "owner" } else { &owner };
            tx.execute(
                "INSERT INTO users (id, name, role, created_at) VALUES (?, ?, 'owner', ?)",
                params![random_uuid(), name, now_iso()],
            )?;
            tx.execute(
                "INSERT INTO schema_meta (key, value) VALUES (?, '0')",
                [CONFIG_REVISION_KEY],
            )?;
        }
        v @ UPGRADES_FROM..SCHEMA_VERSION => {
            if v < 28 {
                upgrade_to_28(&tx)?;
            }
            // Settings › Experimental is gone, and its saved switch with it.
            tx.execute("DELETE FROM defaults WHERE key = 'experimental'", [])?;
        }
        v if v >= SCHEMA_VERSION => return Ok(()),
        v => {
            return Err(Error::Refused(format!(
                "this database is at schema version {v}, from a Vorn too old to upgrade from; \
                 open it with a Vorn that writes version {UPGRADES_FROM} first"
            )))
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?)",
        [SCHEMA_VERSION.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Version 27 to 28: adds `device_tokens.last_address` when missing and drops
/// the columns nothing reads.
fn upgrade_to_28(tx: &Connection) -> Result<()> {
    if !columns(tx, "device_tokens")?
        .iter()
        .any(|c| c == "last_address")
    {
        tx.execute_batch("ALTER TABLE device_tokens ADD COLUMN last_address TEXT")?;
    }
    for (table, column) in DROPPED_IN_28 {
        if columns(tx, table)?.iter().any(|c| c == column) {
            tx.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column}"))?;
        }
    }
    Ok(())
}

/// The column names of `table`, empty when it does not exist.
pub(crate) fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>("name"))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

/// The version recorded in `schema_meta`, 0 for a file with none.
pub(crate) fn version(conn: &Connection) -> Result<i64> {
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(0);
    }
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match value {
        None => Ok(0),
        Some(text) => text
            .trim()
            .parse()
            .map_err(|_| Error::Refused(format!("schema version {text:?} is not a number"))),
    }
}

/// Inserts each seeded workflow once: the flag in `defaults` records that it
/// was, so a deleted seed stays deleted and an upgrade gets it exactly once.
pub(crate) fn seed_system_defaults(store: &mut Store) -> Result<()> {
    let seeds = store.options()?.seed_workflows.clone();
    let conn = store.conn_mut();
    for seed in seeds {
        let flag: Option<String> = conn
            .query_row(
                "SELECT value FROM defaults WHERE key = ?",
                [&seed.flag],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(value) = flag {
            if serde_json::from_str::<Value>(&value).ok() == Some(Value::Bool(true)) {
                continue;
            }
        }
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM workflows WHERE id = ?",
                [&seed.workflow.id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            let w = &seed.workflow;
            conn.execute(
                "INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    w.id,
                    w.name,
                    w.icon,
                    w.icon_color,
                    json_text(&w.nodes)?,
                    json_text(&w.edges)?,
                    i64::from(w.enabled),
                    w.last_run_at,
                    w.last_run_status,
                    opt_num(w.stagger_delay_ms),
                    w.workspace_id.as_deref().unwrap_or("personal"),
                ],
            )?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO defaults (key, value) VALUES (?, ?)",
            params![seed.flag, "true"],
        )?;
    }
    Ok(())
}

/// Copies a corrupt file aside and removes it with its WAL and shared-memory
/// files, so a new database can be created in its place. Returns the copy.
pub(crate) fn set_aside_corrupt(path: &Path) -> Result<PathBuf> {
    let stamp = crate::sql::now_iso().replace([':', '.'], "-");
    let backup = PathBuf::from(format!("{}.corrupt-{stamp}", path.display()));
    if path.exists() {
        std::fs::copy(path, &backup)?;
    }
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        if file.exists() {
            std::fs::remove_file(&file)?;
        }
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn tables(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_new_database_is_at_the_latest_version() {
        let store = test_support::store();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let tables = tables(store.conn());
        for table in [
            "artifacts",
            "connector_inbox",
            "effect_receipts",
            "source_connections",
            "task_source_links",
            "users",
            "workflow_run_nodes",
        ] {
            assert!(
                tables.iter().any(|t| t == table),
                "{table} missing: {tables:?}"
            );
        }
        let revision: String = store
            .conn()
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?",
                [CONFIG_REVISION_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revision, "0");
        let workspaces: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))
            .unwrap();
        assert_eq!(workspaces, 1);
    }

    #[test]
    fn opening_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let (store, opened) = Store::open(&path, test_support::options()).unwrap();
        assert_eq!(opened, crate::Opened::Ok);
        let owner: String = store
            .conn()
            .query_row("SELECT id FROM users", [], |row| row.get(0))
            .unwrap();
        drop(store);
        let (store, _) = Store::open(&path, test_support::options()).unwrap();
        let owners: Vec<String> = store
            .conn()
            .prepare("SELECT id FROM users")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(owners, vec![owner]);
    }

    #[test]
    fn a_file_older_than_27_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO schema_meta (key, value) VALUES ('schema_version', '26');",
        )
        .unwrap();
        drop(conn);
        let Err(err) = Store::open(&path, test_support::options()) else {
            panic!("a version 26 file opened");
        };
        assert!(matches!(err, Error::Refused(_)), "{err:?}");
    }

    #[test]
    fn a_corrupt_file_is_set_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        std::fs::write(&path, b"this is not a database, not even close to one").unwrap();
        let (store, opened) = Store::open(&path, test_support::options()).unwrap();
        let crate::Opened::Recovered { backup } = opened else {
            panic!("expected a recovery, got {opened:?}");
        };
        assert!(backup.exists());
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }
}
