/**
 * vornd's own answers to the reach calls, on a real server that started its
 * own vornd: device tokens, phone pairing end to end, the Origin check and
 * the credential check; and the same calls and checks shadowed, every one
 * compared with the server's.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`).
 */
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '@vornrun/shared/protocol'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { stopServerChild } from './helpers/real-server'

const repoRoot = path.join(__dirname, '..')
const CREDENTIAL = 'native-reach-test-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''
const built = path.join(repoRoot, 'packages', 'core', 'target', 'release')
const vornd = [process.env.VORN_CONFORMANCE_VORND, path.join(built, `vornd${EXE}`)].find(
  (p): p is string =>
    !!p && fs.existsSync(p) && fs.existsSync(path.join(path.dirname(p), `vorn-sessiond${EXE}`))
)

spawnsRealServers()

interface Server {
  child: ChildProcess
  dataDir: string
  /** The server's own port. */
  port: number
  /** vornd's, in front of it. */
  vornd: number
  log: string[]
}

const servers: Server[] = []

async function waitFor<T>(what: string, check: () => Promise<T | null> | T | null): Promise<T> {
  const until = Date.now() + 60_000
  for (;;) {
    const found = await check()
    if (found !== null) return found
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 200))
  }
}

async function startServer(env: Record<string, string>): Promise<Server> {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-reach-'))
  const log: string[] = []
  const child = spawn(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(repoRoot, 'packages', 'server', 'src', 'index.ts'),
      '--data-dir',
      dataDir,
      '--port',
      '0'
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        [BOOTSTRAP_ENV_VAR]: CREDENTIAL,
        VORN_VORND_PATH: vornd!,
        NODE_ENV: 'test',
        VITEST: '',
        ...env
      },
      stdio: ['ignore', 'pipe', 'pipe']
    }
  )
  child.stdout?.on('data', (d) => log.push(String(d)))
  child.stderr?.on('data', (d) => log.push(String(d)))
  const port = await waitFor('the server to listen', () => {
    try {
      const record = JSON.parse(fs.readFileSync(path.join(dataDir, WS_PORT_FILENAME), 'utf-8'))
      return typeof record.port === 'number' ? (record.port as number) : null
    } catch {
      return null
    }
  })
  const direct = await Client.open(port)
  const vorndPort = await waitFor('vornd to start', async () => {
    const status = await direct.result<{ state: string; port?: number }>('server:vornd')
    return status.state === 'on' && status.port ? status.port : null
  })
  direct.close()
  const server = { child, dataDir, port, vornd: vorndPort, log }
  servers.push(server)
  return server
}

/** A WebSocket client that collects what it is told and can make calls. */
class Client {
  private next = 1
  readonly told: Array<{ method: string; params: unknown }> = []
  closed: Promise<number>

  private constructor(private ws: WebSocket) {
    ws.on('message', (raw) => {
      const frame = JSON.parse(String(raw)) as { method?: string; id?: unknown; params?: unknown }
      if (frame.method && frame.id === undefined)
        this.told.push({ method: frame.method, params: frame.params })
    })
    this.closed = new Promise((resolve) => ws.once('close', (code) => resolve(code)))
  }

  static open(
    port: number,
    headers: Record<string, string> = { authorization: `Bearer ${CREDENTIAL}` }
  ): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, { headers })
    return new Promise((resolve, reject) => {
      ws.once('open', () => resolve(new Client(ws)))
      ws.once('error', reject)
      ws.once('unexpected-response', (_req, res) => {
        let body = ''
        res.on('data', (c) => (body += c))
        res.on('end', () =>
          reject(
            Object.assign(new Error(`refused: ${res.statusCode}`), { status: res.statusCode, body })
          )
        )
      })
    })
  }

  call(method: string, params?: unknown): Promise<Record<string, unknown>> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 20_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(String(raw)) as Record<string, unknown>
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  async result<T>(method: string, params?: unknown): Promise<T> {
    const frame = await this.call(method, params)
    if (frame.error) throw new Error(JSON.stringify(frame.error))
    return frame.result as T
  }

  async heard(method: string): Promise<unknown> {
    return waitFor(method, () => this.told.find((t) => t.method === method)?.params ?? null)
  }

  close(): void {
    this.ws.close()
  }
}

async function post(port: number, route: string, body: unknown, type = 'application/json') {
  const res = await fetch(`http://127.0.0.1:${port}${route}`, {
    method: 'POST',
    headers: { 'content-type': type },
    body: typeof body === 'string' ? body : JSON.stringify(body)
  })
  return { status: res.status, body: (await res.json()) as Record<string, unknown> }
}

type Counts = Record<
  string,
  {
    native?: number
    forwarded?: number
    shadowMatched?: number
    shadowMismatched?: number
    shadowUnported?: number
  }
>

async function counts(port: number): Promise<Counts> {
  const res = await fetch(`http://127.0.0.1:${port}/vornd/health`)
  return ((await res.json()) as { groups: Counts }).groups
}

async function refusal(port: number, headers: Record<string, string>) {
  return Client.open(port, headers).then(
    (c) => {
      c.close()
      return null
    },
    (err: { status?: number; body?: string }) => ({ status: err.status, body: err.body })
  )
}

afterAll(async () => {
  for (const s of servers) {
    await stopServerChild(s.child, s.vornd, s.dataDir)
    try {
      fs.rmSync(s.dataDir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    } catch (err) {
      // The session holder outlives the server, and Windows will not delete
      // a running program.
      if (process.platform !== 'win32') throw err
    }
  }
})

describe.skipIf(!vornd)('reach answered by vornd', () => {
  let server: Server
  let desktop: Client

  beforeAll(async () => {
    server = await startServer({})
    desktop = await Client.open(server.vornd)
  }, 120_000)

  afterAll(() => desktop?.close())

  it('mints, lists and revokes device tokens, closing the sockets that hold one', async () => {
    const made = await desktop.result<{ token: { id: string; name: string }; plaintext: string }>(
      'token:create',
      { name: '  Laptop  ' }
    )
    expect(made.token.name).toBe('Laptop')
    expect(made.plaintext).toMatch(/^vorn_[0-9a-f-]{36}_/)
    // vornd wrote it: the server reads the same row.
    const direct = await Client.open(server.port)
    const listed = await direct.result<Array<{ id: string }>>('token:list')
    expect(listed.map((t) => t.id)).toContain(made.token.id)
    expect(await desktop.result('token:list')).toEqual(listed)

    const phone = await Client.open(server.vornd, { authorization: `Bearer ${made.plaintext}` })
    // The server marks the token seen as the phone connects, which vornd,
    // answering at once, can read before it has.
    const seen = await waitFor('the token to be seen', async () => {
      const list =
        await phone.result<Array<{ id: string; lastSeenAt: string | null }>>('token:list')
      return list.find((t) => t.id === made.token.id)?.lastSeenAt ?? null
    })
    expect(seen).toEqual(expect.any(String))
    expect(await desktop.result('token:revoke', made.token.id)).toEqual({ revoked: true })
    expect(await phone.closed).toBe(4002)
    expect(await desktop.result('token:revoke', made.token.id)).toEqual({ revoked: false })

    const again = await Client.open(server.vornd, { authorization: `Bearer ${made.plaintext}` })
    expect(await again.closed).toBe(4002)
    direct.close()
  })

  it('pairs a phone through the server, the desktop told at each step', async () => {
    const { code } = await desktop.result<{ code: string }>('pairing:start')
    // The phone reaches the server, which hands the request to vornd.
    const wrong = await post(server.port, '/api/pair/redeem', {
      code: 'ZZZZ-ZZZZ',
      deviceName: 'x'
    })
    expect(wrong).toEqual({ status: 400, body: { error: 'unknown' } })
    const notJson = await post(server.port, '/api/pair/redeem', 'code', 'text/plain')
    expect(notJson.status).toBe(415)

    const redeemed = await post(server.port, '/api/pair/redeem', { code, deviceName: 'Pixel' })
    expect(redeemed.status).toBe(200)
    const requestId = redeemed.body.requestId as string
    expect(await desktop.heard('pairing:requested')).toMatchObject({
      requestId,
      deviceName: 'Pixel'
    })
    expect(await desktop.result('pairing:pending')).toEqual([
      expect.objectContaining({ requestId, deviceName: 'Pixel' })
    ])
    expect(await post(server.port, '/api/pair/poll', { requestId })).toEqual({
      status: 200,
      body: { status: 'pending' }
    })
    expect(await desktop.result('pairing:approve', { requestId })).toEqual({ ok: true })

    const collected = await post(server.port, '/api/pair/poll', { requestId })
    expect(collected.status).toBe(200)
    expect(collected.body.status).toBe('approved')
    expect(await desktop.heard('pairing:collected')).toEqual({ requestId })
    // Collected once.
    expect((await post(server.port, '/api/pair/poll', { requestId })).body).toEqual({
      status: 'expired'
    })

    const phone = await Client.open(server.vornd, {
      authorization: `Bearer ${collected.body.token as string}`
    })
    const tokens = await phone.result<Array<{ name: string }>>('token:list')
    expect(tokens.map((t) => t.name)).toContain('Pixel')
    phone.close()
  })

  it('refuses a page it does not trust before the server sees it', async () => {
    const host = `127.0.0.1:${server.vornd}`
    expect(await refusal(server.vornd, { origin: 'http://evil.example', host })).toEqual({
      status: 403,
      body: '{"error":"Origin not allowed"}'
    })
    expect(
      await refusal(server.vornd, { origin: 'http://evil.example@127.0.0.1', host })
    ).toMatchObject({ status: 403 })
    expect(await refusal(server.vornd, { origin: `http://${host}`, host })).toBeNull()
    const auth = (await counts(server.vornd)).auth
    expect(auth?.native).toBeGreaterThanOrEqual(3)
  })

  it('admits a socket by its own check of the credential, and the server closes a bad one', async () => {
    const bad = await Client.open(server.vornd, { authorization: 'Bearer vorn_nope' })
    expect(await bad.closed).toBe(4002)
    const browser = await Client.open(server.vornd, {})
    expect((await browser.call('auth:authenticate', { token: CREDENTIAL })).result).toEqual({
      ok: true
    })
    expect(await browser.result('token:list')).toBeInstanceOf(Array)
    browser.close()
  })

  it('answers reachable URLs and Tailscale as the server does', async () => {
    const direct = await Client.open(server.port)
    for (const method of ['server:reachableUrls', 'tailscale:status']) {
      expect(await desktop.result(method)).toEqual(await direct.result(method))
    }
    direct.close()
    const groups = await counts(server.vornd)
    for (const group of ['server', 'tailscale', 'token', 'pairing']) {
      expect({ group, native: (groups[group]?.native ?? 0) > 0 }).toEqual({ group, native: true })
    }
  })
})

describe.skipIf(!vornd)('reach shadowed', () => {
  let server: Server

  beforeAll(async () => {
    server = await startServer({
      VORND_GROUPS: 'server=shadow,tailscale=shadow,token=shadow,pairing=shadow,auth=shadow'
    })
  }, 120_000)

  it("matches the server's answers and its verdicts on every Origin and credential", async () => {
    const desktop = await Client.open(server.vornd)
    const made = await desktop.result<{ plaintext: string }>('token:create', { name: 'shadow' })
    await desktop.result('token:list')
    await desktop.result('server:reachableUrls')
    await desktop.result('tailscale:status')

    const host = `127.0.0.1:${server.vornd}`
    expect(await refusal(server.vornd, { origin: 'http://evil.example', host })).toMatchObject({
      status: 403
    })
    expect(await refusal(server.vornd, { origin: `http://${host}`, host })).toBeNull()

    const phone = await Client.open(server.vornd, { authorization: `Bearer ${made.plaintext}` })
    await phone.result('token:list')
    const bad = await Client.open(server.vornd, {
      authorization: `Bearer ${made.plaintext.slice(0, -2)}xx`
    })
    expect(await bad.closed).toBe(4002)
    const browser = await Client.open(server.vornd, {})
    await browser.result('auth:authenticate', { token: made.plaintext })
    phone.close()
    browser.close()
    desktop.close()

    const groups = await waitFor('the comparisons', async () => {
      const g = await counts(server.vornd)
      return (g.auth?.shadowMatched ?? 0) >= 5 && (g.tailscale?.shadowMatched ?? 0) >= 1 ? g : null
    })
    for (const group of ['auth', 'server', 'tailscale', 'token']) {
      const { shadowMatched = 0, shadowMismatched = 0 } = groups[group] ?? {}
      expect({ group, shadowMismatched, matched: shadowMatched > 0 }).toEqual({
        group,
        shadowMismatched: 0,
        matched: true
      })
    }
  })
})
