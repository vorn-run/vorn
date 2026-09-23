import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { queryDb } from './helpers/database'

let dataDir: string
let dbFile: string

const tables = (): string[] =>
  queryDb(dbFile, (d) =>
    (
      d.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all() as Array<{
        name: string
      }>
    ).map((r) => r.name)
  )

const version = (): string =>
  queryDb(
    dbFile,
    (d) =>
      (
        d.prepare("SELECT value FROM schema_meta WHERE key = 'schema_version'").get() as {
          value: string
        }
      ).value
  )

const nodeColumns = (): string[] =>
  queryDb(dbFile, (d) =>
    (d.prepare('PRAGMA table_info(workflow_run_nodes)').all() as Array<{ name: string }>).map(
      (c) => c.name
    )
  )

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-migration-25-'))
  dbFile = path.join(dataDir, 'vorn.db')
  initDatabase(dataDir)
  closeDatabase()
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('migration 25 — published artifacts', () => {
  it('creates the artifact tables on a database that stopped at 24', () => {
    queryDb(dbFile, (d) => {
      d.exec('DROP TABLE artifact_comments; DROP TABLE artifact_versions; DROP TABLE artifacts;')
      d.prepare("UPDATE schema_meta SET value = '24' WHERE key = 'schema_version'").run()
    })
    initDatabase(dataDir)
    closeDatabase()
    expect(tables()).toEqual(
      expect.arrayContaining(['artifacts', 'artifact_versions', 'artifact_comments'])
    )
    expect(version()).toBe('25')
  })

  it('repairs the gate edit columns when a version was stamped without them', () => {
    queryDb(dbFile, (d) => {
      d.exec('ALTER TABLE workflow_run_nodes DROP COLUMN edited_text')
    })
    initDatabase(dataDir)
    closeDatabase()
    expect(nodeColumns()).toEqual(expect.arrayContaining(['editable_text', 'edited_text']))
  })
})
