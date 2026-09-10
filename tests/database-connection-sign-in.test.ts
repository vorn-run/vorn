import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('node:fs', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, existsSync: vi.fn(() => true), mkdirSync: vi.fn() }
})

import {
  initTestDatabase,
  dbInsertSourceConnection,
  dbGetSourceConnection,
  dbSetConnectionSignIn,
  dbUpdateSourceConnection
} from '../packages/server/src/database'

let teardown: () => void

beforeEach(() => {
  teardown = initTestDatabase()
  dbInsertSourceConnection({
    id: 'substack',
    connectorId: 'mcp',
    name: 'Substack',
    filters: { sdkConnectorId: 'substack' },
    syncIntervalMinutes: 5,
    statusMapping: {},
    createdAt: '2026-09-10T20:00:00.000Z'
  })
})

afterEach(() => {
  teardown()
})

describe('who a connection is signed in as', () => {
  it('is kept once the window signs in, and cleared when it signs out', () => {
    expect(dbGetSourceConnection('substack')?.signedInAs).toBeUndefined()

    dbSetConnectionSignIn(
      'substack',
      'Javier Canizalez (javiercanizalez)',
      '2026-09-10T20:05:00.000Z'
    )
    expect(dbGetSourceConnection('substack')).toMatchObject({
      signedInAs: 'Javier Canizalez (javiercanizalez)',
      signedInAt: '2026-09-10T20:05:00.000Z'
    })

    dbSetConnectionSignIn('substack', null, null)
    const signedOut = dbGetSourceConnection('substack')
    expect(signedOut?.signedInAs).toBeUndefined()
    expect(signedOut?.signedInAt).toBeUndefined()
  })

  it('survives an edit to the connection, which rewrites its filters whole', () => {
    dbSetConnectionSignIn('substack', 'Javier Canizalez', '2026-09-10T20:05:00.000Z')
    dbUpdateSourceConnection('substack', {
      filters: { sdkConnectorId: 'substack', publication: 'novumai' }
    })
    expect(dbGetSourceConnection('substack')?.signedInAs).toBe('Javier Canizalez')
  })
})
