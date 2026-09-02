import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { connectionConnectorId, type SourceConnection } from '@vornrun/shared/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  initDatabase,
  closeDatabase,
  dbInsertTask,
  dbFindTaskByConnectorExternalId
} from '../packages/server/src/database'

let dataDir: string

/** A packaged connector: stored as `mcp`, really `packdemo`. */
const packaged = {
  id: 'conn-1',
  connectorId: 'mcp',
  name: 'Pack Demo',
  filters: { sdkConnectorId: 'packdemo' },
  syncIntervalMinutes: 5,
  statusMapping: {},
  createdAt: '2026-09-01T00:00:00Z'
} as unknown as SourceConnection

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-adoption-'))
  initDatabase(dataDir)
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('finding the task a packaged connector already made', () => {
  /** Written the way an item from this connection is written today. */
  function writeTask(): void {
    dbInsertTask({
      id: 'task-1',
      title: 'Tick 7',
      description: '',
      status: 'todo',
      order: 1,
      createdAt: '2026-09-01T00:00:00Z',
      updatedAt: '2026-09-01T00:00:00Z',
      sourceConnectorId: connectionConnectorId(packaged),
      sourceExternalId: '7'
    } as Parameters<typeof dbInsertTask>[0])
  }

  // The bug: the lookup that re-adopts an orphan asked under `mcp` while the
  // writer recorded `packdemo`, so a deleted-and-re-added connection found
  // nothing and made the task a second time.
  it('finds it under the connector id the task was written with', () => {
    writeTask()

    expect(dbFindTaskByConnectorExternalId(connectionConnectorId(packaged), '7')?.id).toBe('task-1')
  })

  it('does not find it under the storage type the connection is kept as', () => {
    writeTask()

    expect(dbFindTaskByConnectorExternalId(packaged.connectorId, '7')).toBeNull()
  })

  it('names the connector rather than the storage type for a packaged connection', () => {
    expect(connectionConnectorId(packaged)).toBe('packdemo')
  })

  it('leaves a built-in connection naming itself', () => {
    const builtIn = { ...packaged, connectorId: 'github', filters: {} } as SourceConnection
    expect(connectionConnectorId(builtIn)).toBe('github')
  })
})
