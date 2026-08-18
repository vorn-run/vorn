import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

// `serve` is the one command that needs a listening server. Mocking the entry
// point keeps the test to the CLI's own behaviour — dispatch, output, and the
// first-run mint — without binding a port.
const startServer = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/index', () => ({ startServer }))

import { runCli, type CliDeps } from '../packages/server/src/cli'
import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { mintOwnerToken } from '../packages/server/src/token-manager'

let dataDir: string

/** Collects what the CLI wrote, so assertions read against real output. */
function capture(): CliDeps & { out: () => string; err: () => string } {
  const outParts: string[] = []
  const errParts: string[] = []
  return {
    write: (t) => outParts.push(t),
    writeErr: (t) => errParts.push(t),
    out: () => outParts.join(''),
    err: () => errParts.join('')
  }
}

/** `token create` etc. open and close the database themselves. */
function run(argv: string[]) {
  const io = capture()
  return runCli([...argv, '--data-dir', dataDir], io).then((code) => ({ code, io }))
}

/**
 * The real `startServer` initializes the database on its way up, and `runServe`
 * reads tokens afterwards. The stand-in has to do the same or it would hide that
 * ordering rather than exercise it.
 */
function mockServe(port = 4400): void {
  startServer.mockImplementation(async (opts?: { dataDir?: string }) => {
    initDatabase(opts?.dataDir)
    return { port }
  })
}

beforeEach(() => {
  startServer.mockReset()
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-cli-'))
  mockServe()
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('usage and dispatch', () => {
  it('prints usage to stdout and succeeds for --help', async () => {
    const io = capture()
    expect(await runCli(['--help'], io)).toBe(0)
    expect(io.out()).toContain('vorn-server: run a Vorn server')
    expect(io.err()).toBe('')
  })

  it('accepts the help subcommand too', async () => {
    const io = capture()
    expect(await runCli(['help'], io)).toBe(0)
    expect(io.out()).toContain('Usage')
  })

  it('treats a bare invocation as a usage error, on stderr', async () => {
    const io = capture()
    expect(await runCli([], io)).toBe(2)
    expect(io.err()).toContain('Usage')
    expect(io.out()).toBe('')
  })

  it('reports an unknown command', async () => {
    const io = capture()
    expect(await runCli(['bogus'], io)).toBe(2)
    expect(io.err()).toContain('unknown command "bogus"')
  })

  it('reports a malformed argument rather than passing it on', async () => {
    const io = capture()
    expect(await runCli(['serve', '--port=abc'], io)).toBe(2)
    expect(io.err()).toContain('must be a number')
    expect(startServer).not.toHaveBeenCalled()
  })

  it('reports an unknown option', async () => {
    const io = capture()
    expect(await runCli(['serve', '--nope'], io)).toBe(2)
    expect(io.err()).toContain('vorn-server:')
  })
})

describe('token create', () => {
  it('mints a token and shows the plaintext once', async () => {
    const { code, io } = await run(['token', 'create', '--name', 'iPhone'])

    expect(code).toBe(0)
    expect(io.out()).toContain('Created token "iPhone"')
    expect(io.out()).toContain('vorn_')
    expect(io.out()).toContain('This is the only time it is shown')
  })

  it('requires a name', async () => {
    const { code, io } = await run(['token', 'create'])
    expect(code).toBe(2)
    expect(io.err()).toContain('requires --name')
  })
})

describe('token list', () => {
  it('says so when there are none', async () => {
    const { code, io } = await run(['token', 'list'])
    expect(code).toBe(0)
    expect(io.out()).toBe('No device tokens.\n')
  })

  it('lists a token as active, then as revoked', async () => {
    await run(['token', 'create', '--name', 'iPhone'])

    const listed = await run(['token', 'list'])
    expect(listed.io.out()).toContain('active')
    expect(listed.io.out()).toContain('iPhone')
    expect(listed.io.out()).toContain('last seen never')

    const id = listed.io.out().split(' ')[0]
    await run(['token', 'revoke', id])

    const after = await run(['token', 'list'])
    expect(after.io.out()).toContain('revoked')
  })
})

describe('token revoke', () => {
  it('revokes an existing token', async () => {
    initDatabase(dataDir)
    const { token } = mintOwnerToken('iPhone')
    closeDatabase()

    const { code, io } = await run(['token', 'revoke', token.id])
    expect(code).toBe(0)
    expect(io.out()).toBe(`Revoked ${token.id}\n`)
  })

  it('requires an id', async () => {
    const { code, io } = await run(['token', 'revoke'])
    expect(code).toBe(2)
    expect(io.err()).toContain('requires a token id')
  })

  it('fails when the token is unknown or already revoked', async () => {
    const { code, io } = await run(['token', 'revoke', 'not-a-token'])
    expect(code).toBe(1)
    expect(io.err()).toContain('no active token with id not-a-token')
  })

  it('reports an unknown token subcommand', async () => {
    const { code, io } = await run(['token', 'wat'])
    expect(code).toBe(2)
    expect(io.err()).toContain('unknown token command "wat"')
  })

  it('reports a missing token subcommand', async () => {
    const { code, io } = await run(['token'])
    expect(code).toBe(2)
    expect(io.err()).toContain('unknown token command')
  })
})

describe('serve', () => {
  it('reports the port and mints a first-run token on an empty data dir', async () => {
    const io = capture()
    const code = await runCli(['serve', '--data-dir', dataDir], io)

    expect(code).toBe(0)
    expect(startServer).toHaveBeenCalledWith({
      host: undefined,
      port: undefined,
      dataDir
    })
    expect(io.out()).toContain('Vorn server listening on port 4400')
    expect(io.out()).toContain('No device tokens existed')
    expect(io.out()).toContain('vorn_')
  })

  it('does not mint again when a token already exists', async () => {
    initDatabase(dataDir)
    mintOwnerToken('existing')

    const io = capture()
    expect(await runCli(['serve', '--data-dir', dataDir], io)).toBe(0)

    expect(io.out()).toContain('listening on port 4400')
    expect(io.out()).not.toContain('No device tokens existed')
  })

  it('passes host and port through to the server', async () => {
    mockServe(9999)
    const io = capture()
    await runCli(['serve', '--host', '0.0.0.0', '--port', '9999', '--data-dir', dataDir], io)

    expect(startServer).toHaveBeenCalledWith({
      host: '0.0.0.0',
      port: 9999,
      dataDir
    })
    expect(io.out()).toContain('listening on port 9999')
  })
})
