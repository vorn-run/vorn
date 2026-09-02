import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Database from 'libsql'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'

let dataDir: string
let dbFile: string

function query<T>(fn: (d: Database.Database) => T): T {
  const d = new Database(dbFile)
  try {
    return fn(d)
  } finally {
    d.close()
  }
}

const MIGRATION = 17

/**
 * A database as it looked before the migration: a packaged connection, a task
 * from it recorded under `mcp`, and the link that ties the two together.
 */
function seedPackagedTask(options: { withConnection?: boolean } = {}): void {
  const { withConnection = true } = options
  query((d) => {
    if (withConnection) {
      d.prepare(
        `INSERT INTO source_connections
           (id, connector_id, name, filters, sync_interval_minutes, status_mapping, created_at)
         VALUES ('conn-1', 'mcp', 'Pack Demo', ?, 5, '{}', '2026-09-01T00:00:00Z')`
      ).run(JSON.stringify({ sdkConnectorId: 'packdemo' }))
    }
    d.prepare(
      `INSERT INTO tasks (id, title, status, "order", created_at, updated_at, source_connector_id, source_external_id)
       VALUES ('task-1', 'Tick 7', 'todo', 1, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 'mcp', '7')`
    ).run()
    if (withConnection) {
      d.prepare(
        `INSERT INTO task_source_links
           (task_id, connection_id, connector_id, external_id, external_url,
            source_status_raw, source_updated_at, last_synced_at)
         VALUES ('task-1', 'conn-1', 'mcp', '7', '', 'open', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')`
      ).run()
    }
    d.prepare(
      `INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '16')`
    ).run()
  })
}

const taskConnectorId = (): string | null =>
  query(
    (d) =>
      (
        d.prepare("SELECT source_connector_id AS id FROM tasks WHERE id = 'task-1'").get() as {
          id: string | null
        }
      ).id
  )

const linkConnectorId = (): string | null =>
  query((d) => {
    const row = d
      .prepare("SELECT connector_id AS id FROM task_source_links WHERE task_id = 'task-1'")
      .get() as { id: string } | undefined
    return row?.id ?? null
  })

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-migration-17-'))
  dbFile = path.join(dataDir, 'vorn.db')
  initDatabase(dataDir)
  closeDatabase()
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe(`migration ${MIGRATION} — what a packaged connector's tasks came from`, () => {
  it('rewrites mcp to the connector the task really came from', () => {
    seedPackagedTask()

    initDatabase(dataDir)
    closeDatabase()

    expect(taskConnectorId()).toBe('packdemo')
    expect(linkConnectorId()).toBe('packdemo')
  })

  it('leaves a task alone when nothing says which connector it was', () => {
    // No connection, so no `sdkConnectorId` to derive: a guess would be worse
    // than the honest `mcp` already recorded.
    seedPackagedTask({ withConnection: false })

    initDatabase(dataDir)
    closeDatabase()

    expect(taskConnectorId()).toBe('mcp')
  })

  it('leaves a built-in connector task exactly as it was', () => {
    query((d) => {
      d.prepare(
        `INSERT INTO tasks (id, title, status, "order", created_at, updated_at, source_connector_id, source_external_id)
         VALUES ('task-2', 'Issue 3', 'todo', 2, '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z', 'github', '3')`
      ).run()
      d.prepare(
        `INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '16')`
      ).run()
    })

    initDatabase(dataDir)
    closeDatabase()

    const id = query(
      (d) =>
        (
          d.prepare("SELECT source_connector_id AS id FROM tasks WHERE id = 'task-2'").get() as {
            id: string
          }
        ).id
    )
    expect(id).toBe('github')
  })
})
