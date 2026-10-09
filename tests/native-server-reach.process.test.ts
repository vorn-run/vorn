/**
 * vornd's own answers to the reach calls, as the server: device tokens, phone
 * pairing end to end, the Origin check and the credential check.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`).
 */
import fs from 'node:fs'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { builtSessiond, builtVornd, startServed, type Served } from './helpers/served'

const CREDENTIAL = 'native-reach-test-credential'

spawnsRealServers()

interface Server {
  served: Served
  port: number
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

async function startServer(): Promise<Server> {
  const served = await startServed({ credential: CREDENTIAL, sessiond: true })
  const server = { served, port: served.port }
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

type Counts = Record<string, { native?: number; forwarded?: number }>

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
  for (const { served } of servers) {
    await served.stop()
    for (const dir of [served.dataDir, served.home]) {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    }
  }
})

describe.skipIf(!builtVornd || !builtSessiond)('reach answered by vornd', () => {
  let server: Server
  let desktop: Client

  beforeAll(async () => {
    server = await startServer()
    desktop = await Client.open(server.port)
  }, 120_000)

  afterAll(() => desktop?.close())

  it('mints, lists and revokes device tokens, closing the sockets that hold one', async () => {
    const made = await desktop.result<{ token: { id: string; name: string }; plaintext: string }>(
      'token:create',
      { name: '  Laptop  ' }
    )
    expect(made.token.name).toBe('Laptop')
    expect(made.plaintext).toMatch(/^vorn_[0-9a-f-]{36}_/)
    const direct = await Client.open(server.port)
    const listed = await direct.result<Array<{ id: string }>>('token:list')
    expect(listed.map((t) => t.id)).toContain(made.token.id)
    expect(await desktop.result('token:list')).toEqual(listed)

    const phone = await Client.open(server.port, { authorization: `Bearer ${made.plaintext}` })
    // Marked seen as the phone connects, which a call may read before it is.
    const seen = await waitFor('the token to be seen', async () => {
      const list =
        await phone.result<Array<{ id: string; lastSeenAt: string | null }>>('token:list')
      return list.find((t) => t.id === made.token.id)?.lastSeenAt ?? null
    })
    expect(seen).toEqual(expect.any(String))
    expect(await desktop.result('token:revoke', made.token.id)).toEqual({ revoked: true })
    expect(await phone.closed).toBe(4002)
    expect(await desktop.result('token:revoke', made.token.id)).toEqual({ revoked: false })

    const again = await Client.open(server.port, { authorization: `Bearer ${made.plaintext}` })
    expect(await again.closed).toBe(4002)
    direct.close()
  })

  it('pairs a phone over HTTP, the desktop told at each step', async () => {
    const { code } = await desktop.result<{ code: string }>('pairing:start')
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

    const phone = await Client.open(server.port, {
      authorization: `Bearer ${collected.body.token as string}`
    })
    const tokens = await phone.result<Array<{ name: string }>>('token:list')
    expect(tokens.map((t) => t.name)).toContain('Pixel')
    phone.close()
  })

  it('refuses a page it does not trust', async () => {
    const host = `127.0.0.1:${server.port}`
    expect(await refusal(server.port, { origin: 'http://evil.example', host })).toEqual({
      status: 403,
      body: '{"error":"Origin not allowed"}'
    })
    expect(
      await refusal(server.port, { origin: 'http://evil.example@127.0.0.1', host })
    ).toMatchObject({ status: 403 })
    expect(await refusal(server.port, { origin: `http://${host}`, host })).toBeNull()
  })

  it('admits a socket by its check of the credential, and closes a bad one', async () => {
    const bad = await Client.open(server.port, { authorization: 'Bearer vorn_nope' })
    expect(await bad.closed).toBe(4002)
    const browser = await Client.open(server.port, {})
    expect((await browser.call('auth:authenticate', { token: CREDENTIAL })).result).toEqual({
      ok: true
    })
    expect(await browser.result('token:list')).toBeInstanceOf(Array)
    browser.close()
  })

  it('answers reachable URLs and Tailscale itself', async () => {
    expect(await desktop.result('server:reachableUrls')).toEqual(expect.anything())
    await desktop.result('tailscale:status')
    const groups = await counts(server.port)
    for (const group of ['server', 'tailscale', 'token', 'pairing']) {
      expect({ group, native: (groups[group]?.native ?? 0) > 0 }).toEqual({ group, native: true })
    }
  })
})
