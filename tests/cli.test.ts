import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { runCli, type CliDeps } from '../packages/server/src/cli'
import { initDatabase, closeDatabase } from '../packages/server/src/database'
import { mintOwnerToken } from '../packages/server/src/token-manager'

let dataDir: string

// `serve` runs vornd; a stand-in keeps the test to the CLI's dispatch, output and first-run mint.
const runVornd = vi.fn(async () => 0)
const findVornd = vi.fn(() => ({ vornd: '/opt/vorn/vornd', sessiond: '/opt/vorn/vorn-sessiond' }))

/** Collects what the CLI wrote, so assertions read against real output. */
function capture(isTty = false): CliDeps & { out: () => string; err: () => string } {
  const outParts: string[] = []
  const errParts: string[] = []
  return {
    write: (t) => outParts.push(t),
    writeErr: (t) => errParts.push(t),
    isTty,
    findVornd,
    runVornd,
    out: () => outParts.join(''),
    err: () => errParts.join('')
  }
}

/** `token create` etc. open and close the database themselves. */
function run(argv: string[]) {
  const io = capture()
  return runCli([...argv, '--data-dir', dataDir], io).then((code) => ({ code, io }))
}

beforeEach(() => {
  runVornd.mockClear()
  findVornd.mockClear()
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-cli-'))
})

afterEach(() => {
  closeDatabase()
  fs.rmSync(dataDir, { recursive: true, force: true })
})

describe('usage and dispatch', () => {
  it('prints usage to stdout and succeeds for --help', async () => {
    const io = capture()
    expect(await runCli(['--help'], io)).toBe(0)
    expect(io.out()).toContain('vorn: Vorn from the command line')
    expect(io.err()).toBe('')
  })

  it('accepts the help subcommand too', async () => {
    const io = capture()
    expect(await runCli(['help'], io)).toBe(0)
    expect(io.out()).toContain('Usage')
  })

  it('describes the server commands under their own noun', async () => {
    const io = capture()
    expect(await runCli(['server', '--help'], io)).toBe(0)
    expect(io.out()).toContain('vorn server: run a Vorn server')
  })

  it('still takes the bare serve and token commands vorn-server was called with', async () => {
    const io = capture()
    expect(await runCli(['server'], io)).toBe(2)
    expect(io.err()).toContain('vorn server serve')
  })

  it('treats a bare invocation as a usage error, on stderr', async () => {
    const io = capture()
    expect(await runCli([], io)).toBe(2)
    expect(io.err()).toContain('Usage')
    expect(io.out()).toBe('')
  })

  it("finds the command after a global option, not the option's value", async () => {
    const io = capture()
    expect(await runCli(['--data-dir', dataDir, 'token', 'list'], io)).toBe(0)
    expect(io.out()).toBe('No device tokens.\n')
  })

  it('finds it after the noun the server commands live under too', async () => {
    const io = capture()
    expect(await runCli(['--data-dir', dataDir, 'server', 'token', 'list'], io)).toBe(0)
    expect(io.out()).toBe('No device tokens.\n')
  })

  it('names the option that swallowed the command, not the leftovers', async () => {
    const io = capture()
    expect(await runCli(['--data-dir', 'session', 'list'], io)).toBe(2)
    expect(io.err()).toContain('--data-dir needs a value; it took "session" as one')

    const io2 = capture()
    expect(await runCli(['--data-dir', 'session'], io2)).toBe(2)
    expect(io2.err()).toContain('--data-dir needs a value')
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
    expect(runVornd).not.toHaveBeenCalled()
  })

  it('reports an unknown option', async () => {
    const io = capture()
    expect(await runCli(['serve', '--nope'], io)).toBe(2)
    expect(io.err()).toContain('vorn:')
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
  it('keeps the first-run token out of a stream nobody is watching', async () => {
    const io = capture(false)
    const code = await runCli(['serve', '--data-dir', dataDir], io)

    expect(code).toBe(0)
    expect(io.out()).not.toContain('vorn_')
    expect(io.out()).toContain('vorn server token create')
  })

  it('runs vornd for the data dir and mints a first-run token on an empty one', async () => {
    const io = capture(true)
    const code = await runCli(['serve', '--data-dir', dataDir], io)

    expect(code).toBe(0)
    expect(runVornd).toHaveBeenCalledTimes(1)
    const [binaries, args] = runVornd.mock.calls[0] as unknown as [unknown, string[]]
    expect(binaries).toEqual({ vornd: '/opt/vorn/vornd', sessiond: '/opt/vorn/vorn-sessiond' })
    expect(args.slice(0, 4)).toEqual([
      '--data-dir',
      dataDir,
      '--sessiond',
      '/opt/vorn/vorn-sessiond'
    ])
    expect(io.out()).toContain(`Starting the Vorn server for ${dataDir}`)
    expect(io.out()).toContain('No device tokens existed')
    expect(io.out()).toContain('vorn_')
  })

  it('does not mint again when a token already exists', async () => {
    initDatabase(dataDir)
    mintOwnerToken('existing')
    closeDatabase()

    const io = capture()
    expect(await runCli(['serve', '--data-dir', dataDir], io)).toBe(0)

    expect(io.out()).toContain('Starting the Vorn server')
    expect(io.out()).not.toContain('No device tokens existed')
  })

  it('passes host and port through, and vornd’s exit code back', async () => {
    runVornd.mockResolvedValueOnce(3)
    const io = capture()
    const code = await runCli(
      ['serve', '--host', '0.0.0.0', '--port', '9999', '--data-dir', dataDir],
      io
    )

    expect(code).toBe(3)
    const [, args] = runVornd.mock.calls[0] as unknown as [unknown, string[]]
    expect(args.slice(-4)).toEqual(['--port', '9999', '--host', '0.0.0.0'])
  })

  it('says so when this install has no vornd', async () => {
    findVornd.mockReturnValueOnce(null as never)
    const io = capture()
    expect(await runCli(['serve', '--data-dir', dataDir], io)).toBe(1)
    expect(io.err()).toContain('no vornd')
    expect(runVornd).not.toHaveBeenCalled()
  })

  it('will not serve the default data directory from source', async () => {
    const io = capture()
    expect(await runCli(['serve', '--data-dir', path.join(os.homedir(), '.vorn')], io)).toBe(1)
    expect(io.err()).toContain('default data directory')
    expect(runVornd).not.toHaveBeenCalled()
  })
})
