import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { RUNTIME_PROTOCOL_VERSION, type ServerIdentity } from '@vornrun/shared/protocol'
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

const self = {
  dataDir: '/Users/x/.vorn',
  buildChannel: 'packaged' as const,
  expectedPid: 4242
}

function identityOf(over: Partial<ServerIdentity> = {}): ServerIdentity {
  return {
    dataDir: '/Users/x/.vorn',
    buildChannel: 'packaged',
    pid: 4242,
    appVersion: '0.7.0-beta.3',
    ...over
  }
}

const judge = (
  identity: ServerIdentity | null,
  version: number | undefined = RUNTIME_PROTOCOL_VERSION
): ReturnType<typeof judgeAdoption> => judgeAdoption(identity, version, self)

describe('judging a running server', () => {
  it('adopts one that matches', () => {
    expect(judge(identityOf())).toEqual({ kind: 'adopt' })
  })

  it('adopts across app versions', () => {
    // The point of gating on the protocol rather than the release. An update
    // that changed no messages must not end sessions that are still running,
    // and Vorn updates far more often than its protocol changes.
    expect(judge(identityOf({ appVersion: '0.6.1' }))).toEqual({ kind: 'adopt' })
  })

  it('refuses a different protocol version', () => {
    const verdict = judge(identityOf(), RUNTIME_PROTOCOL_VERSION + 1)
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'protocol-mismatch' })
  })

  it('refuses another data directory', () => {
    const verdict = judge(identityOf({ dataDir: '/tmp/other' }))
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'different-data-dir' })
  })

  it('compares data directories after resolving them', () => {
    expect(judge(identityOf({ dataDir: '/Users/x/.vorn/' }))).toEqual({ kind: 'adopt' })
  })

  it('refuses the other build channel', () => {
    // Dev and packaged deliberately share ~/.vorn while keeping separate Electron
    // user data, so without this a `yarn dev` launch adopts the packaged app's
    // bundled server, or the reverse.
    const verdict = judge(identityOf({ buildChannel: 'dev' }))
    expect(verdict).toMatchObject({ kind: 'refuse', reason: 'different-build' })
  })

  it('refuses a server whose pid disagrees with the port file', () => {
    // Both name the same server when everything is honest. Only the port file
    // was written by a process this app can attribute, and this pid is later
    // handed to process.kill.
    expect(judge(identityOf({ pid: 31337 }))).toMatchObject({
      kind: 'refuse',
      reason: 'pid-mismatch'
    })
  })

  it('refuses when no identity arrived at all', () => {
    // Covers both a server too old to send the frame and one that never answered.
    // A timeout means "cannot tell", which must never be read as "nothing is
    // there" — that reading is how a second server gets started on a live port.
    expect(judge(null)).toMatchObject({ kind: 'refuse', reason: 'no-identity' })
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

describe('who the greeting tells this server is', () => {
  it('withholds identity from a peer that is not on loopback', async () => {
    // The greeting is sent before any credential check, deliberately. `dataDir`
    // names the user's home directory, so it carries the account name, and with
    // remote access on the server binds 0.0.0.0 -- where the Origin allowlist
    // does not apply to a peer that simply sends no Origin. Only a desktop on
    // this machine has any use for these fields.
    const { isLoopbackAddress } = await import('../packages/server/src/ws-handler')

    expect(isLoopbackAddress('127.0.0.1')).toBe(true)
    expect(isLoopbackAddress('::1')).toBe(true)
    // What a dual-stack listener actually reports for a v4 loopback connection.
    expect(isLoopbackAddress('::ffff:127.0.0.1')).toBe(true)
    expect(isLoopbackAddress('192.168.1.42')).toBe(false)
    expect(isLoopbackAddress('100.64.0.7')).toBe(false)
    expect(isLoopbackAddress(undefined)).toBe(false)
  })
})

describe('treating the port file and the greeting as untrusted', () => {
  let dir: string
  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-untrusted-'))
  })
  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  const writePortFile = (value: unknown): void =>
    fs.writeFileSync(path.join(dir, 'ws-port'), JSON.stringify(value))

  it('rejects pid 0, which signals this process own group and always succeeds', () => {
    // The nastiest of the three: process.kill(0, 0) succeeds, so a pid of 0 reads
    // as permanently alive. Since a live incumbent is refused rather than
    // replaced, one bad byte in this file would stop the app starting at all.
    expect(isPidAlive(0)).toBe(false)
    writePortFile({ port: 50091, pid: 0 })
    expect(readPortFile(dir)).toBeNull()
  })

  it('rejects a negative pid, which targets a process group', () => {
    expect(isPidAlive(-1)).toBe(false)
    writePortFile({ port: 50091, pid: -1 })
    expect(readPortFile(dir)).toBeNull()
  })

  it('rejects a non-integer pid', () => {
    expect(isPidAlive(1.5)).toBe(false)
    expect(isPidAlive(NaN)).toBe(false)
  })

  it.each([0, -1, 65536, 1.5, NaN])('rejects an out-of-range port: %s', (port) => {
    writePortFile({ port, pid: process.pid })
    expect(readPortFile(dir)).toBeNull()
  })

  it('refuses a greeting missing its data directory rather than throwing', () => {
    // path.resolve(undefined) throws, and a throw that is not an
    // AdoptionRefusedError quits the app -- so the launcher would die exactly
    // where it meant to decline.
    const malformed = { appVersion: '1', buildChannel: 'packaged', pid: 4242 }
    expect(() => judge(malformed as unknown as ServerIdentity)).not.toThrow()
    expect(judge(malformed as unknown as ServerIdentity)).toMatchObject({
      kind: 'refuse',
      reason: 'no-identity'
    })
  })

  it.each([
    [
      'a bogus build channel',
      { dataDir: '/Users/x/.vorn', appVersion: '1', buildChannel: 'x', pid: 1 }
    ],
    ['an empty data directory', { dataDir: '', appVersion: '1', buildChannel: 'packaged', pid: 1 }],
    [
      'a pid of zero',
      { dataDir: '/Users/x/.vorn', appVersion: '1', buildChannel: 'packaged', pid: 0 }
    ],
    ['a non-object', 'not an identity']
  ])('refuses %s', (_label, frame) => {
    expect(judge(frame as unknown as ServerIdentity)).toMatchObject({ reason: 'no-identity' })
  })
})
