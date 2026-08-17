import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Database from 'libsql'

// Real filesystem here: this file is about where the database lands and what
// happens when an existing one is reopened, which a mocked fs cannot show.
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  initDatabase,
  closeDatabase,
  getDataDir,
  dbGetOwnerUser,
  dbSignalChange
} from '../packages/server/src/database'
import { mintOwnerToken } from '../packages/server/src/token-manager'

let dataDir: string
let dbFile: string

/** Open the database file directly, outside the module under test. */
function query<T>(fn: (d: Database.Database) => T): T {
  const d = new Database(dbFile)
  try {
    return fn(d)
  } finally {
    d.close()
  }
}

const readSchemaVersion = (): number =>
  query((d) => {
    const row = d.prepare("SELECT value FROM schema_meta WHERE key = 'schema_version'").get() as
      | { value: string }
      | undefined
    return row ? Number.parseInt(row.value, 10) : 0
  })

const countUsers = (): number =>
  query((d) => (d.prepare('SELECT COUNT(*) AS n FROM users').get() as { n: number }).n)

/** Wind a migrated database back to look like one that never saw version 14. */
const rewindToVersion13 = (): void =>
  query((d) => {
    d.exec('DELETE FROM users')
    d.prepare(
      "INSERT OR REPLACE INTO schema_meta (key, value) VALUES ('schema_version', '13')"
    ).run()
  })

beforeEach(() => {
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-migration-'))
  dbFile = path.join(dataDir, 'vorn.db')
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('migration 14 — identity and device tokens', () => {
  it('lands a fresh database on version 14 with one owner', () => {
    initDatabase(dataDir)
    closeDatabase()

    expect(readSchemaVersion()).toBe(14)
    expect(countUsers()).toBe(1)
  })

  it('seeds the owner when migrating a version 13 database', () => {
    initDatabase(dataDir)
    closeDatabase()

    rewindToVersion13()
    expect(countUsers()).toBe(0)
    expect(readSchemaVersion()).toBe(13)

    initDatabase(dataDir)
    const owner = dbGetOwnerUser()
    closeDatabase()

    expect(owner).not.toBeNull()
    expect(owner?.role).toBe('owner')
    expect(readSchemaVersion()).toBe(14)
    expect(countUsers()).toBe(1)
  })

  it('is a no-op when the database is reopened', () => {
    initDatabase(dataDir)
    const first = dbGetOwnerUser()
    closeDatabase()

    // Reopening runs createSchema and every migration again. A second owner here
    // would mean the seed keys off the version alone rather than off the data,
    // and every restart would add another.
    initDatabase(dataDir)
    const second = dbGetOwnerUser()
    closeDatabase()

    expect(second?.id).toBe(first?.id)
    expect(countUsers()).toBe(1)
  })

  it('does not re-seed an owner that was renamed', () => {
    initDatabase(dataDir)
    closeDatabase()

    query((d) => d.prepare('UPDATE users SET name = ?').run('renamed'))

    initDatabase(dataDir)
    const owner = dbGetOwnerUser()
    closeDatabase()

    expect(owner?.name).toBe('renamed')
    expect(countUsers()).toBe(1)
  })
})

describe('corrupt database recovery', () => {
  it('backs up an unreadable file and starts fresh rather than failing to boot', () => {
    // Every path in the recovery routine derives from the resolved data dir, so
    // a --data-dir server must back up and rebuild its own file, never ~/.vorn's.
    fs.writeFileSync(dbFile, 'this is not a sqlite database')

    initDatabase(dataDir)
    const owner = dbGetOwnerUser()
    closeDatabase()

    const backups = fs.readdirSync(dataDir).filter((f) => f.includes('.corrupt-'))
    expect(backups).toHaveLength(1)
    expect(fs.readFileSync(path.join(dataDir, backups[0]), 'utf-8')).toBe(
      'this is not a sqlite database'
    )

    // Rebuilt, migrated, and seeded — not merely opened.
    expect(readSchemaVersion()).toBe(14)
    expect(owner?.role).toBe('owner')
  })
})

describe('minting against the seeded owner', () => {
  it('refuses when no owner exists, rather than minting an orphan token', () => {
    // A database that somehow lost its owner. Minting anyway would write a token
    // whose user_id references nothing, and device_tokens.user_id is a foreign
    // key — so the failure would surface later, at insert, not here.
    initDatabase(dataDir)
    closeDatabase()

    query((d) => {
      d.exec('DELETE FROM device_tokens')
      d.exec('DELETE FROM users')
    })

    initDatabase(dataDir)
    expect(() => mintOwnerToken('iPhone')).toThrow(/No owner user found/)
  })
})

describe('data directory', () => {
  it('puts the database in an explicit data dir', () => {
    initDatabase(dataDir)
    expect(getDataDir()).toBe(dataDir)
    closeDatabase()

    expect(fs.existsSync(dbFile)).toBe(true)
  })

  it('refuses to answer before a directory is resolved', async () => {
    // Returning the default here instead would hand a too-early caller a
    // plausible ~/.vorn and have it read or watch the wrong directory forever —
    // which is exactly how the config watcher came to re-derive its own path.
    //
    // Deliberately does not call initDatabase() with no argument to check the
    // default: that opens the developer's real database and migrates it as a
    // side effect of running the test suite.
    vi.resetModules()
    const fresh = await import('../packages/server/src/database')
    expect(() => fresh.getDataDir()).toThrow(/not resolved/)
  })

  it('writes the change signal beside the database, not into ~/.vorn', () => {
    // The config watcher watches the resolved directory, so the signal has to
    // land there. Before the consolidation these were two separate paths.
    initDatabase(dataDir)
    dbSignalChange()
    closeDatabase()

    expect(fs.existsSync(path.join(dataDir, '.db-signal'))).toBe(true)
  })

  it('creates the directory when it does not exist', () => {
    const nested = path.join(dataDir, 'deeper', 'still')
    initDatabase(nested)
    closeDatabase()

    expect(fs.existsSync(path.join(nested, 'vorn.db'))).toBe(true)
  })
})
