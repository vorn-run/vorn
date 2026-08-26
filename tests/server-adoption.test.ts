import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { RUNTIME_PROTOCOL_VERSION, type ServerHello } from '@vornrun/shared/protocol'
import {
  isPidAlive,
  judgeAdoption,
  readLocalToken,
  readPortFile
} from '../src/main/server/server-adoption'

/**
 * Deciding whether a server that is already running belongs to this app.
 *
 * The cost of the two wrong answers is not symmetric, and that asymmetry is what
 * these tests pin down. Refusing a server we could have used wastes a process.
 * Adopting one we should not have — a different data directory, a different
 * build — puts two servers on one database. And killing one we could not talk to
 * would end the sessions it was holding, which is the user's work.
 */

const self = { dataDir: '/Users/x/.vorn', buildChannel: 'packaged' as const }

function hello(over: Partial<ServerHello> = {}): ServerHello {
  return {
    protocolVersion: RUNTIME_PROTOCOL_VERSION,
    capabilities: { auth: 1 },
    dataDir: '/Users/x/.vorn',
    buildChannel: 'packaged',
    pid: 4242,
    appVersion: '0.7.0-beta.3',
    ...over
  }
}

describe('judging a running server', () => {
  it('adopts one that matches', () => {
    expect(judgeAdoption(hello(), self)).toEqual({ kind: 'adopt' })
  })

  it('adopts across app versions', () => {
    // The point of gating on the protocol rather than the release. An update
    // that changed no messages must not end sessions that are still running,
    // and Vorn updates far more often than its protocol changes.
    expect(judgeAdoption(hello({ appVersion: '0.6.1' }), self)).toEqual({ kind: 'adopt' })
  })

  it('refuses a different protocol version', () => {
    const verdict = judgeAdoption(hello({ protocolVersion: RUNTIME_PROTOCOL_VERSION + 1 }), self)
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'protocol-mismatch' })
  })

  it('refuses another data directory', () => {
    const verdict = judgeAdoption(hello({ dataDir: '/tmp/other' }), self)
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'different-data-dir' })
  })

  it('compares data directories after resolving them', () => {
    expect(judgeAdoption(hello({ dataDir: '/Users/x/.vorn/' }), self)).toEqual({ kind: 'adopt' })
  })

  it('refuses the other build channel', () => {
    // Dev and packaged deliberately share ~/.vorn while keeping separate Electron
    // user data, so without this a `yarn dev` launch adopts the packaged app's
    // bundled server, or the reverse.
    const verdict = judgeAdoption(hello({ buildChannel: 'dev' }), self)
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'different-build' })
  })

  it('refuses a server that does not report its pid', () => {
    // Without one there is no way to tell a dead server from a reconnecting
    // bridge, and no handle left for stopping one this app did not spawn.
    const { pid: _p, ...noPid } = hello()
    expect(judgeAdoption(noPid as ServerHello, self)).toMatchObject({
      kind: 'refuse',
      reason: 'no-identity'
    })
  })

  it('refuses a server that does not say who it is', () => {
    // A build old enough to predate these fields cannot be told apart from one on
    // another data directory. Declining costs a spawn; guessing costs a database.
    const { dataDir: _d, buildChannel: _b, ...anonymous } = hello()
    expect(judgeAdoption(anonymous as ServerHello, self)).toMatchObject({
      kind: 'refuse',
      reason: 'no-identity'
    })
  })

  it('refuses when no greeting arrived at all', () => {
    // A timeout means "cannot tell", which must never be read as "nothing is
    // there" — that reading is how a second server gets started on a live port.
    expect(judgeAdoption(null, self)).toMatchObject({ kind: 'refuse', reason: 'no-identity' })
  })
})

describe('reading what a running server published', () => {
  let dir: string

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-adopt-'))
  })

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('reads a port file whose process is alive', () => {
    fs.writeFileSync(path.join(dir, 'ws-port'), JSON.stringify({ port: 50091, pid: process.pid }))
    expect(readPortFile(dir)).toEqual({ port: 50091, pid: process.pid })
  })

  it('treats a port file whose process is dead as absent', () => {
    // The server deletes this on an orderly exit only, so a stale file is the
    // normal state after a crash. The port it names may since be anyone's.
    fs.writeFileSync(path.join(dir, 'ws-port'), JSON.stringify({ port: 50091, pid: 0x7ffffffe }))
    expect(readPortFile(dir)).toBeNull()
  })

  it('returns null for a missing or unreadable file', () => {
    expect(readPortFile(dir)).toBeNull()
    fs.writeFileSync(path.join(dir, 'ws-port'), 'not json')
    expect(readPortFile(dir)).toBeNull()
  })

  it('reads the local credential and ignores surrounding whitespace', () => {
    fs.writeFileSync(path.join(dir, 'local-token'), 'secret-value\n')
    expect(readLocalToken(dir)).toBe('secret-value')
  })

  it('returns null rather than an empty credential', () => {
    // An empty string would be presented as a Bearer token and rejected, which
    // looks like a broken server rather than a missing file.
    fs.writeFileSync(path.join(dir, 'local-token'), '   ')
    expect(readLocalToken(dir)).toBeNull()
  })
})

describe('probing whether a process is alive', () => {
  it('says yes for this process', () => {
    expect(isPidAlive(process.pid)).toBe(true)
  })

  it('says no for a pid that is not running', () => {
    expect(isPidAlive(0x7ffffffe)).toBe(false)
  })

  it('treats a permission error as alive', () => {
    // EPERM means the process exists and belongs to someone else. Reading it as
    // dead would let this app start a second server against a live one.
    const spy = vi.spyOn(process, 'kill').mockImplementation(() => {
      const err = new Error('operation not permitted') as NodeJS.ErrnoException
      err.code = 'EPERM'
      throw err
    })
    expect(isPidAlive(1)).toBe(true)
    spy.mockRestore()
  })
})
