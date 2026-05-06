import Database from 'libsql'
import path from 'node:path'
import os from 'node:os'
import fs from 'node:fs'
import log from './logger'
import {
  AppConfig,
  ProjectConfig,
  WorkflowDefinition,
  WorkflowExecution,
  NodeExecutionState,
  AgentCommandConfig,
  RemoteHost,
  AuthMethod,
  SSHKey,
  SSHKeyMeta,
  TaskConfig,
  TerminalSession,
  ScheduleLogEntry,
  AgentType,
  AiAgentType,
  WorkspaceConfig,
  DEFAULT_WORKSPACE,
  SessionLog,
  SessionEvent,
  SessionEventType,
  SourceConnection,
  TaskSourceLink
} from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { DEFAULT_TASK_WORKFLOW_ID, buildDefaultTaskWorkflow } from './default-workflows'

const CONFIG_DIR = path.join(os.homedir(), '.vorn')
const DB_PATH = path.join(CONFIG_DIR, 'vorn.db')
const MAX_LOG_ENTRIES = 200

let db: Database.Database | null = null

function getDb(): Database.Database {
  if (!db) throw new Error('Database not initialized. Call initDatabase() first.')
  return db
}

export function initDatabase(): void {
  if (!fs.existsSync(CONFIG_DIR)) {
    fs.mkdirSync(CONFIG_DIR, { recursive: true, mode: 0o700 })
  }

  try {
    db = new Database(DB_PATH)
    db.pragma('journal_mode = WAL')
    db.pragma('foreign_keys = ON')
    createSchema()
    seedSystemDefaults()
  } catch (err) {
    log.error('[database] Failed to open database:', err)

    // Detect corruption: libsql throws on open or pragma for corrupt files
    const message = err instanceof Error ? err.message : String(err)
    const isCorrupt = /corrupt|notadb|malformed|not a database|file is not a database/i.test(
      message
    )

    if (isCorrupt) {
      log.warn('[database] Database appears corrupt, attempting recovery...')
      recoverCorruptDatabase()
    } else {
      throw err
    }
  }
}

/**
 * Insert the seeded "Default Task Workflow" on first launch. Gated by the
 * `hasSeededDefaultTaskWorkflow` defaults flag: once the user has seen (and
 * possibly deleted) the seeded workflow, we never re-seed. This means delete
 * sticks, and users upgrading from a pre-seed version will get it once.
 *
 * Exported so tests can exercise the seeding flow against an in-memory
 * database via `initTestDatabase` without spinning up the full init path.
 */
export function seedSystemDefaults(): void {
  const d = getDb()

  const flagRow = d
    .prepare("SELECT value FROM defaults WHERE key = 'hasSeededDefaultTaskWorkflow'")
    .get() as { value: string } | undefined
  if (flagRow) {
    try {
      if (JSON.parse(flagRow.value) === true) return
    } catch {
      // corrupted value — fall through and re-seed
    }
  }

  // Safety net: skip if a workflow with the stable id already exists from a
  // manual import or partial upgrade. Still set the flag so we don't retry.
  const existing = d
    .prepare('SELECT id FROM workflows WHERE id = ?')
    .get(DEFAULT_TASK_WORKFLOW_ID) as { id: string } | undefined
  if (!existing) {
    const w = buildDefaultTaskWorkflow()
    d.prepare(
      `INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    ).run(
      w.id,
      w.name,
      w.icon,
      w.iconColor,
      JSON.stringify(w.nodes),
      JSON.stringify(w.edges),
      w.enabled ? 1 : 0,
      w.lastRunAt ?? null,
      w.lastRunStatus ?? null,
      w.staggerDelayMs ?? null,
      w.workspaceId ?? 'personal'
    )
    log.info(`[database] Seeded default task workflow (${DEFAULT_TASK_WORKFLOW_ID})`)
  }

  d.prepare(
    "INSERT OR REPLACE INTO defaults (key, value) VALUES ('hasSeededDefaultTaskWorkflow', ?)"
  ).run(JSON.stringify(true))
}

/**
 * Backs up the corrupt database file, creates a fresh one, and shows
 * a dialog informing the user that their settings were reset.
 */
function recoverCorruptDatabase(): void {
  // Close any partially-opened handle
  try {
    db?.close()
  } catch {
    /* ignore */
  }
  db = null

  // Back up the corrupt file
  const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
  const backupPath = `${DB_PATH}.corrupt-${timestamp}`
  try {
    if (fs.existsSync(DB_PATH)) {
      fs.copyFileSync(DB_PATH, backupPath)
      log.info(`[database] Backed up corrupt database to ${backupPath}`)
    }
    // Remove corrupt DB + WAL/SHM files
    for (const suffix of ['', '-wal', '-shm']) {
      const file = DB_PATH + suffix
      if (fs.existsSync(file)) fs.unlinkSync(file)
    }
  } catch (backupErr) {
    log.error('[database] Failed to back up corrupt database:', backupErr)
  }

  // Create a fresh database
  try {
    db = new Database(DB_PATH)
    db.pragma('journal_mode = WAL')
    db.pragma('foreign_keys = ON')
    createSchema()
    seedSystemDefaults()
    log.info('[database] Successfully created fresh database after corruption recovery')
  } catch (freshErr) {
    log.error('[database] Failed to create fresh database after corruption:', freshErr)
    throw freshErr
  }

  log.warn(`[database] Database was corrupted and has been reset. Backup saved to: ${backupPath}`)
}

/**
 * Touch a signal file so the config-manager watcher detects external DB mutations
 * (e.g. from MCP stdio process). The server's own mutations use notifyChanged() directly.
 */
export function dbSignalChange(): void {
  try {
    const signalPath = path.join(CONFIG_DIR, '.db-signal')
    fs.writeFileSync(signalPath, Date.now().toString())
  } catch {
    // Best-effort — watcher fallback will catch it
  }
}

export function closeDatabase(): void {
  if (db) {
    db.close()
    db = null
  }
}

/** Initialize an in-memory database for tests. Returns teardown function. */
export function initTestDatabase(): () => void {
  if (db) closeDatabase()
  db = new Database(':memory:')
  db.pragma('journal_mode = WAL')
  db.pragma('foreign_keys = ON')
  createSchema()
  return () => closeDatabase()
}

function createSchema(): void {
  const d = getDb()

  // Migrate: if old-format workflows table exists (had 'actions' column),
  // back it up before dropping so we don't silently destroy user data.
  const cols = d.prepare('PRAGMA table_info(workflows)').all() as Array<{ name: string }>
  if (cols.some((c) => c.name === 'actions')) {
    d.exec('ALTER TABLE workflows RENAME TO workflows_backup_old_format')
    log.warn('[database] migrated old-format workflows table to workflows_backup_old_format')
  }
  d.exec(`
    CREATE TABLE IF NOT EXISTS schema_meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS defaults (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

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
      completed_at TEXT
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
      agent_session_id TEXT
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

    CREATE TABLE IF NOT EXISTS workflow_runs (
      id TEXT PRIMARY KEY,
      workflow_id TEXT NOT NULL,
      started_at TEXT NOT NULL,
      completed_at TEXT,
      status TEXT NOT NULL DEFAULT 'running',
      trigger_task_id TEXT
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
      FOREIGN KEY (run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
    );

    CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow ON workflow_runs(workflow_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_runs_task ON workflow_runs(trigger_task_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_run_nodes_run ON workflow_run_nodes(run_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_run_nodes_task ON workflow_run_nodes(task_id);

    CREATE TABLE IF NOT EXISTS session_logs (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      task_id TEXT NOT NULL,
      session_id TEXT NOT NULL,
      agent_type TEXT,
      branch TEXT,
      status TEXT NOT NULL DEFAULT 'running',
      started_at TEXT NOT NULL,
      completed_at TEXT,
      exit_code INTEGER,
      logs TEXT,
      project_name TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_session_logs_task ON session_logs(task_id);
    CREATE INDEX IF NOT EXISTS idx_session_logs_session ON session_logs(session_id);

    CREATE TABLE IF NOT EXISTS session_events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id TEXT NOT NULL,
      event_type TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      metadata TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id, timestamp DESC);
    CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(event_type, timestamp DESC);
  `)

  migrateSchema(d)
  verifySchema(d)
}

function migrateSchema(d: Database.Database): void {
  const row = d.prepare("SELECT value FROM schema_meta WHERE key = 'schema_version'").get() as
    | { value: string }
    | undefined
  const version = row ? parseInt(row.value, 10) : 0

  if (version < 1) {
    d.transaction(() => {
      // Add workspace_id to projects and workflows
      const projectCols = d.prepare('PRAGMA table_info(projects)').all() as Array<{ name: string }>
      if (!projectCols.some((c) => c.name === 'workspace_id')) {
        d.exec("ALTER TABLE projects ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'")
      }

      const workflowCols = d.prepare('PRAGMA table_info(workflows)').all() as Array<{
        name: string
      }>
      if (!workflowCols.some((c) => c.name === 'workspace_id')) {
        d.exec("ALTER TABLE workflows ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'")
      }

      // Seed default workspace
      d.prepare(
        `INSERT OR IGNORE INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)`
      ).run(
        DEFAULT_WORKSPACE.id,
        DEFAULT_WORKSPACE.name,
        DEFAULT_WORKSPACE.icon ?? null,
        DEFAULT_WORKSPACE.iconColor ?? null,
        DEFAULT_WORKSPACE.order
      )

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '1')"
      ).run()
    })()
    log.info('[database] migrated schema to version 1 (workspaces)')
  }

  if (version < 2) {
    d.transaction(() => {
      // Add new columns to remote_hosts for credential support
      const hostCols = d.prepare('PRAGMA table_info(remote_hosts)').all() as Array<{ name: string }>
      if (!hostCols.some((c) => c.name === 'auth_method')) {
        d.exec('ALTER TABLE remote_hosts ADD COLUMN auth_method TEXT')
        d.exec('ALTER TABLE remote_hosts ADD COLUMN credential_id TEXT')
        d.exec('ALTER TABLE remote_hosts ADD COLUMN encrypted_password TEXT')

        // Migrate: key-file if sshKeyPath set, otherwise agent
        d.exec(
          "UPDATE remote_hosts SET auth_method = CASE WHEN ssh_key_path IS NOT NULL AND ssh_key_path != '' THEN 'key-file' ELSE 'agent' END"
        )
      }

      // Create ssh_keys table
      d.exec(`
        CREATE TABLE IF NOT EXISTS ssh_keys (
          id TEXT PRIMARY KEY,
          label TEXT NOT NULL,
          encrypted_private_key TEXT NOT NULL,
          public_key TEXT,
          certificate TEXT,
          key_type TEXT,
          created_at TEXT NOT NULL
        )
      `)

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '2')"
      ).run()
    })()
    log.info('[database] migrated schema to version 2 (ssh credential vault)')
  }

  if (version < 3) {
    d.transaction(() => {
      const sessionCols = d.prepare('PRAGMA table_info(sessions)').all() as Array<{ name: string }>
      if (!sessionCols.some((c) => c.name === 'sort_order')) {
        d.exec('ALTER TABLE sessions ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0')
      }

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '3')"
      ).run()
    })()
    log.info('[database] migrated schema to version 3 (session sort order)')
  }

  if (version < 4) {
    d.transaction(() => {
      const sessionCols = d.prepare('PRAGMA table_info(sessions)').all() as Array<{ name: string }>
      if (!sessionCols.some((c) => c.name === 'worktree_name')) {
        d.exec('ALTER TABLE sessions ADD COLUMN worktree_name TEXT')
      }

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '4')"
      ).run()
    })()
    log.info('[database] migrated schema to version 4 (worktree name)')
  }

  if (version < 5) {
    d.transaction(() => {
      const agentCols = d.prepare('PRAGMA table_info(agent_commands)').all() as Array<{
        name: string
      }>
      if (!agentCols.some((c) => c.name === 'headless_args')) {
        d.exec('ALTER TABLE agent_commands ADD COLUMN headless_args TEXT')
      }

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '5')"
      ).run()
    })()
    log.info('[database] migrated schema to version 5 (headless args)')
  }

  if (version < 6) {
    d.transaction(() => {
      const sessionCols = d.prepare('PRAGMA table_info(sessions)').all() as Array<{
        name: string
      }>
      // Skip adding claude_session_id if agent_session_id already exists
      // (fresh DBs create agent_session_id directly via createSchema)
      if (
        !sessionCols.some((c) => c.name === 'claude_session_id') &&
        !sessionCols.some((c) => c.name === 'agent_session_id')
      ) {
        d.exec('ALTER TABLE sessions ADD COLUMN claude_session_id TEXT')
      }
      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '6')"
      ).run()
    })()
    log.info('[database] migrated schema to version 6 (claude session id)')
  }

  if (version < 7) {
    d.transaction(() => {
      const sessionCols = d.prepare('PRAGMA table_info(sessions)').all() as Array<{
        name: string
      }>
      const hasOld = sessionCols.some((c) => c.name === 'claude_session_id')
      const hasNew = sessionCols.some((c) => c.name === 'agent_session_id')
      if (hasOld && !hasNew) {
        try {
          d.exec('ALTER TABLE sessions RENAME COLUMN claude_session_id TO agent_session_id')
        } catch {
          // SQLite < 3.25 fallback: add new column and copy data
          d.exec('ALTER TABLE sessions ADD COLUMN agent_session_id TEXT')
          d.exec('UPDATE sessions SET agent_session_id = claude_session_id')
        }
      } else if (hasOld && hasNew) {
        // Both columns exist (e.g. fresh DB ran v6 before v7) — backfill any
        // data from claude_session_id into agent_session_id so resume IDs
        // aren't stranded, then drop the redundant column.
        d.exec(
          'UPDATE sessions SET agent_session_id = claude_session_id WHERE agent_session_id IS NULL AND claude_session_id IS NOT NULL'
        )
      } else if (!hasNew) {
        d.exec('ALTER TABLE sessions ADD COLUMN agent_session_id TEXT')
      }
      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '7')"
      ).run()
    })()
    log.info(
      '[database] migrated schema to version 7 (rename claude_session_id → agent_session_id)'
    )
  }

  if (version < 8) {
    d.transaction(() => {
      const cols = d.prepare('PRAGMA table_info(workflow_run_nodes)').all() as Array<{
        name: string
      }>
      if (!cols.some((c) => c.name === 'approved_at')) {
        d.exec('ALTER TABLE workflow_run_nodes ADD COLUMN approved_at TEXT')
      }

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '8')"
      ).run()
    })()
    log.info('[database] migrated schema to version 8 (approval gate timestamp)')
  }

  if (version < 9) {
    d.transaction(() => {
      d.exec(`
        CREATE TABLE IF NOT EXISTS source_connections (
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
          created_at TEXT NOT NULL
        )
      `)

      d.exec(`
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
        )
      `)

      // Add source columns to tasks table
      const taskCols = d.prepare('PRAGMA table_info(tasks)').all() as Array<{ name: string }>
      if (!taskCols.some((c) => c.name === 'source_connector_id')) {
        d.exec('ALTER TABLE tasks ADD COLUMN source_connector_id TEXT')
      }
      if (!taskCols.some((c) => c.name === 'source_external_url')) {
        d.exec('ALTER TABLE tasks ADD COLUMN source_external_url TEXT')
      }
      if (!taskCols.some((c) => c.name === 'source_external_id')) {
        d.exec('ALTER TABLE tasks ADD COLUMN source_external_id TEXT')
      }

      d.prepare(
        "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '9')"
      ).run()
    })()
    log.info('[database] migrated schema to version 9 (connector source connections)')
  }
}

/**
 * Self-healing schema check — runs after migrations to repair columns that
 * migrations may have failed to add (e.g. version bumped but ALTER TABLE
 * didn't stick). Only touches migration-added columns; logs repairs, stays
 * silent when everything is healthy.
 */
function verifySchema(d: Database.Database): void {
  // Grouped by table to avoid redundant PRAGMA calls
  const expectedByTable: Record<string, { column: string; ddl: string }[]> = {
    projects: [
      {
        column: 'workspace_id',
        ddl: "ALTER TABLE projects ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'"
      }
    ],
    workflows: [
      {
        column: 'workspace_id',
        ddl: "ALTER TABLE workflows ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'personal'"
      }
    ],
    remote_hosts: [
      { column: 'auth_method', ddl: 'ALTER TABLE remote_hosts ADD COLUMN auth_method TEXT' },
      { column: 'credential_id', ddl: 'ALTER TABLE remote_hosts ADD COLUMN credential_id TEXT' },
      {
        column: 'encrypted_password',
        ddl: 'ALTER TABLE remote_hosts ADD COLUMN encrypted_password TEXT'
      }
    ],
    sessions: [
      {
        column: 'sort_order',
        ddl: 'ALTER TABLE sessions ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0'
      },
      { column: 'worktree_name', ddl: 'ALTER TABLE sessions ADD COLUMN worktree_name TEXT' },
      { column: 'agent_session_id', ddl: 'ALTER TABLE sessions ADD COLUMN agent_session_id TEXT' }
    ],
    agent_commands: [
      {
        column: 'headless_args',
        ddl: 'ALTER TABLE agent_commands ADD COLUMN headless_args TEXT'
      }
    ],
    workflow_run_nodes: [
      { column: 'agent_type', ddl: 'ALTER TABLE workflow_run_nodes ADD COLUMN agent_type TEXT' },
      {
        column: 'project_name',
        ddl: 'ALTER TABLE workflow_run_nodes ADD COLUMN project_name TEXT'
      },
      {
        column: 'project_path',
        ddl: 'ALTER TABLE workflow_run_nodes ADD COLUMN project_path TEXT'
      },
      { column: 'approved_at', ddl: 'ALTER TABLE workflow_run_nodes ADD COLUMN approved_at TEXT' }
    ],
    tasks: [
      {
        column: 'source_connector_id',
        ddl: 'ALTER TABLE tasks ADD COLUMN source_connector_id TEXT'
      },
      {
        column: 'source_external_url',
        ddl: 'ALTER TABLE tasks ADD COLUMN source_external_url TEXT'
      },
      {
        column: 'source_external_id',
        ddl: 'ALTER TABLE tasks ADD COLUMN source_external_id TEXT'
      }
    ]
  }

  for (const [table, columns] of Object.entries(expectedByTable)) {
    const existing = new Set(
      (d.prepare(`PRAGMA table_info(${table})`).all() as Array<{ name: string }>).map((c) => c.name)
    )
    for (const { column, ddl } of columns) {
      if (existing.has(column)) continue
      try {
        d.exec(ddl)
        log.warn(`[database] self-heal: added missing column ${table}.${column}`)
      } catch (err) {
        log.error(`[database] self-heal: failed to add ${table}.${column}:`, err)
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Config: load
// ---------------------------------------------------------------------------

export function loadConfig(): AppConfig {
  const d = getDb()

  const defaults = loadDefaults(d)
  const projects = loadProjects(d)
  const agentCommands = loadAgentCommands(d)
  const workflows = loadWorkflows(d)
  const remoteHosts = loadRemoteHosts(d)
  const tasks = loadTasks(d)
  const workspaces = loadWorkspaces(d)

  return {
    version: 1,
    defaults,
    projects,
    agentCommands:
      Object.keys(agentCommands).length > 0 ? agentCommands : { ...DEFAULT_AGENT_COMMANDS },
    workflows,
    remoteHosts,
    tasks,
    workspaces
  }
}

function loadDefaults(d: Database.Database): AppConfig['defaults'] {
  const rows = d.prepare('SELECT key, value FROM defaults').all() as {
    key: string
    value: string
  }[]
  const map: Record<string, unknown> = {}
  for (const row of rows) {
    map[row.key] = JSON.parse(row.value)
  }

  return {
    shell:
      (map.shell as string) ??
      (process.platform === 'win32'
        ? process.env.COMSPEC || 'powershell.exe'
        : process.env.SHELL || '/bin/zsh'),
    fontSize: (map.fontSize as number) ?? 13,
    theme: (map.theme as 'dark' | 'light') ?? 'dark',
    ...(map.rowHeight !== undefined && { rowHeight: map.rowHeight as number }),
    ...(map.defaultAgent !== undefined && { defaultAgent: map.defaultAgent as AiAgentType }),
    ...(map.notifications !== undefined && {
      notifications: map.notifications as AppConfig['defaults']['notifications']
    }),
    ...(map.hasSeenOnboarding !== undefined && {
      hasSeenOnboarding: map.hasSeenOnboarding as boolean | number
    }),
    ...(map.reopenSessions !== undefined && { reopenSessions: map.reopenSessions as boolean }),
    ...(map.widgetEnabled !== undefined && { widgetEnabled: map.widgetEnabled as boolean }),
    ...(map.taskViewMode !== undefined && {
      taskViewMode: map.taskViewMode as AppConfig['defaults']['taskViewMode']
    }),
    ...(map.activeWorkspace !== undefined && {
      activeWorkspace: map.activeWorkspace as string
    }),
    ...(map.mainViewMode !== undefined && {
      mainViewMode: map.mainViewMode as AppConfig['defaults']['mainViewMode']
    }),
    ...(map.layoutMode !== undefined && {
      layoutMode: map.layoutMode as AppConfig['defaults']['layoutMode']
    }),
    ...(map.minimizedPlacement !== undefined && {
      minimizedPlacement: map.minimizedPlacement as AppConfig['defaults']['minimizedPlacement']
    }),
    ...(map.updateChannel !== undefined && {
      updateChannel: map.updateChannel as AppConfig['defaults']['updateChannel']
    }),
    ...(map.webAccessEnabled !== undefined && {
      webAccessEnabled: map.webAccessEnabled as boolean
    }),
    ...(map.mobileAccessEnabled !== undefined && {
      mobileAccessEnabled: map.mobileAccessEnabled as boolean
    }),
    ...(map.networkAccessEnabled !== undefined && {
      networkAccessEnabled: map.networkAccessEnabled as boolean
    }),
    ...(map.showHeadlessAgents !== undefined && {
      showHeadlessAgents: map.showHeadlessAgents as boolean
    }),
    ...(map.headlessRetentionMinutes !== undefined && {
      headlessRetentionMinutes: map.headlessRetentionMinutes as number
    }),
    ...(map.hasSeededDefaultTaskWorkflow !== undefined && {
      hasSeededDefaultTaskWorkflow: map.hasSeededDefaultTaskWorkflow as boolean
    })
  }
}

function loadProjects(d: Database.Database): ProjectConfig[] {
  const rows = d.prepare('SELECT * FROM projects').all() as Array<{
    name: string
    path: string
    preferred_agents: string
    icon: string | null
    icon_color: string | null
    host_ids: string | null
    workspace_id: string | null
  }>
  return rows.map(rowToProject)
}

function loadWorkflows(d: Database.Database): WorkflowDefinition[] {
  const rows = d.prepare('SELECT * FROM workflows').all() as Array<{
    id: string
    name: string
    icon: string
    icon_color: string
    nodes: string
    edges: string
    enabled: number
    last_run_at: string | null
    last_run_status: string | null
    stagger_delay_ms: number | null
    workspace_id: string | null
  }>
  return rows.map(rowToWorkflow)
}

function loadAgentCommands(d: Database.Database): Partial<Record<AiAgentType, AgentCommandConfig>> {
  const rows = d.prepare('SELECT * FROM agent_commands').all() as Array<{
    agent_type: string
    command: string
    args: string
    headless_args: string | null
    fallback_command: string | null
    fallback_args: string | null
  }>
  const result: Partial<Record<AiAgentType, AgentCommandConfig>> = {}
  for (const r of rows) {
    result[r.agent_type as AiAgentType] = {
      command: r.command,
      args: JSON.parse(r.args),
      ...(r.headless_args != null && { headlessArgs: JSON.parse(r.headless_args) }),
      ...(r.fallback_command != null && { fallbackCommand: r.fallback_command }),
      ...(r.fallback_args != null && { fallbackArgs: JSON.parse(r.fallback_args) })
    }
  }
  return result
}

function loadRemoteHosts(d: Database.Database): RemoteHost[] {
  const rows = d.prepare('SELECT * FROM remote_hosts').all() as Array<{
    id: string
    label: string
    hostname: string
    user: string
    port: number
    auth_method: string | null
    ssh_key_path: string | null
    credential_id: string | null
    encrypted_password: string | null
    ssh_options: string | null
  }>
  return rows.map((r) => ({
    id: r.id,
    label: r.label,
    hostname: r.hostname,
    user: r.user,
    port: r.port,
    ...(r.auth_method != null && { authMethod: r.auth_method as AuthMethod }),
    ...(r.ssh_key_path != null && { sshKeyPath: r.ssh_key_path }),
    ...(r.credential_id != null && { credentialId: r.credential_id }),
    ...(r.encrypted_password != null && { encryptedPassword: r.encrypted_password }),
    ...(r.ssh_options != null && { sshOptions: r.ssh_options })
  }))
}

function loadTasks(d: Database.Database): TaskConfig[] {
  const rows = d.prepare('SELECT * FROM tasks ORDER BY "order"').all() as Array<{
    id: string
    project_name: string
    title: string
    description: string
    status: string
    order: number
    assigned_session_id: string | null
    assigned_agent: string | null
    agent_session_id: string | null
    branch: string | null
    use_worktree: number | null
    created_at: string
    updated_at: string
    completed_at: string | null
  }>
  return rows.map(rowToTask)
}

function loadWorkspaces(d: Database.Database): WorkspaceConfig[] {
  const rows = d.prepare('SELECT * FROM workspaces ORDER BY "order"').all() as Array<{
    id: string
    name: string
    icon: string | null
    icon_color: string | null
    order: number
  }>
  return rows.map(rowToWorkspace)
}

// ---------------------------------------------------------------------------
// Config: save (full replace inside a transaction)
// ---------------------------------------------------------------------------

export function saveConfig(config: AppConfig): void {
  const d = getDb()

  const run = d.transaction(() => {
    // Defaults
    d.prepare('DELETE FROM defaults').run()
    const insertDefault = d.prepare('INSERT INTO defaults (key, value) VALUES (?, ?)')
    for (const [key, value] of Object.entries(config.defaults)) {
      if (value !== undefined) {
        insertDefault.run(key, JSON.stringify(value))
      }
    }

    // Projects
    d.prepare('DELETE FROM projects').run()
    const insertProject = d.prepare(
      'INSERT INTO projects (name, path, preferred_agents, icon, icon_color, host_ids, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?)'
    )
    for (const p of config.projects) {
      insertProject.run(
        p.name,
        p.path,
        JSON.stringify(p.preferredAgents),
        p.icon ?? null,
        p.iconColor ?? null,
        p.hostIds ? JSON.stringify(p.hostIds) : null,
        p.workspaceId ?? 'personal'
      )
    }

    // Workflows
    d.prepare('DELETE FROM workflows').run()
    const insertWorkflow = d.prepare(
      `INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    for (const w of config.workflows ?? []) {
      insertWorkflow.run(
        w.id,
        w.name,
        w.icon,
        w.iconColor,
        JSON.stringify(w.nodes),
        JSON.stringify(w.edges),
        w.enabled ? 1 : 0,
        w.lastRunAt ?? null,
        w.lastRunStatus ?? null,
        w.staggerDelayMs ?? null,
        w.workspaceId ?? 'personal'
      )
    }

    // Agent commands
    d.prepare('DELETE FROM agent_commands').run()
    const insertAgent = d.prepare(
      'INSERT INTO agent_commands (agent_type, command, args, headless_args, fallback_command, fallback_args) VALUES (?, ?, ?, ?, ?, ?)'
    )
    if (config.agentCommands) {
      for (const [agentType, cmd] of Object.entries(config.agentCommands)) {
        if (cmd) {
          insertAgent.run(
            agentType,
            cmd.command,
            JSON.stringify(cmd.args),
            cmd.headlessArgs ? JSON.stringify(cmd.headlessArgs) : null,
            cmd.fallbackCommand ?? null,
            cmd.fallbackArgs ? JSON.stringify(cmd.fallbackArgs) : null
          )
        }
      }
    }

    // Remote hosts
    d.prepare('DELETE FROM remote_hosts').run()
    const insertHost = d.prepare(
      'INSERT INTO remote_hosts (id, label, hostname, user, port, auth_method, ssh_key_path, credential_id, encrypted_password, ssh_options) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)'
    )
    for (const h of config.remoteHosts ?? []) {
      insertHost.run(
        h.id,
        h.label,
        h.hostname,
        h.user,
        h.port,
        h.authMethod ?? 'agent',
        h.sshKeyPath ?? null,
        h.credentialId ?? null,
        h.encryptedPassword ?? null,
        h.sshOptions ?? null
      )
    }

    // Tasks
    d.prepare('DELETE FROM tasks').run()
    const insertTask = d.prepare(
      `INSERT INTO tasks (id, project_name, title, description, status, "order", assigned_session_id, assigned_agent, agent_session_id, branch, use_worktree, created_at, updated_at, completed_at, source_connector_id, source_external_url, source_external_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    for (const t of config.tasks ?? []) {
      insertTask.run(
        t.id,
        t.projectName,
        t.title,
        t.description,
        t.status,
        t.order,
        t.assignedSessionId ?? null,
        t.assignedAgent ?? null,
        t.agentSessionId ?? null,
        t.branch ?? null,
        t.useWorktree ? 1 : 0,
        t.createdAt,
        t.updatedAt,
        t.completedAt ?? null,
        t.sourceConnectorId ?? null,
        t.sourceExternalUrl ?? null,
        t.sourceExternalId ?? null
      )
    }

    // Workspaces
    d.prepare('DELETE FROM workspaces').run()
    const insertWorkspace = d.prepare(
      `INSERT INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)`
    )
    for (const ws of config.workspaces ?? [DEFAULT_WORKSPACE]) {
      insertWorkspace.run(ws.id, ws.name, ws.icon ?? null, ws.iconColor ?? null, ws.order)
    }
  })

  run()
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Tasks
// ---------------------------------------------------------------------------

export function dbListTasks(projectName?: string, status?: string): TaskConfig[] {
  const d = getDb()
  let sql = 'SELECT * FROM tasks'
  const params: string[] = []
  const clauses: string[] = []
  if (projectName) {
    clauses.push('project_name = ?')
    params.push(projectName)
  }
  if (status) {
    clauses.push('status = ?')
    params.push(status)
  }
  if (clauses.length) sql += ' WHERE ' + clauses.join(' AND ')
  sql += ' ORDER BY "order"'
  const rows = d.prepare(sql).all(...params) as Array<{
    id: string
    project_name: string
    title: string
    description: string
    status: string
    order: number
    assigned_session_id: string | null
    assigned_agent: string | null
    agent_session_id: string | null
    branch: string | null
    use_worktree: number | null
    created_at: string
    updated_at: string
    completed_at: string | null
  }>
  return rows.map(rowToTask)
}

export function dbGetTask(id: string): TaskConfig | null {
  const row = getDb().prepare('SELECT * FROM tasks WHERE id = ?').get(id) as
    | {
        id: string
        project_name: string
        title: string
        description: string
        status: string
        order: number
        assigned_session_id: string | null
        assigned_agent: string | null
        agent_session_id: string | null
        branch: string | null
        use_worktree: number | null
        created_at: string
        updated_at: string
        completed_at: string | null
      }
    | undefined
  return row ? rowToTask(row) : null
}

export function dbInsertTask(task: TaskConfig): void {
  getDb()
    .prepare(
      `INSERT INTO tasks (id, project_name, title, description, status, "order", assigned_session_id, assigned_agent, agent_session_id, branch, use_worktree, created_at, updated_at, completed_at, source_connector_id, source_external_url, source_external_id)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    .run(
      task.id,
      task.projectName,
      task.title,
      task.description,
      task.status,
      task.order,
      task.assignedSessionId ?? null,
      task.assignedAgent ?? null,
      task.agentSessionId ?? null,
      task.branch ?? null,
      task.useWorktree ? 1 : 0,
      task.createdAt,
      task.updatedAt,
      task.completedAt ?? null,
      task.sourceConnectorId ?? null,
      task.sourceExternalUrl ?? null,
      task.sourceExternalId ?? null
    )
}

export function dbUpdateTask(id: string, updates: Partial<TaskConfig>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.title !== undefined) {
    sets.push('title = ?')
    params.push(updates.title)
  }
  if (updates.description !== undefined) {
    sets.push('description = ?')
    params.push(updates.description)
  }
  if (updates.status !== undefined) {
    sets.push('status = ?')
    params.push(updates.status)
  }
  if (updates.order !== undefined) {
    sets.push('"order" = ?')
    params.push(updates.order)
  }
  if (updates.branch !== undefined) {
    sets.push('branch = ?')
    params.push(updates.branch)
  }
  if (updates.useWorktree !== undefined) {
    sets.push('use_worktree = ?')
    params.push(updates.useWorktree ? 1 : 0)
  }
  if (updates.assignedAgent !== undefined) {
    sets.push('assigned_agent = ?')
    params.push(updates.assignedAgent)
  }
  if (updates.assignedSessionId !== undefined) {
    sets.push('assigned_session_id = ?')
    params.push(updates.assignedSessionId)
  }
  if (updates.agentSessionId !== undefined) {
    sets.push('agent_session_id = ?')
    params.push(updates.agentSessionId)
  }
  if (updates.updatedAt !== undefined) {
    sets.push('updated_at = ?')
    params.push(updates.updatedAt)
  }
  if ('completedAt' in updates) {
    sets.push('completed_at = ?')
    params.push(updates.completedAt ?? null)
  }
  if (updates.sourceConnectorId !== undefined) {
    sets.push('source_connector_id = ?')
    params.push(updates.sourceConnectorId)
  }
  if (updates.sourceExternalUrl !== undefined) {
    sets.push('source_external_url = ?')
    params.push(updates.sourceExternalUrl)
  }
  if (updates.sourceExternalId !== undefined) {
    sets.push('source_external_id = ?')
    params.push(updates.sourceExternalId)
  }
  if (sets.length === 0) return
  params.push(id)
  getDb()
    .prepare(`UPDATE tasks SET ${sets.join(', ')} WHERE id = ?`)
    .run(...params)
}

export function dbDeleteTask(id: string): void {
  getDb().prepare('DELETE FROM tasks WHERE id = ?').run(id)
}

export function dbGetMaxTaskOrder(projectName: string): number {
  const row = getDb()
    .prepare('SELECT MAX("order") as m FROM tasks WHERE project_name = ?')
    .get(projectName) as { m: number | null }
  return row.m ?? -1
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Source Connections
// ---------------------------------------------------------------------------

interface SourceConnectionRow {
  id: string
  connector_id: string
  name: string
  filters: string
  sync_interval_minutes: number
  status_mapping: string
  execution_project: string | null
  last_sync_at: string | null
  last_sync_error: string | null
  sync_cursor: string | null
  created_at: string
}

function rowToSourceConnection(r: SourceConnectionRow): SourceConnection {
  return {
    id: r.id,
    connectorId: r.connector_id,
    name: r.name,
    filters: JSON.parse(r.filters),
    syncIntervalMinutes: r.sync_interval_minutes,
    statusMapping: JSON.parse(r.status_mapping),
    ...(r.execution_project != null && { executionProject: r.execution_project }),
    ...(r.last_sync_at != null && { lastSyncAt: r.last_sync_at }),
    ...(r.last_sync_error != null && { lastSyncError: r.last_sync_error }),
    ...(r.sync_cursor != null && { syncCursor: r.sync_cursor }),
    createdAt: r.created_at
  }
}

export function dbListSourceConnections(connectorId?: string): SourceConnection[] {
  const d = getDb()
  if (connectorId) {
    const rows = d
      .prepare('SELECT * FROM source_connections WHERE connector_id = ?')
      .all(connectorId) as SourceConnectionRow[]
    return rows.map(rowToSourceConnection)
  }
  const rows = d.prepare('SELECT * FROM source_connections').all() as SourceConnectionRow[]
  return rows.map(rowToSourceConnection)
}

export function dbGetSourceConnection(id: string): SourceConnection | null {
  const row = getDb().prepare('SELECT * FROM source_connections WHERE id = ?').get(id) as
    | SourceConnectionRow
    | undefined
  return row ? rowToSourceConnection(row) : null
}

export function dbInsertSourceConnection(conn: SourceConnection): void {
  getDb()
    .prepare(
      `INSERT INTO source_connections (id, connector_id, name, filters, sync_interval_minutes, status_mapping, execution_project, last_sync_at, last_sync_error, sync_cursor, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    .run(
      conn.id,
      conn.connectorId,
      conn.name,
      JSON.stringify(conn.filters),
      conn.syncIntervalMinutes,
      JSON.stringify(conn.statusMapping),
      conn.executionProject ?? null,
      conn.lastSyncAt ?? null,
      conn.lastSyncError ?? null,
      conn.syncCursor ?? null,
      conn.createdAt
    )
}

export function dbUpdateSourceConnection(id: string, updates: Partial<SourceConnection>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.name !== undefined) {
    sets.push('name = ?')
    params.push(updates.name)
  }
  if (updates.filters !== undefined) {
    sets.push('filters = ?')
    params.push(JSON.stringify(updates.filters))
  }
  if (updates.syncIntervalMinutes !== undefined) {
    sets.push('sync_interval_minutes = ?')
    params.push(updates.syncIntervalMinutes)
  }
  if (updates.statusMapping !== undefined) {
    sets.push('status_mapping = ?')
    params.push(JSON.stringify(updates.statusMapping))
  }
  if (updates.executionProject !== undefined) {
    sets.push('execution_project = ?')
    params.push(updates.executionProject)
  }
  if ('lastSyncAt' in updates) {
    sets.push('last_sync_at = ?')
    params.push(updates.lastSyncAt ?? null)
  }
  if ('lastSyncError' in updates) {
    sets.push('last_sync_error = ?')
    params.push(updates.lastSyncError ?? null)
  }
  if ('syncCursor' in updates) {
    sets.push('sync_cursor = ?')
    params.push(updates.syncCursor ?? null)
  }
  if (sets.length === 0) return
  params.push(id)
  getDb()
    .prepare(`UPDATE source_connections SET ${sets.join(', ')} WHERE id = ?`)
    .run(...params)
}

export function dbDeleteSourceConnection(id: string): void {
  getDb().prepare('DELETE FROM source_connections WHERE id = ?').run(id)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Task Source Links
// ---------------------------------------------------------------------------

interface TaskSourceLinkRow {
  task_id: string
  connection_id: string
  connector_id: string
  external_id: string
  external_url: string
  source_status_raw: string
  source_updated_at: string
  last_synced_at: string
  conflict_state: string
}

function rowToTaskSourceLink(r: TaskSourceLinkRow): TaskSourceLink {
  return {
    taskId: r.task_id,
    connectionId: r.connection_id,
    connectorId: r.connector_id,
    externalId: r.external_id,
    externalUrl: r.external_url,
    sourceStatusRaw: r.source_status_raw,
    sourceUpdatedAt: r.source_updated_at,
    lastSyncedAt: r.last_synced_at,
    conflictState: r.conflict_state as TaskSourceLink['conflictState']
  }
}

export function dbGetTaskSourceLink(taskId: string): TaskSourceLink | null {
  const row = getDb().prepare('SELECT * FROM task_source_links WHERE task_id = ?').get(taskId) as
    | TaskSourceLinkRow
    | undefined
  return row ? rowToTaskSourceLink(row) : null
}

export function dbGetTaskSourceLinkByExternalId(
  connectionId: string,
  externalId: string
): TaskSourceLink | null {
  const row = getDb()
    .prepare('SELECT * FROM task_source_links WHERE connection_id = ? AND external_id = ?')
    .get(connectionId, externalId) as TaskSourceLinkRow | undefined
  return row ? rowToTaskSourceLink(row) : null
}

/**
 * Fallback lookup for orphan re-linking: find a task whose own
 * sourceConnectorId/sourceExternalId matches, even if its task_source_links
 * row is missing (e.g. because a prior connection was deleted and cascaded
 * the link). Used by the import path to re-adopt existing tasks instead of
 * creating duplicates.
 */
export function dbFindTaskByConnectorExternalId(
  connectorId: string,
  externalId: string
): TaskConfig | null {
  const row = getDb()
    .prepare('SELECT * FROM tasks WHERE source_connector_id = ? AND source_external_id = ? LIMIT 1')
    .get(connectorId, externalId) as
    | {
        id: string
        project_name: string
        title: string
        description: string
        status: string
        [k: string]: unknown
      }
    | undefined
  if (!row) return null
  return dbGetTask(row.id)
}

export function dbListTaskSourceLinks(connectionId: string): TaskSourceLink[] {
  const rows = getDb()
    .prepare('SELECT * FROM task_source_links WHERE connection_id = ?')
    .all(connectionId) as TaskSourceLinkRow[]
  return rows.map(rowToTaskSourceLink)
}

export function dbInsertTaskSourceLink(link: TaskSourceLink): void {
  getDb()
    .prepare(
      `INSERT INTO task_source_links (task_id, connection_id, connector_id, external_id, external_url, source_status_raw, source_updated_at, last_synced_at, conflict_state)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    .run(
      link.taskId,
      link.connectionId,
      link.connectorId,
      link.externalId,
      link.externalUrl,
      link.sourceStatusRaw,
      link.sourceUpdatedAt,
      link.lastSyncedAt,
      link.conflictState
    )
}

export function dbUpdateTaskSourceLink(taskId: string, updates: Partial<TaskSourceLink>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.sourceStatusRaw !== undefined) {
    sets.push('source_status_raw = ?')
    params.push(updates.sourceStatusRaw)
  }
  if (updates.sourceUpdatedAt !== undefined) {
    sets.push('source_updated_at = ?')
    params.push(updates.sourceUpdatedAt)
  }
  if (updates.lastSyncedAt !== undefined) {
    sets.push('last_synced_at = ?')
    params.push(updates.lastSyncedAt)
  }
  if (updates.conflictState !== undefined) {
    sets.push('conflict_state = ?')
    params.push(updates.conflictState)
  }
  if (sets.length === 0) return
  params.push(taskId)
  getDb()
    .prepare(`UPDATE task_source_links SET ${sets.join(', ')} WHERE task_id = ?`)
    .run(...params)
}

export function dbDeleteTaskSourceLink(taskId: string): void {
  getDb().prepare('DELETE FROM task_source_links WHERE task_id = ?').run(taskId)
}

export function dbListProjects(): ProjectConfig[] {
  const rows = getDb().prepare('SELECT * FROM projects').all() as Array<{
    name: string
    path: string
    preferred_agents: string
    icon: string | null
    icon_color: string | null
    host_ids: string | null
    workspace_id: string | null
  }>
  return rows.map(rowToProject)
}

export function dbGetProject(name: string): ProjectConfig | null {
  const row = getDb().prepare('SELECT * FROM projects WHERE name = ?').get(name) as
    | {
        name: string
        path: string
        preferred_agents: string
        icon: string | null
        icon_color: string | null
        host_ids: string | null
        workspace_id: string | null
      }
    | undefined
  return row ? rowToProject(row) : null
}

export function dbInsertProject(project: ProjectConfig): void {
  getDb()
    .prepare(
      'INSERT INTO projects (name, path, preferred_agents, icon, icon_color, host_ids, workspace_id) VALUES (?, ?, ?, ?, ?, ?, ?)'
    )
    .run(
      project.name,
      project.path,
      JSON.stringify(project.preferredAgents),
      project.icon ?? null,
      project.iconColor ?? null,
      project.hostIds ? JSON.stringify(project.hostIds) : null,
      project.workspaceId ?? 'personal'
    )
}

export function dbUpdateProject(name: string, updates: Partial<ProjectConfig>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.path !== undefined) {
    sets.push('path = ?')
    params.push(updates.path)
  }
  if (updates.preferredAgents !== undefined) {
    sets.push('preferred_agents = ?')
    params.push(JSON.stringify(updates.preferredAgents))
  }
  if (updates.icon !== undefined) {
    sets.push('icon = ?')
    params.push(updates.icon)
  }
  if (updates.iconColor !== undefined) {
    sets.push('icon_color = ?')
    params.push(updates.iconColor)
  }
  if (updates.hostIds !== undefined) {
    sets.push('host_ids = ?')
    params.push(JSON.stringify(updates.hostIds))
  }
  if (updates.workspaceId !== undefined) {
    sets.push('workspace_id = ?')
    params.push(updates.workspaceId)
  }
  if (sets.length === 0) return
  params.push(name)
  getDb()
    .prepare(`UPDATE projects SET ${sets.join(', ')} WHERE name = ?`)
    .run(...params)
}

export function dbDeleteProject(name: string): void {
  const d = getDb()
  d.transaction(() => {
    d.prepare('DELETE FROM tasks WHERE project_name = ?').run(name)
    d.prepare('DELETE FROM projects WHERE name = ?').run(name)
  })()
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Workflows
// ---------------------------------------------------------------------------

export function dbListWorkflows(): WorkflowDefinition[] {
  const rows = getDb().prepare('SELECT * FROM workflows').all() as Array<{
    id: string
    name: string
    icon: string
    icon_color: string
    nodes: string
    edges: string
    enabled: number
    last_run_at: string | null
    last_run_status: string | null
    stagger_delay_ms: number | null
    workspace_id: string | null
  }>
  return rows.map(rowToWorkflow)
}

export function dbGetWorkflow(id: string): WorkflowDefinition | null {
  const row = getDb().prepare('SELECT * FROM workflows WHERE id = ?').get(id) as
    | {
        id: string
        name: string
        icon: string
        icon_color: string
        nodes: string
        edges: string
        enabled: number
        last_run_at: string | null
        last_run_status: string | null
        stagger_delay_ms: number | null
        workspace_id: string | null
      }
    | undefined
  return row ? rowToWorkflow(row) : null
}

export function dbInsertWorkflow(workflow: WorkflowDefinition): void {
  getDb()
    .prepare(
      `INSERT INTO workflows (id, name, icon, icon_color, nodes, edges, enabled, last_run_at, last_run_status, stagger_delay_ms, workspace_id)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    .run(
      workflow.id,
      workflow.name,
      workflow.icon,
      workflow.iconColor,
      JSON.stringify(workflow.nodes),
      JSON.stringify(workflow.edges),
      workflow.enabled ? 1 : 0,
      workflow.lastRunAt ?? null,
      workflow.lastRunStatus ?? null,
      workflow.staggerDelayMs ?? null,
      workflow.workspaceId ?? 'personal'
    )
}

export function dbUpdateWorkflow(id: string, updates: Partial<WorkflowDefinition>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.name !== undefined) {
    sets.push('name = ?')
    params.push(updates.name)
  }
  if (updates.nodes !== undefined) {
    sets.push('nodes = ?')
    params.push(JSON.stringify(updates.nodes))
  }
  if (updates.edges !== undefined) {
    sets.push('edges = ?')
    params.push(JSON.stringify(updates.edges))
  }
  if (updates.icon !== undefined) {
    sets.push('icon = ?')
    params.push(updates.icon)
  }
  if (updates.iconColor !== undefined) {
    sets.push('icon_color = ?')
    params.push(updates.iconColor)
  }
  if (updates.enabled !== undefined) {
    sets.push('enabled = ?')
    params.push(updates.enabled ? 1 : 0)
  }
  if (updates.staggerDelayMs !== undefined) {
    sets.push('stagger_delay_ms = ?')
    params.push(updates.staggerDelayMs)
  }
  if (updates.workspaceId !== undefined) {
    sets.push('workspace_id = ?')
    params.push(updates.workspaceId)
  }
  if (sets.length === 0) return
  params.push(id)
  getDb()
    .prepare(`UPDATE workflows SET ${sets.join(', ')} WHERE id = ?`)
    .run(...params)
}

export function dbDeleteWorkflow(id: string): void {
  getDb().prepare('DELETE FROM workflows WHERE id = ?').run(id)
}

// ---------------------------------------------------------------------------
// Targeted CRUD: Workspaces
// ---------------------------------------------------------------------------

export function dbListWorkspaces(): WorkspaceConfig[] {
  const rows = getDb().prepare('SELECT * FROM workspaces ORDER BY "order"').all() as Array<{
    id: string
    name: string
    icon: string | null
    icon_color: string | null
    order: number
  }>
  return rows.map(rowToWorkspace)
}

export function dbInsertWorkspace(workspace: WorkspaceConfig): void {
  getDb()
    .prepare(`INSERT INTO workspaces (id, name, icon, icon_color, "order") VALUES (?, ?, ?, ?, ?)`)
    .run(
      workspace.id,
      workspace.name,
      workspace.icon ?? null,
      workspace.iconColor ?? null,
      workspace.order
    )
}

export function dbUpdateWorkspace(id: string, updates: Partial<WorkspaceConfig>): void {
  const sets: string[] = []
  const params: unknown[] = []
  if (updates.name !== undefined) {
    sets.push('name = ?')
    params.push(updates.name)
  }
  if (updates.icon !== undefined) {
    sets.push('icon = ?')
    params.push(updates.icon)
  }
  if (updates.iconColor !== undefined) {
    sets.push('icon_color = ?')
    params.push(updates.iconColor)
  }
  if (updates.order !== undefined) {
    sets.push('"order" = ?')
    params.push(updates.order)
  }
  if (sets.length === 0) return
  params.push(id)
  getDb()
    .prepare(`UPDATE workspaces SET ${sets.join(', ')} WHERE id = ?`)
    .run(...params)
}

export function dbDeleteWorkspace(id: string): void {
  const d = getDb()
  d.transaction(() => {
    // Move projects and workflows to 'personal' before deleting
    d.prepare("UPDATE projects SET workspace_id = 'personal' WHERE workspace_id = ?").run(id)
    d.prepare("UPDATE workflows SET workspace_id = 'personal' WHERE workspace_id = ?").run(id)
    d.prepare('DELETE FROM workspaces WHERE id = ?').run(id)
  })()
}

// ---------------------------------------------------------------------------
// Targeted CRUD: SSH Keys
// ---------------------------------------------------------------------------

export function dbSaveSSHKey(key: SSHKey): void {
  getDb()
    .prepare(
      'INSERT INTO ssh_keys (id, label, encrypted_private_key, public_key, certificate, key_type, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)'
    )
    .run(
      key.id,
      key.label,
      key.encryptedPrivateKey,
      key.publicKey ?? null,
      key.certificate ?? null,
      key.keyType ?? null,
      key.createdAt
    )
}

export function dbListSSHKeys(): SSHKeyMeta[] {
  const rows = getDb()
    .prepare('SELECT id, label, key_type, public_key, created_at FROM ssh_keys')
    .all() as Array<{
    id: string
    label: string
    key_type: string | null
    public_key: string | null
    created_at: string
  }>
  return rows.map((r) => ({
    id: r.id,
    label: r.label,
    ...(r.key_type != null && { keyType: r.key_type }),
    ...(r.public_key != null && { publicKey: r.public_key }),
    createdAt: r.created_at
  }))
}

export function dbGetSSHKey(id: string): SSHKey | null {
  const row = getDb().prepare('SELECT * FROM ssh_keys WHERE id = ?').get(id) as
    | {
        id: string
        label: string
        encrypted_private_key: string
        public_key: string | null
        certificate: string | null
        key_type: string | null
        created_at: string
      }
    | undefined
  if (!row) return null
  return {
    id: row.id,
    label: row.label,
    encryptedPrivateKey: row.encrypted_private_key,
    ...(row.public_key != null && { publicKey: row.public_key }),
    ...(row.certificate != null && { certificate: row.certificate }),
    ...(row.key_type != null && { keyType: row.key_type }),
    createdAt: row.created_at
  }
}

export function dbDeleteSSHKey(id: string): void {
  getDb().prepare('DELETE FROM ssh_keys WHERE id = ?').run(id)
}

// ---------------------------------------------------------------------------
// Row mappers (shared between loadConfig and targeted queries)
// ---------------------------------------------------------------------------

function rowToTask(r: {
  id: string
  project_name: string
  title: string
  description: string
  status: string
  order: number
  assigned_session_id: string | null
  assigned_agent: string | null
  agent_session_id: string | null
  branch: string | null
  use_worktree: number | null
  created_at: string
  updated_at: string
  completed_at: string | null
  source_connector_id?: string | null
  source_external_url?: string | null
  source_external_id?: string | null
}): TaskConfig {
  return {
    id: r.id,
    projectName: r.project_name,
    title: r.title,
    description: r.description,
    status: r.status as TaskConfig['status'],
    order: r.order,
    ...(r.assigned_session_id != null && { assignedSessionId: r.assigned_session_id }),
    ...(r.assigned_agent != null && { assignedAgent: r.assigned_agent as AiAgentType }),
    ...(r.agent_session_id != null && { agentSessionId: r.agent_session_id }),
    ...(r.branch != null && { branch: r.branch }),
    ...(r.use_worktree != null && r.use_worktree !== 0 && { useWorktree: true }),
    createdAt: r.created_at,
    updatedAt: r.updated_at,
    ...(r.completed_at != null && { completedAt: r.completed_at }),
    ...(r.source_connector_id != null && { sourceConnectorId: r.source_connector_id }),
    ...(r.source_external_url != null && { sourceExternalUrl: r.source_external_url }),
    ...(r.source_external_id != null && {
      sourceExternalId: r.source_external_id
    })
  }
}

function rowToProject(r: {
  name: string
  path: string
  preferred_agents: string
  icon: string | null
  icon_color: string | null
  host_ids: string | null
  workspace_id?: string | null
}): ProjectConfig {
  return {
    name: r.name,
    path: r.path,
    preferredAgents: JSON.parse(r.preferred_agents) as AiAgentType[],
    ...(r.icon != null && { icon: r.icon }),
    ...(r.icon_color != null && { iconColor: r.icon_color }),
    ...(r.host_ids != null && { hostIds: JSON.parse(r.host_ids) as string[] }),
    workspaceId: r.workspace_id ?? 'personal'
  }
}

function rowToWorkflow(r: {
  id: string
  name: string
  icon: string
  icon_color: string
  nodes: string
  edges: string
  enabled: number
  last_run_at: string | null
  last_run_status: string | null
  stagger_delay_ms: number | null
  workspace_id?: string | null
}): WorkflowDefinition {
  return {
    id: r.id,
    name: r.name,
    icon: r.icon,
    iconColor: r.icon_color,
    nodes: JSON.parse(r.nodes),
    edges: JSON.parse(r.edges),
    enabled: r.enabled === 1,
    ...(r.last_run_at != null && { lastRunAt: r.last_run_at }),
    ...(r.last_run_status != null && { lastRunStatus: r.last_run_status as 'success' | 'error' }),
    ...(r.stagger_delay_ms != null && { staggerDelayMs: r.stagger_delay_ms }),
    workspaceId: r.workspace_id ?? 'personal'
  }
}

function rowToWorkspace(r: {
  id: string
  name: string
  icon: string | null
  icon_color: string | null
  order: number
}): WorkspaceConfig {
  return {
    id: r.id,
    name: r.name,
    ...(r.icon != null && { icon: r.icon }),
    ...(r.icon_color != null && { iconColor: r.icon_color }),
    order: r.order
  }
}

// ---------------------------------------------------------------------------
// Granular updates (avoids full load/save cycle for hot paths)
// ---------------------------------------------------------------------------

export function updateWorkflowRunStatus(
  id: string,
  lastRunAt: string,
  lastRunStatus: string
): void {
  getDb()
    .prepare('UPDATE workflows SET last_run_at = ?, last_run_status = ? WHERE id = ?')
    .run(lastRunAt, lastRunStatus, id)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export function saveSessions(sessions: TerminalSession[]): void {
  const d = getDb()
  const savedAt = Date.now()

  const run = d.transaction(() => {
    d.prepare('DELETE FROM sessions').run()
    const insert = d.prepare(
      `INSERT INTO sessions (id, agent_type, project_name, project_path, status, created_at, pid, display_name, branch, worktree_path, is_worktree, remote_host_id, remote_host_label, hook_session_id, status_source, saved_at, sort_order, worktree_name, agent_session_id)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    for (let i = 0; i < sessions.length; i++) {
      const s = sessions[i]
      insert.run(
        s.id,
        s.agentType,
        s.projectName,
        s.projectPath,
        s.status,
        s.createdAt,
        s.pid,
        s.displayName ?? null,
        s.branch ?? null,
        s.worktreePath ?? null,
        s.isWorktree ? 1 : 0,
        s.remoteHostId ?? null,
        s.remoteHostLabel ?? null,
        s.hookSessionId ?? null,
        s.statusSource ?? null,
        savedAt,
        i,
        s.worktreeName ?? null,
        s.agentSessionId ?? null
      )
    }
  })

  run()
}

export function getPreviousSessions(): TerminalSession[] {
  const rows = getDb().prepare('SELECT * FROM sessions ORDER BY sort_order ASC').all() as Array<{
    id: string
    agent_type: string
    project_name: string
    project_path: string
    status: string
    created_at: number
    pid: number
    display_name: string | null
    branch: string | null
    worktree_path: string | null
    is_worktree: number | null
    remote_host_id: string | null
    remote_host_label: string | null
    hook_session_id: string | null
    status_source: string | null
    saved_at: number | null
    worktree_name: string | null
    agent_session_id: string | null
  }>
  return rows.map((r) => ({
    id: r.id,
    agentType: r.agent_type as AgentType,
    projectName: r.project_name,
    projectPath: r.project_path,
    status: r.status as TerminalSession['status'],
    createdAt: r.created_at,
    pid: r.pid,
    ...(r.display_name != null && { displayName: r.display_name }),
    ...(r.branch != null && { branch: r.branch }),
    ...(r.worktree_path != null && { worktreePath: r.worktree_path }),
    ...(r.is_worktree != null && r.is_worktree !== 0 && { isWorktree: true }),
    ...(r.remote_host_id != null && { remoteHostId: r.remote_host_id }),
    ...(r.remote_host_label != null && { remoteHostLabel: r.remote_host_label }),
    ...(r.hook_session_id != null && { hookSessionId: r.hook_session_id }),
    ...(r.status_source != null && {
      statusSource: r.status_source as TerminalSession['statusSource']
    }),
    ...(r.worktree_name != null && { worktreeName: r.worktree_name }),
    ...(r.agent_session_id != null && { agentSessionId: r.agent_session_id })
  }))
}

export function clearSessions(): void {
  getDb().prepare('DELETE FROM sessions').run()
}

// ---------------------------------------------------------------------------
// Schedule log
// ---------------------------------------------------------------------------

export function addScheduleLogEntry(entry: ScheduleLogEntry): void {
  const d = getDb()
  d.prepare(
    `INSERT INTO schedule_log (workflow_id, workflow_name, executed_at, status, sessions_launched, error)
     VALUES (?, ?, ?, ?, ?, ?)`
  ).run(
    entry.workflowId,
    entry.workflowName,
    entry.executedAt,
    entry.status,
    entry.sessionsLaunched,
    entry.error ?? null
  )

  // Trim to max entries
  const count = (d.prepare('SELECT COUNT(*) as c FROM schedule_log').get() as { c: number }).c
  if (count > MAX_LOG_ENTRIES) {
    d.prepare(
      `DELETE FROM schedule_log WHERE id IN (
        SELECT id FROM schedule_log ORDER BY id ASC LIMIT ?
      )`
    ).run(count - MAX_LOG_ENTRIES)
  }
}

export function getScheduleLogEntries(workflowId?: string): ScheduleLogEntry[] {
  const d = getDb()
  let rows: Array<{
    workflow_id: string
    workflow_name: string
    executed_at: string
    status: string
    sessions_launched: number
    error: string | null
  }>

  if (workflowId) {
    rows = d
      .prepare('SELECT * FROM schedule_log WHERE workflow_id = ? ORDER BY id')
      .all(workflowId) as typeof rows
  } else {
    rows = d.prepare('SELECT * FROM schedule_log ORDER BY id').all() as typeof rows
  }

  return rows.map((r) => ({
    workflowId: r.workflow_id,
    workflowName: r.workflow_name,
    executedAt: r.executed_at,
    status: r.status as ScheduleLogEntry['status'],
    sessionsLaunched: r.sessions_launched,
    ...(r.error != null && { error: r.error })
  }))
}

export function clearScheduleLog(): void {
  getDb().prepare('DELETE FROM schedule_log').run()
}

// ---------------------------------------------------------------------------
// Workflow runs
// ---------------------------------------------------------------------------

const MAX_WORKFLOW_RUNS = 50

export function saveWorkflowRun(execution: WorkflowExecution): void {
  const d = getDb()

  const run = d.transaction(() => {
    d.prepare(
      `INSERT OR REPLACE INTO workflow_runs (id, workflow_id, started_at, completed_at, status, trigger_task_id)
       VALUES (?, ?, ?, ?, ?, ?)`
    ).run(
      execution.workflowId + ':' + execution.startedAt,
      execution.workflowId,
      execution.startedAt,
      execution.completedAt ?? null,
      execution.status,
      execution.triggerTaskId ?? null
    )

    const runId = execution.workflowId + ':' + execution.startedAt

    // Delete existing nodes for this run (for upsert behavior)
    d.prepare('DELETE FROM workflow_run_nodes WHERE run_id = ?').run(runId)

    const insertNode = d.prepare(
      `INSERT INTO workflow_run_nodes (run_id, node_id, status, started_at, completed_at, session_id, error, logs, task_id, agent_session_id, agent_type, project_name, project_path, approved_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    )
    for (const ns of execution.nodeStates) {
      insertNode.run(
        runId,
        ns.nodeId,
        ns.status,
        ns.startedAt ?? null,
        ns.completedAt ?? null,
        ns.sessionId ?? null,
        ns.error ?? null,
        ns.logs ?? null,
        ns.taskId ?? null,
        ns.agentSessionId ?? null,
        ns.agentType ?? null,
        ns.projectName ?? null,
        ns.projectPath ?? null,
        ns.approvedAt ?? null
      )
    }

    // Trim old runs for this workflow
    const count = (
      d
        .prepare('SELECT COUNT(*) as c FROM workflow_runs WHERE workflow_id = ?')
        .get(execution.workflowId) as { c: number }
    ).c
    if (count > MAX_WORKFLOW_RUNS) {
      d.prepare(
        `DELETE FROM workflow_runs WHERE id IN (
          SELECT id FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at ASC LIMIT ?
        )`
      ).run(execution.workflowId, count - MAX_WORKFLOW_RUNS)
    }
  })

  run()
}

type WorkflowRunNodeRow = {
  run_id: string
  node_id: string
  status: string
  started_at: string | null
  completed_at: string | null
  session_id: string | null
  error: string | null
  logs: string | null
  task_id: string | null
  agent_session_id: string | null
  agent_type: string | null
  project_name: string | null
  project_path: string | null
  approved_at: string | null
}

function mapNodeRow(n: WorkflowRunNodeRow): NodeExecutionState {
  return {
    nodeId: n.node_id,
    status: n.status as NodeExecutionState['status'],
    ...(n.started_at != null && { startedAt: n.started_at }),
    ...(n.completed_at != null && { completedAt: n.completed_at }),
    ...(n.session_id != null && { sessionId: n.session_id }),
    ...(n.error != null && { error: n.error }),
    ...(n.logs != null && { logs: n.logs }),
    ...(n.task_id != null && { taskId: n.task_id }),
    ...(n.agent_session_id != null && { agentSessionId: n.agent_session_id }),
    ...(n.agent_type != null && { agentType: n.agent_type as NodeExecutionState['agentType'] }),
    ...(n.project_name != null && { projectName: n.project_name }),
    ...(n.project_path != null && { projectPath: n.project_path }),
    ...(n.approved_at != null && { approvedAt: n.approved_at })
  }
}

function fetchNodesByRunIds(
  d: Database.Database,
  runIds: string[]
): Map<string, NodeExecutionState[]> {
  if (runIds.length === 0) return new Map()
  const placeholders = runIds.map(() => '?').join(',')
  const rows = d
    .prepare(`SELECT * FROM workflow_run_nodes WHERE run_id IN (${placeholders})`)
    .all(...runIds) as WorkflowRunNodeRow[]
  const out = new Map<string, NodeExecutionState[]>()
  for (const r of rows) {
    const bucket = out.get(r.run_id)
    const node = mapNodeRow(r)
    if (bucket) bucket.push(node)
    else out.set(r.run_id, [node])
  }
  return out
}

type RunRow = {
  id: string
  workflow_id: string
  started_at: string
  completed_at: string | null
  status: string
  trigger_task_id: string | null
  workflow_name?: string | null
}

function mapRunRows(
  rows: RunRow[],
  nodesByRun: Map<string, NodeExecutionState[]>
): (WorkflowExecution & { workflowName?: string })[] {
  return rows.map((r) => ({
    workflowId: r.workflow_id,
    startedAt: r.started_at,
    ...(r.completed_at != null && { completedAt: r.completed_at }),
    status: r.status as WorkflowExecution['status'],
    ...(r.trigger_task_id != null && { triggerTaskId: r.trigger_task_id }),
    ...(r.workflow_name != null && { workflowName: r.workflow_name }),
    nodeStates: nodesByRun.get(r.id) ?? []
  }))
}

export function listWorkflowRuns(workflowId: string, limit = 20): WorkflowExecution[] {
  const d = getDb()
  const rows = d
    .prepare('SELECT * FROM workflow_runs WHERE workflow_id = ? ORDER BY started_at DESC LIMIT ?')
    .all(workflowId, limit) as RunRow[]
  return mapRunRows(
    rows,
    fetchNodesByRunIds(
      d,
      rows.map((r) => r.id)
    )
  )
}

export function listWorkflowRunsByTask(
  taskId: string,
  limit = 20
): (WorkflowExecution & { workflowName?: string })[] {
  const d = getDb()
  const rows = d
    .prepare(
      `SELECT DISTINCT wr.*, w.name as workflow_name
       FROM workflow_runs wr
       LEFT JOIN workflows w ON w.id = wr.workflow_id
       WHERE wr.trigger_task_id = ?
          OR wr.id IN (SELECT run_id FROM workflow_run_nodes WHERE task_id = ?)
       ORDER BY wr.started_at DESC
       LIMIT ?`
    )
    .all(taskId, taskId, limit) as RunRow[]
  return mapRunRows(
    rows,
    fetchNodesByRunIds(
      d,
      rows.map((r) => r.id)
    )
  )
}

/**
 * Runs in `running` state. Used at renderer startup to reconcile orphaned
 * runs: when the renderer reloads mid-execution, headless agents in the main
 * process keep going but the in-memory exit-promise dies, leaving the run
 * stuck. The reconciler closes these out against `session_events`.
 */
export function listRunningRuns(): WorkflowExecution[] {
  const d = getDb()
  const rows = d
    .prepare(
      `SELECT * FROM workflow_runs
       WHERE status = 'running'
       ORDER BY started_at DESC`
    )
    .all() as RunRow[]
  return mapRunRows(
    rows,
    fetchNodesByRunIds(
      d,
      rows.map((r) => r.id)
    )
  )
}

// Surfaces every run that has at least one waiting node — small in practice
// because gates pause execution. No LIMIT is intentional so the badge count
// matches the real backlog. If this ever grows, cap with a LIMIT here and
// chunk `fetchNodesByRunIds` to stay under SQLite's IN-clause variable cap.
export function listRunsWithWaitingGates(): WorkflowExecution[] {
  const d = getDb()
  const rows = d
    .prepare(
      `SELECT DISTINCT wr.*
       FROM workflow_runs wr
       JOIN workflow_run_nodes wrn ON wrn.run_id = wr.id
       WHERE wrn.status = 'waiting'
       ORDER BY wr.started_at DESC`
    )
    .all() as RunRow[]
  return mapRunRows(
    rows,
    fetchNodesByRunIds(
      d,
      rows.map((r) => r.id)
    )
  )
}

/**
 * Cross-workflow run history for the Workflows → All runs view. Joins on
 * the workflows table so the renderer can display the workflow name without
 * a second lookup. When `workspaceId` is provided, restricts to workflows
 * in that workspace; otherwise returns runs across every workflow.
 */
export function listAllWorkflowRuns(
  workspaceId?: string,
  limit = 50
): (WorkflowExecution & { workflowName?: string })[] {
  const d = getDb()
  // Clamp to keep the IN-clause below SQLite's default 999-variable cap
  // when fetching node rows for each run.
  const cappedLimit = Math.max(1, Math.min(limit, 500))

  // When filtering by workspace, exclude orphaned runs (workflow deleted, the
  // LEFT JOIN nulls everything on `w`). Without `w.id IS NOT NULL`, the
  // COALESCE would silently bucket every orphan into 'personal'.
  const where = workspaceId
    ? `WHERE w.id IS NOT NULL AND COALESCE(w.workspace_id, 'personal') = ?`
    : ''
  const sql = `SELECT wr.*, w.name as workflow_name
               FROM workflow_runs wr
               LEFT JOIN workflows w ON w.id = wr.workflow_id
               ${where}
               ORDER BY wr.started_at DESC
               LIMIT ?`
  const params = workspaceId ? [workspaceId, cappedLimit] : [cappedLimit]
  const rows = d.prepare(sql).all(...params) as RunRow[]
  return mapRunRows(
    rows,
    fetchNodesByRunIds(
      d,
      rows.map((r) => r.id)
    )
  )
}

// ─── Session Logs ─────────────────────────────────────────────────

const MAX_SESSION_LOGS_PER_TASK = 10
const MAX_SESSION_OUTPUT_CHARS = 100_000

export function createSessionLog(entry: SessionLog): void {
  const d = getDb()
  d.prepare(
    `INSERT INTO session_logs (task_id, session_id, agent_type, branch, status, started_at, completed_at, exit_code, logs, project_name)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
  ).run(
    entry.taskId,
    entry.sessionId,
    entry.agentType ?? null,
    entry.branch ?? null,
    entry.status,
    entry.startedAt,
    entry.completedAt ?? null,
    entry.exitCode ?? null,
    entry.logs ?? null,
    entry.projectName ?? null
  )

  // Prune old runs — keep only the most recent N per task
  d.prepare(
    `DELETE FROM session_logs WHERE task_id = ? AND id NOT IN (
       SELECT id FROM session_logs WHERE task_id = ? ORDER BY started_at DESC LIMIT ?
     )`
  ).run(entry.taskId, entry.taskId, MAX_SESSION_LOGS_PER_TASK)
}

export function updateSessionLog(
  sessionId: string,
  updates: Partial<Pick<SessionLog, 'status' | 'completedAt' | 'exitCode' | 'logs'>>
): void {
  const d = getDb()
  const sets: string[] = []
  const vals: unknown[] = []

  if (updates.status !== undefined) {
    sets.push('status = ?')
    vals.push(updates.status)
  }
  if (updates.completedAt !== undefined) {
    sets.push('completed_at = ?')
    vals.push(updates.completedAt)
  }
  if (updates.exitCode !== undefined) {
    sets.push('exit_code = ?')
    vals.push(updates.exitCode)
  }
  if (updates.logs !== undefined) {
    sets.push('logs = ?')
    vals.push(updates.logs)
  }

  if (sets.length === 0) return
  vals.push(sessionId)
  d.prepare(`UPDATE session_logs SET ${sets.join(', ')} WHERE session_id = ?`).run(...vals)
}

export function appendSessionOutput(sessionId: string, data: string): void {
  const d = getDb()
  d.prepare(
    `UPDATE session_logs SET logs = CASE
       WHEN length(COALESCE(logs, '') || ?) > ?
       THEN substr(COALESCE(logs, '') || ?, -?)
       ELSE COALESCE(logs, '') || ?
     END
     WHERE session_id = ?`
  ).run(
    data,
    MAX_SESSION_OUTPUT_CHARS,
    data,
    Math.floor(MAX_SESSION_OUTPUT_CHARS * 0.8),
    data,
    sessionId
  )
}

export function listSessionLogs(taskId: string): SessionLog[] {
  const d = getDb()
  const rows = d
    .prepare('SELECT * FROM session_logs WHERE task_id = ? ORDER BY started_at DESC')
    .all(taskId) as Array<Record<string, unknown>>

  return rows.map((r) => ({
    id: r.id as number,
    taskId: r.task_id as string,
    sessionId: r.session_id as string,
    agentType: (r.agent_type as string | null) ?? undefined,
    branch: (r.branch as string | null) ?? undefined,
    status: r.status as SessionLog['status'],
    startedAt: r.started_at as string,
    completedAt: (r.completed_at as string | null) ?? undefined,
    exitCode: (r.exit_code as number | null) ?? undefined,
    logs: (r.logs as string | null) ?? undefined,
    projectName: (r.project_name as string | null) ?? undefined
  }))
}

// ─── Session Events ───────────────────────────────────────────────

const MAX_SESSION_EVENTS_PER_SESSION = 200

export function insertSessionEvent(event: SessionEvent): void {
  const d = getDb()
  d.prepare(
    `INSERT INTO session_events (session_id, event_type, timestamp, metadata)
     VALUES (?, ?, ?, ?)`
  ).run(
    event.sessionId,
    event.eventType,
    event.timestamp,
    event.metadata ? JSON.stringify(event.metadata) : null
  )

  // Prune old events — keep only the most recent N per session
  d.prepare(
    `DELETE FROM session_events WHERE session_id = ? AND id NOT IN (
       SELECT id FROM session_events WHERE session_id = ? ORDER BY timestamp DESC LIMIT ?
     )`
  ).run(event.sessionId, event.sessionId, MAX_SESSION_EVENTS_PER_SESSION)
}

export function listSessionEvents(eventType?: SessionEventType, limit = 100): SessionEvent[] {
  const d = getDb()
  let rows: Array<Record<string, unknown>>
  if (eventType) {
    rows = d
      .prepare('SELECT * FROM session_events WHERE event_type = ? ORDER BY timestamp DESC LIMIT ?')
      .all(eventType, limit) as Array<Record<string, unknown>>
  } else {
    rows = d
      .prepare('SELECT * FROM session_events ORDER BY timestamp DESC LIMIT ?')
      .all(limit) as Array<Record<string, unknown>>
  }
  return rows.map(mapSessionEventRow)
}

export function listSessionEventsBySession(sessionId: string, limit = 100): SessionEvent[] {
  const d = getDb()
  const rows = d
    .prepare('SELECT * FROM session_events WHERE session_id = ? ORDER BY timestamp DESC LIMIT ?')
    .all(sessionId, limit) as Array<Record<string, unknown>>
  return rows.map(mapSessionEventRow)
}

function mapSessionEventRow(r: Record<string, unknown>): SessionEvent {
  const meta = r.metadata as string | null
  return {
    id: r.id as number,
    sessionId: r.session_id as string,
    eventType: r.event_type as SessionEvent['eventType'],
    timestamp: r.timestamp as string,
    ...(meta != null && { metadata: JSON.parse(meta) })
  }
}
