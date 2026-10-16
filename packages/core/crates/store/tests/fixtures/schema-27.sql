-- A version 27 database: the schema as that version left it on disk.
CREATE TABLE schema_meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
CREATE TABLE defaults (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
CREATE TABLE projects (
      name TEXT PRIMARY KEY,
      path TEXT NOT NULL,
      preferred_agents TEXT NOT NULL DEFAULT '[]',
      icon TEXT,
      icon_color TEXT,
      host_ids TEXT
    , workspace_id TEXT NOT NULL DEFAULT 'personal', row_revision INTEGER NOT NULL DEFAULT 0);
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
      stagger_delay_ms INTEGER
    , workspace_id TEXT NOT NULL DEFAULT 'personal');
CREATE TABLE agent_commands (
      agent_type TEXT PRIMARY KEY,
      command TEXT NOT NULL,
      args TEXT NOT NULL DEFAULT '[]',
      headless_args TEXT,
      fallback_command TEXT,
      fallback_args TEXT
    , row_revision INTEGER NOT NULL DEFAULT 0);
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
      ssh_options TEXT
    , row_revision INTEGER NOT NULL DEFAULT 0);
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
      completed_at TEXT
    , source_connector_id TEXT, source_external_url TEXT, source_import_mode TEXT, source_external_id TEXT, archived_at TEXT, row_revision INTEGER NOT NULL DEFAULT 0);
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
      worktree_name TEXT
    , agent_session_id TEXT, claude_session_id TEXT, shell_cwd TEXT, head_commit TEXT, group_id TEXT, renamed_by_person INTEGER);
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
CREATE INDEX idx_tasks_project ON tasks(project_name, status);
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
CREATE TABLE workspaces (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0
    , row_revision INTEGER NOT NULL DEFAULT 0);
CREATE TABLE workflow_runs (
      id TEXT PRIMARY KEY,
      workflow_id TEXT NOT NULL,
      started_at TEXT NOT NULL,
      completed_at TEXT,
      status TEXT NOT NULL DEFAULT 'running',
      trigger_task_id TEXT
    , inputs TEXT, connector_inbox_id INTEGER, connector_item TEXT, connector_inbox_lease_token TEXT, connector_inbox_disposition TEXT, definition TEXT);
CREATE TABLE workflow_run_nodes (
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
      agent_session_id TEXT, approved_at TEXT, agent_type TEXT, project_name TEXT, project_path TEXT, diagnostics TEXT, output TEXT, structured_output TEXT, iteration INTEGER, worktree_path TEXT, worktree_name TEXT, worktree_origin TEXT, waiting_for TEXT, message TEXT, view_token TEXT, round INTEGER, feedback TEXT, rejected_at TEXT, editable_text TEXT, edited_text TEXT,
      FOREIGN KEY (run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
    );
CREATE INDEX idx_workflow_runs_workflow ON workflow_runs(workflow_id);
CREATE INDEX idx_workflow_runs_task ON workflow_runs(trigger_task_id);
CREATE INDEX idx_workflow_run_nodes_run ON workflow_run_nodes(run_id);
CREATE INDEX idx_workflow_run_nodes_task ON workflow_run_nodes(task_id);
CREATE TABLE session_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id TEXT NOT NULL,
      event_type TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      metadata TEXT
    );
CREATE INDEX idx_session_events_session ON session_events(session_id, timestamp DESC);
CREATE INDEX idx_session_events_type ON session_events(event_type, timestamp DESC);
CREATE TABLE source_connections (
          id TEXT PRIMARY KEY,
          connector_id TEXT NOT NULL,
          name TEXT NOT NULL,
          filters TEXT NOT NULL DEFAULT '{}',
          import_mode TEXT NOT NULL DEFAULT 'actionable',
          sync_interval_minutes INTEGER NOT NULL DEFAULT 5,
          status_mapping TEXT NOT NULL DEFAULT '{}',
          execution_project TEXT,
          last_sync_at TEXT,
          last_sync_error TEXT,
          sync_cursor TEXT,
          created_at TEXT NOT NULL
        , signed_in_as TEXT, signed_in_at TEXT);
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
CREATE TABLE users (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      role TEXT NOT NULL DEFAULT 'owner',
      created_at TEXT NOT NULL
    );
CREATE TABLE device_tokens (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      token_hash TEXT NOT NULL,
      created_at TEXT NOT NULL,
      last_seen_at TEXT,
      revoked_at TEXT
    );
CREATE INDEX idx_device_tokens_user
      ON device_tokens(user_id);
CREATE TABLE session_groups (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      icon TEXT,
      icon_color TEXT,
      "order" INTEGER NOT NULL DEFAULT 0,
      workspace_id TEXT NOT NULL DEFAULT 'personal',
      row_revision INTEGER NOT NULL DEFAULT 0
    );
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
CREATE TABLE effect_receipts (
    effect_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    received_at INTEGER NOT NULL
  );
CREATE INDEX idx_effect_receipts_received ON effect_receipts(received_at);

-- A few rows, so the upgrade is seen to keep them.
INSERT INTO schema_meta (key, value) VALUES ('schema_version', '27'), ('config_revision', '12');
INSERT INTO workspaces (id, name, "order", row_revision) VALUES ('personal', 'Personal', 0, 3);
INSERT INTO users (id, name, role, created_at) VALUES ('u1', 'owner', 'owner', '2026-01-01T00:00:00.000Z');
INSERT INTO agent_commands (agent_type, command, args, headless_args, fallback_command, fallback_args, row_revision)
  VALUES ('claude', 'claude', '["--a"]', '["-p"]', NULL, NULL, 4);
INSERT INTO sessions (id, agent_type, project_name, project_path, status, created_at, pid, agent_session_id, claude_session_id)
  VALUES ('s1', 'claude', 'proj', '/tmp/proj', 'idle', 1, 0, 'a1', NULL);
INSERT INTO tasks (id, project_name, title, created_at, updated_at, source_import_mode)
  VALUES ('t1', 'proj', 'A task', '2026-01-01', '2026-01-01', NULL);
INSERT INTO source_connections (id, connector_id, name, import_mode, created_at)
  VALUES ('c1', 'sdk', 'Issues', 'actionable', '2026-01-01');
