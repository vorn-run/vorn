import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  initDatabase,
  closeDatabase,
  claimEffectReceipt,
  pruneEffectReceipts
} from '../packages/server/src/database'
import { queryDb } from './helpers/database'

let dataDir: string
let dbFile: string

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

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-migration-26-'))
  dbFile = path.join(dataDir, 'vorn.db')
  initDatabase(dataDir)
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('migration 26 — effect receipts', () => {
  it('creates the table on a database that stopped at 25', () => {
    closeDatabase()
    queryDb(dbFile, (d) => {
      d.exec('DROP TABLE effect_receipts')
      d.prepare("UPDATE schema_meta SET value = '25' WHERE key = 'schema_version'").run()
    })
    initDatabase(dataDir)
    expect(version()).toBe('26')
    expect(claimEffectReceipt('s:1:2:0', 'trigger', 10)).toBe(true)
  })

  it('claims an effect id once, whatever the kind of the repeat', () => {
    expect(claimEffectReceipt('s:1:2:0', 'notify', 10)).toBe(true)
    expect(claimEffectReceipt('s:1:2:0', 'notify', 11)).toBe(false)
    expect(claimEffectReceipt('s:1:2:1', 'notify', 11)).toBe(true)
  })

  it('outlives a restart of the server', () => {
    expect(claimEffectReceipt('s:1:9:0', 'trigger', 10)).toBe(true)
    closeDatabase()
    initDatabase(dataDir)
    expect(claimEffectReceipt('s:1:9:0', 'trigger', 20)).toBe(false)
  })

  it('prunes by kind and age only', () => {
    claimEffectReceipt('old-notify', 'notify', 10)
    claimEffectReceipt('new-notify', 'notify', 100)
    claimEffectReceipt('old-trigger', 'trigger', 10)
    expect(pruneEffectReceipts('notify', 50)).toBe(1)
    expect(claimEffectReceipt('old-notify', 'notify', 200)).toBe(true)
    expect(claimEffectReceipt('new-notify', 'notify', 200)).toBe(false)
    expect(claimEffectReceipt('old-trigger', 'trigger', 200)).toBe(false)
  })
})
