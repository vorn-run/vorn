import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import Database from 'libsql'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  authenticateCredential,
  bearerFrom,
  initBootstrapSecret,
  clearLocalCredential
} from '../packages/server/src/ws-auth'
import { LOCAL_TOKEN_FILENAME } from '@vornrun/shared/protocol'
import { initDatabase, closeDatabase, dbGetOwnerUser } from '../packages/server/src/database'
import { mintOwnerToken, revokeToken, listTokens } from '../packages/server/src/token-manager'

let dataDir: string

/**
 * Only the suites that touch credentials need a database. Applied per-describe
 * rather than to the file, so the pure-function tests do not each pay for a temp
 * directory and fourteen migrations.
 */
function withDatabaseFixture(): void {
  beforeEach(() => {
    dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-ws-auth-'))
    initDatabase(dataDir)
  })

  afterEach(() => {
    clearLocalCredential()
    closeDatabase()
    fs.rmSync(dataDir, { recursive: true, force: true })
  })
}

describe('bearerFrom', () => {
  it.each([
    ['empty', ''],
    ['wrong scheme', 'Basic abc'],
    ['no value', 'Bearer '],
    ['lowercase scheme', 'bearer abc']
  ])('returns undefined for %s', (_label, header) => {
    expect(bearerFrom(header as string | undefined)).toBeUndefined()
  })

  it('extracts the token', () => {
    expect(bearerFrom('Bearer vorn_abc_def')).toBe('vorn_abc_def')
  })
})

describe('the local credential', () => {
  withDatabaseFixture()

  it('publishes a file only readable by this user', () => {
    initBootstrapSecret(dataDir)
    const file = path.join(dataDir, LOCAL_TOKEN_FILENAME)

    expect(fs.existsSync(file)).toBe(true)
    // Same trust boundary hook-server already accepts. A hostile web page can
    // open a socket to loopback but cannot read a file off disk, which is why
    // this does not weaken the control that matters.
    expect(fs.statSync(file).mode & 0o777).toBe(0o600)
  })

  it('authenticates the secret it published', () => {
    initBootstrapSecret(dataDir)
    const published = fs.readFileSync(path.join(dataDir, LOCAL_TOKEN_FILENAME), 'utf-8')

    const result = authenticateCredential(published)
    expect(result?.userId).toBe(dbGetOwnerUser()?.id)
    // No database row backs it, so there is nothing to revoke or to leak — and
    // `kind` is what lets the bridge claim tell the desktop from a remote client.
    expect(result?.kind).toBe('bootstrap')
    expect(listTokens()).toEqual([])
  })

  it('uses the secret the desktop supplied rather than inventing one', () => {
    initBootstrapSecret(dataDir, 'supplied-by-the-desktop')
    expect(authenticateCredential('supplied-by-the-desktop')).not.toBeNull()
  })

  it('generates one when nothing was supplied, so a standalone server still works', () => {
    initBootstrapSecret(dataDir, undefined)
    const published = fs.readFileSync(path.join(dataDir, LOCAL_TOKEN_FILENAME), 'utf-8')
    expect(published.length).toBeGreaterThan(20)
    expect(authenticateCredential(published)).not.toBeNull()
  })

  it('removes the file on shutdown, so nothing usable outlives the process', () => {
    initBootstrapSecret(dataDir)
    clearLocalCredential()
    expect(fs.existsSync(path.join(dataDir, LOCAL_TOKEN_FILENAME))).toBe(false)
  })

  it('degrades to no published credential when the directory is unwritable', () => {
    // The server must still start and still serve the desktop, which authenticates
    // from the environment. Only the same-machine tools that read the file lose out,
    // which is why this warns rather than throws.
    const unwritable = path.join(dataDir, 'does', 'not', 'exist')
    expect(() => initBootstrapSecret(unwritable)).not.toThrow()
  })

  it('publishes nothing when another server owns the directory', () => {
    // The bug this closes, reproduced: a `yarn dev` server starting beside a
    // packaged one wrote its own secret over the credential the running app had
    // published, so MCP read one server's port beside another server's token and
    // every call timed out until Vorn was restarted.
    const file = path.join(dataDir, 'local-token')
    fs.writeFileSync(file, 'the-running-servers-secret', { mode: 0o600 })

    initBootstrapSecret(dataDir, 'this-servers-secret', false)

    expect(fs.readFileSync(file, 'utf-8')).toBe('the-running-servers-secret')
  })

  it('still authenticates its own secret when it publishes nothing', () => {
    // Standing aside is about the file, not about the server. The desktop that
    // started this process handed the secret over in the environment and must
    // still be able to use it.
    initBootstrapSecret(dataDir, 'this-servers-secret', false)

    expect(authenticateCredential('this-servers-secret')).toMatchObject({ kind: 'bootstrap' })
  })

  it('removes nothing on shutdown when it published nothing', () => {
    const file = path.join(dataDir, 'local-token')
    fs.writeFileSync(file, 'the-running-servers-secret', { mode: 0o600 })

    initBootstrapSecret(dataDir, 'this-servers-secret', false)
    clearLocalCredential()

    expect(fs.readFileSync(file, 'utf-8')).toBe('the-running-servers-secret')
  })

  it('leaves a credential that has since been replaced', () => {
    // Publishing and shutting down are minutes apart. A file that is no longer
    // ours belongs to whoever wrote it, and removing it would leave that server
    // unreachable -- the same failure, caused on the way out instead of in.
    const file = path.join(dataDir, 'local-token')
    initBootstrapSecret(dataDir, 'ours')
    fs.writeFileSync(file, 'somebody-elses', { mode: 0o600 })

    clearLocalCredential()

    expect(fs.readFileSync(file, 'utf-8')).toBe('somebody-elses')
  })

  it('replaces an existing file rather than inheriting its permissions', () => {
    const file = path.join(dataDir, LOCAL_TOKEN_FILENAME)
    fs.writeFileSync(file, 'stale', { mode: 0o644 })

    initBootstrapSecret(dataDir)

    // `mode` on writeFileSync only applies when the file is created — the flaw in
    // the hook-server precedent this follows.
    expect(fs.statSync(file).mode & 0o777).toBe(0o600)
    expect(fs.readFileSync(file, 'utf-8')).not.toBe('stale')
  })
})

describe('authenticateCredential', () => {
  withDatabaseFixture()

  it.each([
    ['undefined', undefined],
    ['empty', ''],
    ['a wrong secret', 'nonsense'],
    ['a malformed token', 'vorn_only-one-part']
  ])('refuses %s', (_label, raw) => {
    initBootstrapSecret(dataDir)
    expect(authenticateCredential(raw as string | undefined)).toBeNull()
  })

  it('accepts a device token and records that it was seen', () => {
    const { token, plaintext } = mintOwnerToken('iPhone')
    expect(listTokens()[0].lastSeenAt).toBeNull()

    const result = authenticateCredential(plaintext)
    expect(result).toEqual({ userId: token.userId, kind: 'device', tokenId: token.id })
    expect(listTokens()[0].lastSeenAt).toBeTruthy()
  })

  it('refuses a revoked device token', () => {
    const { token, plaintext } = mintOwnerToken('iPhone')
    revokeToken(token.id)
    expect(authenticateCredential(plaintext)).toBeNull()
  })

  it('refuses the local secret when the database has no owner to attribute it to', () => {
    // The credential is valid but there is nobody to be. Admitting the socket with
    // no userId would leave per-message authorization with nothing to read.
    initBootstrapSecret(dataDir, 'supplied-by-the-desktop')

    // Wipe the seeded owner through a second connection to the same file. The
    // reopen does not re-seed: schema_version is already 14, so migration 14 does
    // not run again.
    closeDatabase()
    const d = new Database(path.join(dataDir, 'vorn.db'))
    d.exec('DELETE FROM device_tokens')
    d.exec('DELETE FROM users')
    d.close()
    initDatabase(dataDir)

    expect(authenticateCredential('supplied-by-the-desktop')).toBeNull()
  })

  it('does not accept a near-miss of the local secret', () => {
    initBootstrapSecret(dataDir, 'exactly-this-secret')
    expect(authenticateCredential('exactly-this-secre')).toBeNull()
    expect(authenticateCredential('exactly-this-secret!')).toBeNull()
    expect(authenticateCredential('Exactly-this-secret')).toBeNull()
  })
})
