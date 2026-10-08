//! The schema and its migrations, as `database.ts` creates and upgrades them.
//!
//! Same DDL, same version numbers in `schema_meta`, same guards around every
//! `ALTER`, so a file either implementation has opened is current for the
//! other, and opening it again changes nothing.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::sql::{json_text, now_iso, num, opt_num, opt_text, random_uuid};
use crate::{Result, Store};

/// The tables `saveConfig` rewrites from a whole snapshot, and so the ones
/// whose rows carry the revision that wrote them.
pub(crate) const REVISIONED_TABLES: [&str; 6] = [
    "projects",
    "tasks",
    "workspaces",
    "session_groups",
    "remote_hosts",
    "agent_commands",
];

pub(crate) const CONFIG_REVISION_KEY: &str = "config_revision";

const BASE_DDL: &str = r#"
    CREATE TABLE IF NOT EXISTS schema_meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS defaults (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      role TEXT NOT NULL DEFAULT 'owner',
      created_at TEXT NOT NULL
    );

    -- Only the hash is stored. The plaintext is shown once at creation and is
    -- not recoverable afterwards, so a leaked database yields no usable
    -- credential.
    CREATE TABLE IF NOT EXISTS device_tokens (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      token_hash TEXT NOT NULL,
      created_at TEXT NOT NULL,
      last_seen_at TEXT,
      revoked_at TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_device_tokens_user
      ON device_tokens(user_id);

    CREATE TABLE IF NOT EXISTS projects (
      name TEXT PRIMARY KEY,
      path TEXT NOT NULL,
      preferred_agents TEXT NOT NULL DEFAULT '[]',
      icon TEXT,
      icon_color TEXT,
      host_ids TEXT
    );

    CREATE TABLE IF NOT EXISTS workflows (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT NOT NULL,
      icon_color TEXT NOT NULL,
      nodes TEXT NOT NULL DEFAULT '[]',
      edges TEXT NOT NULL DEFAULT '[]',
      enabled INTEGER NOT NULL DEFAULT 1,
      last_run_at TEXT,
      last_run_status TEXT,
      stagger_delay_ms INTEGER
    );

    CREATE TABLE IF NOT EXISTS agent_commands (
      agent_type TEXT PRIMARY KEY,
      command TEXT NOT NULL,
      args TEXT NOT NULL DEFAULT '[]',
      headless_args TEXT,
      fallback_command TEXT,
      fallback_args TEXT
    );

    CREATE TABLE IF NOT EXISTS remote_hosts (
      id TEXT PRIMARY KEY,
      label TEXT NOT NULL,
      hostname TEXT NOT NULL,
      user TEXT NOT NULL,
      port INTEGER NOT NULL DEFAULT 22,
      auth_method TEXT DEFAULT 'agent',
      ssh_key_path TEXT,
      credential_id TEXT,
      encrypted_password TEXT,
      ssh_options TEXT
    );

    CREATE TABLE IF NOT EXISTS ssh_keys (
      id TEXT PRIMARY KEY,
      label TEXT NOT NULL,
      encrypted_private_key TEXT NOT NULL,
      public_key TEXT,
      certificate TEXT,
      key_type TEXT,
      created_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS tasks (
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
      archived_at TEXT
    );

    CREATE TABLE IF NOT EXISTS sessions (
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
      renamed_by_person INTEGER
    );

    CREATE TABLE IF NOT EXISTS schedule_log (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      workflow_id TEXT NOT NULL,
      workflow_name TEXT NOT NULL,
      executed_at TEXT NOT NULL,
      status TEXT NOT NULL,
      sessions_launched INTEGER NOT NULL DEFAULT 0,
      error TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_schedule_log_workflow_id ON schedule_log(workflow_id);
    CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project_name, status);

    CREATE TABLE IF NOT EXISTS workspaces (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS session_groups (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0,
      workspace_id TEXT NOT NULL DEFAULT 'personal',
      row_revision INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS workflow_runs (
      id TEXT PRIMARY KEY,
      workflow_id TEXT NOT NULL,
      started_at TEXT NOT NULL,
      completed_at TEXT,
      status TEXT NOT NULL DEFAULT 'running',
      trigger_task_id TEXT,
      inputs TEXT,
      connector_item TEXT,
      connector_inbox_id INTEGER,
      connector_inbox_lease_token TEXT,
      connector_inbox_disposition TEXT,
      definition TEXT
    );

    CREATE TABLE IF NOT EXISTS workflow_run_nodes (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      run_id TEXT NOT NULL,
      node_id TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      started_at TEXT,
      completed_at TEXT,
      session_id TEXT,
      error TEXT,
      logs TEXT,
      task_id TEXT,
      agent_session_id TEXT,
      agent_type TEXT,
      project_name TEXT,
      project_path TEXT,
      approved_at TEXT,
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
      edited_text TEXT,
      FOREIGN KEY (run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
    );

    CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow ON workflow_runs(workflow_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_runs_task ON workflow_runs(trigger_task_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_run_nodes_run ON workflow_run_nodes(run_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_run_nodes_task ON workflow_run_nodes(task_id);

    CREATE TABLE IF NOT EXISTS session_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id TEXT NOT NULL,
      event_type TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      metadata TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id, timestamp DESC);
    CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(event_type, timestamp DESC);

    CREATE TABLE IF NOT EXISTS connector_poll_state (
      workflow_id TEXT PRIMARY KEY REFERENCES workflows(id) ON DELETE CASCADE,
      connection_id TEXT NOT NULL REFERENCES source_connections(id) ON DELETE CASCADE,
      cursor TEXT,
      last_polled_at TEXT,
      last_error TEXT
    );

    CREATE TABLE IF NOT EXISTS connector_inbox (
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

    CREATE INDEX IF NOT EXISTS idx_connector_inbox_ready
      ON connector_inbox(status, available_at, lease_until);
    CREATE INDEX IF NOT EXISTS idx_connector_inbox_connection
      ON connector_inbox(connection_id, created_at);
"#;

/// The effects of a session's output the server has acted on, by the id the
/// native daemon gives each one.
const EFFECT_RECEIPTS_DDL: &str = r#"
  CREATE TABLE IF NOT EXISTS effect_receipts (
    effect_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    received_at INTEGER NOT NULL
  );

  CREATE INDEX IF NOT EXISTS idx_effect_receipts_received ON effect_receipts(received_at);
"#;

/// Published artifacts, every version they have had, and the comments written on them.
const ARTIFACT_DDL: &str = r#"
  CREATE TABLE IF NOT EXISTS artifacts (
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

  CREATE INDEX IF NOT EXISTS idx_artifacts_session ON artifacts(session_id, updated_at DESC);
  CREATE INDEX IF NOT EXISTS idx_artifacts_project ON artifacts(project_name, updated_at DESC);

  CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    author TEXT NOT NULL,
    answers_batch_id TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (artifact_id, version)
  );

  CREATE TABLE IF NOT EXISTS artifact_comments (
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

  CREATE INDEX IF NOT EXISTS idx_artifact_comments_artifact
    ON artifact_comments(artifact_id, created_at);
"#;

/// What a gate asked, its review page token, which round it is on, what the
/// reviewer wrote, and when a person rejected it.
const GATE_COLUMNS: [(&str, &str); 5] = [
    ("message", "TEXT"),
    ("view_token", "TEXT"),
    ("round", "INTEGER"),
    ("feedback", "TEXT"),
    ("rejected_at", "TEXT"),
];

/// A gate's editable text as the steps produced it, and the reviewer's rewrite of it.
const GATE_EDIT_COLUMNS: [(&str, &str); 2] = [("editable_text", "TEXT"), ("edited_text", "TEXT")];

/// Creates what is missing, runs every migration the file has not had, and
/// repairs columns a migration should have added.
pub(crate) fn create(store: &mut Store) -> Result<()> {
    let conn = store.conn_mut();

    // An old-format workflows table (it had `actions`) is set aside, not dropped.
    if columns(conn, "workflows")?.iter().any(|c| c == "actions") {
        conn.execute_batch("ALTER TABLE workflows RENAME TO workflows_backup_old_format")?;
    }
    conn.execute_batch(BASE_DDL)?;
    conn.execute_batch(ARTIFACT_DDL)?;
    conn.execute_batch(EFFECT_RECEIPTS_DDL)?;

    let workspace = store.options()?.default_workspace.clone();
    let owner = store.options()?.owner_name.clone();
    migrate(store.conn_mut(), &workspace, &owner)?;
    verify(store.conn_mut())?;
    seed_legacy_connector_poll_state(store.conn_mut())?;
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

fn has(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(columns(conn, table)?.iter().any(|c| c == column))
}

/// The version recorded in `schema_meta`, 0 when none is.
pub(crate) fn version(conn: &Connection) -> Result<i64> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    // parseInt: leading digits, else NaN, which compares false with every
    // `version < n` and so runs no migration.
    Ok(match value {
        None => 0,
        Some(text) => parse_int(&text).unwrap_or(i64::MAX),
    })
}

/// JavaScript `parseInt(text, 10)`: the leading integer, `None` for NaN.
fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse::<i64>().ok().map(|n| sign * n)
}

fn set_version(conn: &Connection, v: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', ?)",
        [v.to_string()],
    )?;
    Ok(())
}

/// Runs `body` in one transaction and records version `v` with it.
fn step(conn: &mut Connection, v: i64, body: impl FnOnce(&Connection) -> Result<()>) -> Result<()> {
    let tx = crate::write_transaction(conn)?;
    body(&tx)?;
    set_version(&tx, v)?;
    tx.commit()?;
    Ok(())
}

fn migrate(
    conn: &mut Connection,
    workspace: &vorn_protocol::WorkspaceConfig,
    owner: &str,
) -> Result<()> {
    let version = version(conn)?;

    if version < 1 {
        step(conn, 1, |d| {
            if !has(d, "projects", "workspace_id")? {
                d.execute_batch(
                    "ALTER TABLE projects ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'",
                )?;
            }
            if !has(d, "workflows", "workspace_id")? {
                d.execute_batch(
                    "ALTER TABLE workflows ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'",
                )?;
            }
            d.execute(
                r#"INSERT OR IGNORE INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)"#,
                params![
                    workspace.id,
                    workspace.name,
                    opt_text(workspace.icon.as_deref()),
                    opt_text(workspace.icon_color.as_deref()),
                    num(workspace.order),
                ],
            )?;
            Ok(())
        })?;
    }

    if version < 2 {
        step(conn, 2, |d| {
            if !has(d, "remote_hosts", "auth_method")? {
                d.execute_batch(
                    "ALTER TABLE remote_hosts ADD COLUMN auth_method TEXT;
                     ALTER TABLE remote_hosts ADD COLUMN credential_id TEXT;
                     ALTER TABLE remote_hosts ADD COLUMN encrypted_password TEXT;
                     UPDATE remote_hosts SET auth_method = CASE WHEN ssh_key_path IS NOT NULL AND ssh_key_path != '' THEN 'key-file' ELSE 'agent' END;",
                )?;
            }
            d.execute_batch(
                "CREATE TABLE IF NOT EXISTS ssh_keys (
                   id TEXT PRIMARY KEY,
                   label TEXT NOT NULL,
                   encrypted_private_key TEXT NOT NULL,
                   public_key TEXT,
                   certificate TEXT,
                   key_type TEXT,
                   created_at TEXT NOT NULL
                 )",
            )?;
            Ok(())
        })?;
    }

    if version < 3 {
        step(conn, 3, |d| {
            if !has(d, "sessions", "sort_order")? {
                d.execute_batch(
                    "ALTER TABLE sessions ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
                )?;
            }
            Ok(())
        })?;
    }

    if version < 4 {
        step(conn, 4, |d| {
            if !has(d, "sessions", "worktree_name")? {
                d.execute_batch("ALTER TABLE sessions ADD COLUMN worktree_name TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 5 {
        step(conn, 5, |d| {
            if !has(d, "agent_commands", "headless_args")? {
                d.execute_batch("ALTER TABLE agent_commands ADD COLUMN headless_args TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 6 {
        step(conn, 6, |d| {
            // A fresh database already has agent_session_id from the DDL.
            if !has(d, "sessions", "claude_session_id")? && !has(d, "sessions", "agent_session_id")?
            {
                d.execute_batch("ALTER TABLE sessions ADD COLUMN claude_session_id TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 7 {
        step(conn, 7, |d| {
            let has_old = has(d, "sessions", "claude_session_id")?;
            let has_new = has(d, "sessions", "agent_session_id")?;
            if has_old && !has_new {
                if d.execute_batch(
                    "ALTER TABLE sessions RENAME COLUMN claude_session_id TO agent_session_id",
                )
                .is_err()
                {
                    d.execute_batch(
                        "ALTER TABLE sessions ADD COLUMN agent_session_id TEXT;
                         UPDATE sessions SET agent_session_id = claude_session_id;",
                    )?;
                }
            } else if has_old && has_new {
                d.execute_batch(
                    "UPDATE sessions SET agent_session_id = claude_session_id WHERE agent_session_id IS NULL AND claude_session_id IS NOT NULL",
                )?;
            } else if !has_new {
                d.execute_batch("ALTER TABLE sessions ADD COLUMN agent_session_id TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 8 {
        step(conn, 8, |d| {
            if !has(d, "workflow_run_nodes", "approved_at")? {
                d.execute_batch("ALTER TABLE workflow_run_nodes ADD COLUMN approved_at TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 9 {
        step(conn, 9, |d| {
            d.execute_batch(
                "CREATE TABLE IF NOT EXISTS source_connections (
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

                 CREATE TABLE IF NOT EXISTS task_source_links (
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
                 );",
            )?;
            for column in [
                "source_connector_id",
                "source_external_url",
                "source_external_id",
            ] {
                if !has(d, "tasks", column)? {
                    d.execute_batch(&format!("ALTER TABLE tasks ADD COLUMN {column} TEXT"))?;
                }
            }
            Ok(())
        })?;
    }

    if version < 10 {
        step(conn, 10, |d| {
            d.execute_batch("DROP TABLE IF EXISTS session_logs")?;
            Ok(())
        })?;
    }

    if version < 11 {
        step(conn, 11, |d| {
            if !has(d, "workflow_runs", "inputs")? {
                d.execute_batch("ALTER TABLE workflow_runs ADD COLUMN inputs TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 12 {
        step(conn, 12, |d| {
            for (column, kind) in [
                ("connector_inbox_id", "INTEGER"),
                ("connector_item", "TEXT"),
                ("connector_inbox_lease_token", "TEXT"),
                ("connector_inbox_disposition", "TEXT"),
            ] {
                if !has(d, "workflow_runs", column)? {
                    d.execute_batch(&format!(
                        "ALTER TABLE workflow_runs ADD COLUMN {column} {kind}"
                    ))?;
                }
            }
            d.execute_batch(
                "CREATE TABLE IF NOT EXISTS connector_poll_state (
                   workflow_id TEXT PRIMARY KEY REFERENCES workflows(id) ON DELETE CASCADE,
                   connection_id TEXT NOT NULL REFERENCES source_connections(id) ON DELETE CASCADE,
                   cursor TEXT,
                   last_polled_at TEXT,
                   last_error TEXT
                 );

                 CREATE TABLE IF NOT EXISTS connector_inbox (
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
                   UNIQUE (workflow_id, event_type, event_id)
                 );

                 CREATE INDEX IF NOT EXISTS idx_connector_inbox_ready
                   ON connector_inbox(status, available_at, lease_until);
                 CREATE INDEX IF NOT EXISTS idx_connector_inbox_connection
                   ON connector_inbox(connection_id, created_at);",
            )?;
            Ok(())
        })?;
    }

    if version < 13 {
        step(conn, 13, |d| {
            if !has(d, "connector_inbox", "lease_token")? {
                d.execute_batch("ALTER TABLE connector_inbox ADD COLUMN lease_token TEXT")?;
            }
            d.execute_batch(
                "ALTER TABLE connector_inbox RENAME TO connector_inbox_v12;

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

                 INSERT INTO connector_inbox (
                   id, workflow_id, connection_id, connector_id, event_id, event_type,
                   event_timestamp, payload, status, attempts, available_at, lease_until,
                   lease_token, last_error, created_at, processed_at
                 )
                 SELECT
                   id, workflow_id, connection_id, connector_id, event_id, event_type,
                   event_timestamp, payload, status, attempts, available_at, lease_until,
                   lease_token, last_error, created_at, processed_at
                 FROM connector_inbox_v12;

                 DROP TABLE connector_inbox_v12;

                 CREATE INDEX idx_connector_inbox_ready
                   ON connector_inbox(status, available_at, lease_until);
                 CREATE INDEX idx_connector_inbox_connection
                   ON connector_inbox(connection_id, created_at);",
            )?;
            Ok(())
        })?;
    }

    if version < 14 {
        step(conn, 14, |d| {
            let n: i64 = d.query_row("SELECT COUNT(*) AS n FROM users", [], |row| row.get(0))?;
            if n == 0 {
                let name = if owner.is_empty() { "owner" } else { owner };
                d.execute(
                    "INSERT INTO users (id, name, role, created_at) VALUES (?, ?, 'owner', ?)",
                    params![random_uuid(), name, now_iso()],
                )?;
            }
            Ok(())
        })?;
    }

    if version < 15 {
        step(conn, 15, |d| {
            for table in REVISIONED_TABLES {
                if has(d, table, "row_revision")? {
                    continue;
                }
                d.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN row_revision INTEGER NOT NULL DEFAULT 0"
                ))?;
            }
            d.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value) VALUES (?, ?)",
                params![CONFIG_REVISION_KEY, "0"],
            )?;
            Ok(())
        })?;
    }

    if version < 16 {
        step(conn, 16, |d| {
            if !has(d, "sessions", "shell_cwd")? {
                d.execute_batch("ALTER TABLE sessions ADD COLUMN shell_cwd TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 17 {
        step(conn, 17, |d| {
            let derived = "(
                SELECT json_extract(sc.filters, '$.sdkConnectorId')
                  FROM task_source_links tsl
                  JOIN source_connections sc ON sc.id = tsl.connection_id
                 WHERE tsl.task_id = tasks.id
              )";
            d.execute_batch(&format!(
                "UPDATE tasks
                    SET source_connector_id = {derived}
                  WHERE source_connector_id = 'mcp' AND {derived} IS NOT NULL"
            ))?;
            d.execute_batch(
                "UPDATE task_source_links
                    SET connector_id = (
                      SELECT json_extract(sc.filters, '$.sdkConnectorId')
                        FROM source_connections sc
                       WHERE sc.id = task_source_links.connection_id
                    )
                  WHERE connector_id = 'mcp'
                    AND (
                      SELECT json_extract(sc.filters, '$.sdkConnectorId')
                        FROM source_connections sc
                       WHERE sc.id = task_source_links.connection_id
                    ) IS NOT NULL",
            )?;
            Ok(())
        })?;
    }

    if version < 18 {
        step(conn, 18, |d| {
            if !has(d, "sessions", "group_id")? {
                d.execute_batch("ALTER TABLE sessions ADD COLUMN group_id TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 19 {
        step(conn, 19, |d| {
            for column in ["worktree_path", "worktree_name", "worktree_origin"] {
                if !has(d, "workflow_run_nodes", column)? {
                    d.execute_batch(&format!(
                        "ALTER TABLE workflow_run_nodes ADD COLUMN {column} TEXT"
                    ))?;
                }
            }
            Ok(())
        })?;
    }

    if version < 20 {
        step(conn, 20, |d| {
            for column in ["signed_in_as", "signed_in_at"] {
                if !has(d, "source_connections", column)? {
                    d.execute_batch(&format!(
                        "ALTER TABLE source_connections ADD COLUMN {column} TEXT"
                    ))?;
                }
            }
            Ok(())
        })?;
    }

    if version < 21 {
        // A package's connections move to `sdk` under the same ids; a catalog
        // MCP server never records `sdkVersion`, so it stays.
        let packaged = "connector_id = 'mcp'
               AND coalesce(json_extract(filters, '$.sdkConnectorId'), '') <> ''
               AND coalesce(json_extract(filters, '$.sdkVersion'), '') <> ''";
        step(conn, 21, |d| {
            d.execute_batch(&format!(
                r"UPDATE source_connections
                    SET filters = json_set(filters, '$.sdkTrigger', substr(json_extract(filters, '$.pollTool'), 6))
                  WHERE {packaged}
                    AND json_extract(filters, '$.pollTool') LIKE 'poll\_%' ESCAPE '\'"
            ))?;
            d.execute_batch(&format!(
                "UPDATE source_connections
                    SET connector_id = 'sdk',
                        filters = json_remove(filters, '$.discoveredTools', '$.pollTool', '$.pollArgs',
                          '$.itemsPath', '$.idField', '$.timestampField', '$.titleField', '$.urlField',
                          '$.cursorArg', '$.cursorPath')
                  WHERE {packaged}"
            ))?;
            d.execute_batch(
                "UPDATE connector_inbox SET connector_id = 'sdk'
                  WHERE connector_id = 'mcp'
                    AND connection_id IN (SELECT id FROM source_connections WHERE connector_id = 'sdk')",
            )?;
            Ok(())
        })?;
    }

    if version < 22 {
        step(conn, 22, |d| {
            if !has(d, "workflow_runs", "definition")? {
                d.execute_batch("ALTER TABLE workflow_runs ADD COLUMN definition TEXT")?;
            }
            Ok(())
        })?;
    }

    if version < 23 {
        step(conn, 23, |d| {
            add_missing(d, "workflow_run_nodes", &GATE_COLUMNS)
        })?;
    }

    if version < 24 {
        step(conn, 24, |d| {
            add_missing(d, "workflow_run_nodes", &GATE_EDIT_COLUMNS)
        })?;
    }

    if version < 25 {
        step(conn, 25, |d| {
            d.execute_batch(ARTIFACT_DDL)?;
            Ok(())
        })?;
    }

    if version < 26 {
        step(conn, 26, |d| {
            d.execute_batch(EFFECT_RECEIPTS_DDL)?;
            Ok(())
        })?;
    }

    // The Experimental settings are gone: their one switch is how Vorn always runs now.
    if version < 27 {
        step(conn, 27, |d| {
            d.execute("DELETE FROM defaults WHERE key = 'experimental'", [])?;
            Ok(())
        })?;
    }

    Ok(())
}

fn add_missing(d: &Connection, table: &str, wanted: &[(&str, &str)]) -> Result<()> {
    let existing = columns(d, table)?;
    for (column, kind) in wanted {
        if !existing.iter().any(|c| c == column) {
            d.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))?;
        }
    }
    Ok(())
}

/// Repairs columns a migration should have added but did not (a version bumped
/// whose ALTER did not stick). Silent when the schema is whole.
fn verify(conn: &mut Connection) -> Result<()> {
    let mut expected: Vec<(&str, Vec<(String, String)>)> = vec![
        (
            "projects",
            vec![col(
                "workspace_id",
                "ALTER TABLE projects ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'",
            )],
        ),
        (
            "workflows",
            vec![col(
                "workspace_id",
                "ALTER TABLE workflows ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'",
            )],
        ),
        (
            "remote_hosts",
            vec![
                col(
                    "auth_method",
                    "ALTER TABLE remote_hosts ADD COLUMN auth_method TEXT",
                ),
                col(
                    "credential_id",
                    "ALTER TABLE remote_hosts ADD COLUMN credential_id TEXT",
                ),
                col(
                    "encrypted_password",
                    "ALTER TABLE remote_hosts ADD COLUMN encrypted_password TEXT",
                ),
            ],
        ),
        (
            "sessions",
            vec![
                col(
                    "sort_order",
                    "ALTER TABLE sessions ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
                ),
                col("group_id", "ALTER TABLE sessions ADD COLUMN group_id TEXT"),
                col(
                    "worktree_name",
                    "ALTER TABLE sessions ADD COLUMN worktree_name TEXT",
                ),
                col(
                    "agent_session_id",
                    "ALTER TABLE sessions ADD COLUMN agent_session_id TEXT",
                ),
                col(
                    "shell_cwd",
                    "ALTER TABLE sessions ADD COLUMN shell_cwd TEXT",
                ),
                col(
                    "head_commit",
                    "ALTER TABLE sessions ADD COLUMN head_commit TEXT",
                ),
                col(
                    "renamed_by_person",
                    "ALTER TABLE sessions ADD COLUMN renamed_by_person INTEGER",
                ),
            ],
        ),
        (
            "agent_commands",
            vec![col(
                "headless_args",
                "ALTER TABLE agent_commands ADD COLUMN headless_args TEXT",
            )],
        ),
        (
            "workflow_runs",
            vec![
                col("inputs", "ALTER TABLE workflow_runs ADD COLUMN inputs TEXT"),
                col(
                    "connector_item",
                    "ALTER TABLE workflow_runs ADD COLUMN connector_item TEXT",
                ),
                col(
                    "connector_inbox_id",
                    "ALTER TABLE workflow_runs ADD COLUMN connector_inbox_id INTEGER",
                ),
                col(
                    "connector_inbox_lease_token",
                    "ALTER TABLE workflow_runs ADD COLUMN connector_inbox_lease_token TEXT",
                ),
                col(
                    "connector_inbox_disposition",
                    "ALTER TABLE workflow_runs ADD COLUMN connector_inbox_disposition TEXT",
                ),
                col(
                    "definition",
                    "ALTER TABLE workflow_runs ADD COLUMN definition TEXT",
                ),
            ],
        ),
        (
            "connector_inbox",
            vec![col(
                "lease_token",
                "ALTER TABLE connector_inbox ADD COLUMN lease_token TEXT",
            )],
        ),
        ("workflow_run_nodes", {
            let mut nodes = vec![
                col(
                    "waiting_for",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN waiting_for TEXT",
                ),
                col(
                    "agent_type",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN agent_type TEXT",
                ),
                col(
                    "project_name",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN project_name TEXT",
                ),
                col(
                    "project_path",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN project_path TEXT",
                ),
                col(
                    "approved_at",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN approved_at TEXT",
                ),
                col(
                    "diagnostics",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN diagnostics TEXT",
                ),
                col(
                    "output",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN output TEXT",
                ),
                col(
                    "structured_output",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN structured_output TEXT",
                ),
                col(
                    "iteration",
                    "ALTER TABLE workflow_run_nodes ADD COLUMN iteration INTEGER",
                ),
            ];
            for (column, kind) in GATE_COLUMNS.iter().chain(GATE_EDIT_COLUMNS.iter()) {
                nodes.push(col(
                    column,
                    &format!("ALTER TABLE workflow_run_nodes ADD COLUMN {column} {kind}"),
                ));
            }
            nodes
        }),
        (
            "tasks",
            vec![
                col(
                    "source_connector_id",
                    "ALTER TABLE tasks ADD COLUMN source_connector_id TEXT",
                ),
                col(
                    "source_external_url",
                    "ALTER TABLE tasks ADD COLUMN source_external_url TEXT",
                ),
                col(
                    "source_external_id",
                    "ALTER TABLE tasks ADD COLUMN source_external_id TEXT",
                ),
                col(
                    "archived_at",
                    "ALTER TABLE tasks ADD COLUMN archived_at TEXT",
                ),
            ],
        ),
    ];
    // Migration 15, appended to each table's list as database.ts does.
    for table in REVISIONED_TABLES {
        let ddl = format!("ALTER TABLE {table} ADD COLUMN row_revision INTEGER NOT NULL DEFAULT 0");
        match expected.iter_mut().find(|(t, _)| *t == table) {
            Some((_, list)) => list.push(col("row_revision", &ddl)),
            None => expected.push((table, vec![col("row_revision", &ddl)])),
        }
    }

    for (table, wanted) in expected {
        let existing = columns(conn, table)?;
        for (column, ddl) in wanted {
            if existing.contains(&column) {
                continue;
            }
            // Best effort, as database.ts: a repair that fails is logged there
            // and skipped here.
            let _ = conn.execute_batch(&ddl);
        }
    }
    Ok(())
}

fn col(column: &str, ddl: &str) -> (String, String) {
    (column.to_owned(), ddl.to_owned())
}

/// A workflow polling a connection before poll state was per workflow gets
/// the connection's cursor as its own, once.
fn seed_legacy_connector_poll_state(conn: &mut Connection) -> Result<()> {
    let workflows: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT id, nodes FROM workflows")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (id, nodes) in workflows {
        let Ok(Value::Array(nodes)) = serde_json::from_str::<Value>(&nodes) else {
            continue;
        };
        let trigger = nodes.iter().find(|node| {
            node.get("type").and_then(Value::as_str) == Some("trigger")
                && node
                    .get("config")
                    .and_then(|c| c.get("triggerType"))
                    .and_then(Value::as_str)
                    == Some("connectorPoll")
        });
        let Some(connection_id) = trigger
            .and_then(|t| t.get("config"))
            .and_then(|c| c.get("connectionId"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let connection: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT sync_cursor, last_sync_at FROM source_connections WHERE id = ?",
                [&connection_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((cursor, last_sync_at)) = connection else {
            continue;
        };
        conn.execute(
            "INSERT OR IGNORE INTO connector_poll_state (
               workflow_id, connection_id, cursor, last_polled_at, last_error
             ) VALUES (?, ?, ?, ?, NULL)",
            params![id, connection_id, cursor, last_sync_at],
        )?;
    }
    Ok(())
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

    #[test]
    fn a_new_database_is_at_the_latest_version() {
        let store = test_support::store();
        assert_eq!(store.schema_version().unwrap(), 27);
        let tables: Vec<String> = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
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
    }

    #[test]
    fn drops_the_experimental_settings_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vorn.db");
        let (store, _) = Store::open(&path, test_support::options()).unwrap();
        store
            .conn()
            .execute_batch(
                r#"INSERT INTO defaults (key, value) VALUES ('experimental', '{"nativeServer":true}');
                   INSERT OR REPLACE INTO defaults (key, value) VALUES ('theme', '"dark"');
                   UPDATE schema_meta SET value = '26' WHERE key = 'schema_version';"#,
            )
            .unwrap();
        drop(store);
        let (store, _) = Store::open(&path, test_support::options()).unwrap();
        let keys: Vec<String> = store
            .conn()
            .prepare("SELECT key FROM defaults WHERE key IN ('experimental', 'theme')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(keys, ["theme"]);
        assert_eq!(store.schema_version().unwrap(), 27);
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
    fn parses_versions_as_parse_int_does() {
        assert_eq!(parse_int("26"), Some(26));
        assert_eq!(parse_int("12abc"), Some(12));
        assert_eq!(parse_int("abc"), None);
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
        assert_eq!(store.schema_version().unwrap(), 27);
    }
}
