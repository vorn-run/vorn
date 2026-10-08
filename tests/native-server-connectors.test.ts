/**
 * vornd's own answers to the `connection:` and `connector:` calls, against
 * the server's answers to the same calls.
 *
 * One server is started on a real database, and three vornds in front of it:
 * one answering them itself, one with the desktop's launch token as well, and one shadowing the same groups. Calls that change nothing are
 * made directly to the server and through vornd on the same connections.
 * Calls that start an MCP connection's child are made once on each side, on
 * two connections with the same settings, against the fixture MCP server.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'

const TEST_CREDENTIAL = 'native-server-connectors-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''
const FIXTURE = path.resolve(__dirname, 'fixtures/mcp-server.mjs')

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

type Frame = Record<string, unknown>

/** A WebSocket client that sends one call at a time and returns its frame. */
class Client {
  private next = 1
  private constructor(private ws: WebSocket) {}

  static async open(port: number): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    await new Promise<void>((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })
    const client = new Client(ws)
    // A socket that opened with the bearer header is admitted silently; the
    // first answer the server sends it shows vornd it was.
    await client.call('config:load')
    return client
  }

  call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(raw.toString()) as Frame
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  /** The call's result; throws on an error frame. */
  async result<T = unknown>(method: string, params?: unknown): Promise<T> {
    const frame = await this.call(method, params)
    if (frame.error) throw new Error(`${method}: ${JSON.stringify(frame.error)}`)
    return frame.result as T
  }

  close(): void {
    this.ws.close()
  }
}

/** A frame without its id, as plain JSON. */
function answerOf(frame: Frame): Frame {
  const { id: _id, ...rest } = frame
  return JSON.parse(JSON.stringify(rest)) as Frame
}

interface Vornd {
  port: number
  child: ChildProcess
}

async function startVornd(
  upstream: number,
  args: string[],
  env: Record<string, string> = {}
): Promise<Vornd> {
  const child = spawn(vornd!, ['--upstream', `127.0.0.1:${upstream}`, ...args], {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: {
      ...process.env,
      VORND_LOG: process.env.VORND_LOG ?? 'warn',
      // Secrets stay in memory: a test run never writes the OS keychain.
      VORND_KEYCHAIN: '0',
      ...env
    }
  })
  const port = await new Promise<number>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vornd did not start')), 10_000)
    createInterface({ input: child.stdout! }).once('line', (line) => {
      clearTimeout(timer)
      resolve((JSON.parse(line) as { port: number }).port)
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`vornd exited with ${code} before listening`))
    })
  })
  return { port, child }
}

function stopVornd(v: Vornd | undefined): Promise<void> {
  if (!v || v.child.exitCode !== null || v.child.signalCode !== null) return Promise.resolve()
  return new Promise((resolve) => {
    v.child.once('exit', () => resolve())
    v.child.kill()
  })
}

type Counts = Record<
  string,
  {
    mode?: string
    forwarded?: number
    native?: number
    shadowMatched?: number
    shadowMismatched?: number
    shadowUnported?: number
  }
>

async function counts(v: Vornd): Promise<Counts> {
  const res = await fetch(`http://127.0.0.1:${v.port}/vornd/health`)
  return ((await res.json()) as { groups: Counts }).groups
}

interface Connection {
  id: string
  connectorId: string
  filters: Record<string, unknown>
  lastSyncAt?: string
  lastSyncError?: string
}

let serverPort: number
let closeServer: () => Promise<void>
let native: Vornd | undefined
let desktop: Vornd | undefined
let shadow: Vornd | undefined
let dataDir: string
let repos: string
let direct: Client
let through: Client

/** An MCP connection that runs the fixture. */
function fixtureFilters(extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    command: process.execPath,
    args: JSON.stringify([FIXTURE]),
    env: JSON.stringify({ FIXTURE_PLAIN: 'plain value' }),
    ...extra
  }
}

async function createConnection(
  name: string,
  connectorId: string,
  filters: Record<string, unknown>
): Promise<string> {
  const conn = await direct.result<Connection>('connection:create', {
    connectorId,
    name,
    filters,
    syncIntervalMinutes: 0,
    statusMapping: {}
  })
  return conn.id
}

async function connectionRow(id: string): Promise<Connection> {
  const all = await direct.result<Connection[]>('connection:list', {})
  const row = all.find((c) => c.id === id)
  if (!row) throw new Error(`no connection ${id}`)
  return row
}

describe.skipIf(!vornd)('the native server answers connection calls as the server does', () => {
  beforeAll(async () => {
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    dataDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-connectors-')))
    const { startServer } = await import('../packages/server/src/index')
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    try {
      const { app, port } = await startServer({ port: 0, dataDir })
      serverPort = port
      closeServer = () => app.close()
    } finally {
      process.stdout.write = origWrite
    }
    const db = path.join(dataDir, 'vorn.db')
    native = await startVornd(serverPort, ['--db', db])
    desktop = await startVornd(serverPort, ['--db', db], {
      VORND_DESKTOP_TOKEN: TEST_CREDENTIAL
    })
    shadow = await startVornd(serverPort, [
      '--groups',
      'connection=shadow,connector=shadow',
      '--db',
      db
    ])
    direct = await Client.open(serverPort)
    through = await Client.open(native.port)

    repos = path.join(dataDir, 'repos')
    for (const [name, origin] of [
      ['github', 'git@github.com:vorn-run/vorn.git'],
      ['elsewhere', 'https://gitlab.com/a/b.git'],
      ['no-origin', null]
    ] as const) {
      const dir = path.join(repos, name)
      fs.mkdirSync(dir, { recursive: true })
      execFileSync('git', ['init', '-q', dir])
      if (origin) execFileSync('git', ['-C', dir, 'remote', 'add', 'origin', origin])
    }
  }, 60_000)

  afterAll(async () => {
    delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    direct?.close()
    through?.close()
    await Promise.all([stopVornd(native), stopVornd(desktop), stopVornd(shadow)])
    await closeServer?.()
    if (dataDir) fs.rmSync(dataDir, { recursive: true, force: true })
  })

  it('answers every call that changes nothing with the server’s frame', async () => {
    const mcp = await createConnection('Fixture', 'mcp', fixtureFilters())
    await direct.result('connection:refreshMcpTools', mcp)
    const http = await createConnection('Profile', 'http', { baseUrl: 'https://example.invalid' })

    const calls: Array<[string, unknown]> = [
      ['connection:list', {}],
      ['connection:list', { connectorId: 'mcp' }],
      ['connection:list', { connectorId: 'nothing' }],
      ['connection:getSourceLink', 'no-such-task'],
      ['connection:listMcpTools', mcp],
      ['connection:listMcpTools', http],
      ['connection:listMcpTools', 'no-such-connection'],
      ['connection:listActions', mcp],
      ['connection:listActions', http],
      ['connection:listActions', 'no-such-connection'],
      ['connection:preflight', mcp],
      ['connection:preflight', 'no-such-connection'],
      ['connector:detectRepo', path.join(repos, 'github')],
      ['connector:detectRepo', path.join(repos, 'elsewhere')],
      ['connector:detectRepo', path.join(repos, 'no-origin')],
      ['connector:detectRepo', path.join(repos, 'missing')],
      ['connector:detectRepo', 'relative/path']
    ]
    for (const [method, params] of calls) {
      const want = answerOf(await direct.call(method, params))
      const got = answerOf(await through.call(method, params))
      expect(got, `${method} ${JSON.stringify(params)}`).toEqual(want)
    }
    expect(
      answerOf(await through.call('connector:detectRepo', path.join(repos, 'github')))
    ).toEqual({ jsonrpc: '2.0', result: { owner: 'vorn-run', repo: 'vorn' } })

    const viaShadow = await Client.open(shadow!.port)
    try {
      for (const [method, params] of calls) {
        const want = answerOf(await direct.call(method, params))
        expect(answerOf(await viaShadow.call(method, params)), `${method}`).toEqual(want)
      }
    } finally {
      viaShadow.close()
    }
    const shadowed = await counts(shadow!)
    const nativeCounts = await counts(native!)
    for (const group of ['connection', 'connector']) {
      expect(nativeCounts[group]?.mode).toBe('native')
      expect(nativeCounts[group]?.native ?? 0).toBeGreaterThan(0)
      expect(shadowed[group]?.shadowMatched ?? 0).toBeGreaterThan(0)
      expect(shadowed[group]?.shadowMismatched ?? 0).toBe(0)
    }
  })

  it('discovers tools and runs them as the server does', async () => {
    const theirs = await createConnection('Theirs', 'mcp', fixtureFilters())
    const mine = await createConnection('Mine', 'mcp', fixtureFilters())

    const before = (await counts(native!)).connection?.native ?? 0
    const refreshed = await through.result('connection:refreshMcpTools', mine)
    expect(refreshed).toEqual(await direct.result('connection:refreshMcpTools', theirs))
    expect(refreshed).toEqual({ ok: true, count: 5 })
    expect((await counts(native!)).connection?.native ?? 0).toBe(before + 1)

    const [a, b] = await Promise.all([connectionRow(theirs), connectionRow(mine)])
    expect(b.filters).toEqual(a.filters)
    expect(b.lastSyncAt).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/)
    expect(b).not.toHaveProperty('lastSyncError')

    const runs: Array<[string, Record<string, unknown> | undefined]> = [
      ['echo', { text: 'hi', count: '3', ratio: '', loud: 'true', tags: '["x","y"]', mode: '3' }],
      ['echo', { text: 'plain', count: 'many', loud: 'no' }],
      ['echo', undefined],
      ['structured', {}],
      ['fails', {}],
      ['secret', {}],
      ['no-such-tool', {}]
    ]
    for (const [action, args] of runs) {
      const want = answerOf(
        await direct.call('connection:executeAction', { connectionId: theirs, action, args })
      )
      const got = answerOf(
        await through.call('connection:executeAction', { connectionId: mine, action, args })
      )
      expect(got, `${action} ${JSON.stringify(args)}`).toEqual(want)
    }
    expect(answerOf(await through.call('connection:listMcpTools', mine))).toEqual(
      answerOf(await direct.call('connection:listMcpTools', theirs))
    )
  })

  it('words a connection that cannot start as the server does', async () => {
    const pairs: Array<Record<string, unknown>> = [
      { command: '', args: '[]' },
      { command: 'vorn-no-such-command', args: '[]' },
      fixtureFilters({ args: JSON.stringify([path.join(dataDir, 'missing.mjs')]) })
    ]
    for (const filters of pairs) {
      const theirs = await createConnection('Broken theirs', 'mcp', filters)
      const mine = await createConnection('Broken mine', 'mcp', filters)
      const refreshed = answerOf(await through.call('connection:refreshMcpTools', mine))
      expect(refreshed, `${JSON.stringify(filters)}`).toEqual(
        answerOf(await direct.call('connection:refreshMcpTools', theirs))
      )
      const [a, b] = await Promise.all([connectionRow(theirs), connectionRow(mine)])
      expect(b.lastSyncError).toBe(a.lastSyncError)
      expect(b.filters).toEqual(a.filters)
      const call = { action: 'echo', args: { text: 'x' } }
      expect(
        answerOf(await through.call('connection:executeAction', { connectionId: mine, ...call }))
      ).toEqual(
        answerOf(await direct.call('connection:executeAction', { connectionId: theirs, ...call }))
      )
    }
    expect(
      answerOf(
        await through.call('connection:executeAction', { connectionId: 'gone', action: 'echo' })
      )
    ).toEqual(
      answerOf(
        await direct.call('connection:executeAction', { connectionId: 'gone', action: 'echo' })
      )
    )
  })

  it('runs with the secrets the desktop pushes, and finds them again in the vault', async () => {
    // What the desktop would decrypt is pushed in plain text; the row holds
    // only the stored form.
    const stored = { secretEnv: 'stored-ciphertext' }
    const theirs = await createConnection('Secret theirs', 'mcp', fixtureFilters(stored))
    const mine = await createConnection('Secret mine', 'mcp', fixtureFilters(stored))
    const secret = JSON.stringify({ FIXTURE_SECRET: 'first' })
    for (const connectionId of [theirs, mine]) {
      await through.result('credentials:setDecrypted', {
        connectionId,
        fields: { secretEnv: secret }
      })
    }
    const run = (client: Client, connectionId: string): Promise<Frame> =>
      client.call('connection:executeAction', { connectionId, action: 'secret', args: {} })

    const want = answerOf(await run(direct, theirs))
    expect(want.result).toEqual({
      success: true,
      output: { content: [{ type: 'text', text: '{"secret":"first","plain":"plain value"}' }] }
    })
    expect(answerOf(await run(through, mine))).toEqual(want)

    // This vornd never saw the push, but reads it from the vault the first one filed it in.
    const desk = await Client.open(desktop!.port)
    try {
      const before = (await counts(desktop!)).connection ?? {}
      expect(answerOf(await run(desk, mine))).toEqual(want)
      const after = (await counts(desktop!)).connection ?? {}
      expect(after.native ?? 0).toBe((before.native ?? 0) + 1)
      expect(after.forwarded ?? 0).toBe(before.forwarded ?? 0)
    } finally {
      desk.close()
    }

    // A rotated secret is used from the next call on, on both sides.
    const rotated = JSON.stringify({ FIXTURE_SECRET: 'second' })
    for (const [client, connectionId] of [
      [direct, theirs],
      [through, mine]
    ] as const) {
      expect(
        await client.result('connection:rotateSecret', {
          connectionId,
          field: 'secretEnv',
          value: 'rotated-ciphertext',
          plaintext: rotated
        })
      ).toEqual({ ok: true })
    }
    const second = answerOf(await run(direct, theirs))
    expect(JSON.stringify(second)).toContain('second')
    expect(answerOf(await run(through, mine))).toEqual(second)

    // A deleted connection is not found by either.
    await through.result('connection:delete', mine)
    await direct.result('connection:delete', theirs)
    expect(answerOf(await run(through, mine))).toEqual(answerOf(await run(direct, mine)))
  })

  it('sends the calls the server keeps to the server', async () => {
    const before = (await counts(native!)).connector?.forwarded ?? 0
    const listed = answerOf(await through.call('connector:list'))
    expect(listed).toEqual(answerOf(await direct.call('connector:list')))
    expect((await counts(native!)).connector?.forwarded ?? 0).toBe(before + 1)
  })
})
