import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { builtSessiond, builtVornd, startServed, type Served } from './helpers/served'

/**
 * Vorn with nothing but vornd and its session holder: no Node server
 * process anywhere. A client finds the server by what it publishes,
 * authenticates, subscribes, hears what changes and drives a terminal; a
 * phone pairs over HTTP; a browser loads the web client. Nothing is handed
 * to anything behind vornd, because there is nothing behind it.
 */

const CREDENTIAL = 'vorn-without-node-credential'

interface Frame {
  id?: number
  method?: string
  params?: Record<string, unknown>
  result?: unknown
  error?: { code: number; message: string }
}

/** A client of the server: what it was told, and calls by id. */
class Client {
  readonly told: Frame[] = []
  closed: number | null = null
  private next = 1
  private constructor(readonly ws: WebSocket) {
    ws.on('message', (raw, binary) => {
      if (binary) return
      this.told.push(JSON.parse(String(raw)) as Frame)
    })
    ws.on('close', (code) => {
      this.closed = code
    })
  }

  static open(port: number, options: { token?: string; query?: string; origin?: string } = {}) {
    const headers: Record<string, string> = {}
    if (options.token) headers.authorization = `Bearer ${options.token}`
    if (options.origin) headers.origin = options.origin
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws${options.query ?? ''}`, { headers })
    return new Promise<Client>((resolve, reject) => {
      ws.once('open', () => resolve(new Client(ws)))
      ws.once('unexpected-response', (_req, res) => reject(new Error(`upgrade ${res.statusCode}`)))
      ws.once('error', reject)
    })
  }

  async call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    return until(`${method} answered`, () => this.told.find((f) => f.id === id))
  }

  notify(method: string, params?: unknown): void {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
  }

  saw(method: string): Frame[] {
    return this.told.filter((f) => f.method === method)
  }
}

async function until<T>(what: string, check: () => T | undefined | false): Promise<T> {
  const deadline = Date.now() + 20_000
  for (;;) {
    const got = check()
    if (got) return got
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 25))
  }
}

/** Every process under `pid`, by command name. */
function descendants(pid: number): string[] {
  const rows = execFileSync('ps', ['-A', '-o', 'pid=,ppid=,comm='], { encoding: 'utf-8' })
    .trim()
    .split('\n')
    .map((line) => line.trim().split(/\s+/, 3))
  const found: string[] = []
  const walk = (parent: string): void => {
    for (const [p, pp, comm] of rows) {
      if (pp === parent) {
        found.push(comm ?? '')
        walk(p!)
      }
    }
  }
  walk(String(pid))
  return found
}

describe.runIf(builtVornd && builtSessiond && process.platform !== 'win32')(
  'Vorn with no Node server',
  () => {
    let served: Served
    let web: string

    beforeAll(async () => {
      web = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-web-'))
      fs.writeFileSync(path.join(web, 'index.html'), '<!doctype html><title>Vorn</title>')
      served = await startServed({
        credential: CREDENTIAL,
        sessiond: true,
        args: ['--web', web]
      })
      await until('the session holder', async () => {
        const res = await fetch(`http://127.0.0.1:${served.port}/vornd/health`)
        const health = (await res.json()) as { sessiond?: { current?: { pid?: number } } }
        return !!health.sessiond?.current?.pid
      })
    }, 60_000)

    afterAll(async () => {
      await served?.stop()
      for (const dir of [served?.dataDir, served?.home, web]) {
        if (dir) fs.rmSync(dir, { recursive: true, force: true })
      }
    })

    it('publishes where it is and how to reach it, and runs no Node', () => {
      const record = JSON.parse(fs.readFileSync(path.join(served.dataDir, 'ws-port'), 'utf-8'))
      expect(record).toEqual({ port: served.port, pid: served.child.pid })
      expect(fs.readFileSync(path.join(served.dataDir, 'local-token'), 'utf-8')).toBe(CREDENTIAL)
      const under = descendants(served.child.pid!)
      expect(under.some((c) => /(^|\/)node$/.test(c))).toBe(false)
      expect(under.some((c) => c.includes('vorn-sessiond'))).toBe(true)
    })

    it('greets a socket, takes only authentication before it, and admits by message', async () => {
      const early = await Client.open(served.port)
      await until('the greeting', () => early.saw('server:hello')[0])
      const refused = await early.call('config:load')
      expect(refused.error?.code).toBe(-32001)
      await until('the close', () => early.closed)
      expect(early.closed).toBe(4001)

      const phone = await Client.open(served.port)
      const ok = await phone.call('auth:authenticate', { token: CREDENTIAL })
      expect(ok.result).toEqual({ ok: true })
      expect(phone.saw('auth:ok')).toHaveLength(1)
      const config = await phone.call('config:load')
      expect(config.result).toHaveProperty('projects')
      phone.ws.close()
    })

    it('refuses a page from somewhere else, and takes its own', async () => {
      await expect(
        Client.open(served.port, { token: CREDENTIAL, origin: 'https://evil.example' })
      ).rejects.toThrow('upgrade 403')
      const own = await Client.open(served.port, {
        token: CREDENTIAL,
        origin: `http://127.0.0.1:${served.port}`
      })
      own.ws.close()
    })

    it('tells each client what it asked for, and drives a terminal', async () => {
      const watching = await Client.open(served.port, {
        token: CREDENTIAL,
        query: '?topics=session:*'
      })
      const elsewhere = await Client.open(served.port, {
        token: CREDENTIAL,
        query: '?topics=config:changed'
      })
      const shell = await watching.call('shell:create', served.dataDir)
      const id = (shell.result as { id: string }).id
      await until('the new terminal told', () =>
        watching.saw('session:created').find((f) => f.params?.id === id)
      )
      await until('its program up', () =>
        watching
          .saw('session:updated')
          .find((f) => f.params?.id === id && Number(f.params?.pid) > 0)
      )
      watching.notify('terminal:write', { id, data: 'echo vorn-alone\r' })
      await until('the echo', async () => {
        const out = (await watching.call('terminal:readOutput', { id })).result as string[]
        return out.some((line) => line.includes('vorn-alone'))
      })
      expect(elsewhere.saw('session:created')).toEqual([])
      const killed = await watching.call('terminal:kill', id)
      expect(killed.error).toBeUndefined()
      watching.ws.close()
      elsewhere.ws.close()
    })

    it('pairs a phone over HTTP, approved on the desktop', async () => {
      const post = async (route: string, body: unknown, type = 'application/json') => {
        const res = await fetch(`http://127.0.0.1:${served.port}${route}`, {
          method: 'POST',
          headers: { 'content-type': type },
          body: typeof body === 'string' ? body : JSON.stringify(body)
        })
        return { status: res.status, json: (await res.json()) as Record<string, unknown> }
      }
      expect((await post('/api/pair/redeem', 'code=AAAA-AAAA', 'text/plain')).status).toBe(415)
      expect((await post('/api/pair/redeem', { code: 'AAAA-AAAA', deviceName: 'x' })).status).toBe(
        400
      )

      const desktop = await Client.open(served.port, {
        token: CREDENTIAL,
        query: '?topics=pairing:*'
      })
      const started = (await desktop.call('pairing:start')).result as { code: string }
      const redeemed = await post('/api/pair/redeem', { code: started.code, deviceName: 'Phone' })
      const requestId = String(redeemed.json.requestId)
      await until('the request told', () => desktop.saw('pairing:requested')[0])
      expect((await post('/api/pair/poll', { requestId })).json).toEqual({ status: 'pending' })
      expect((await desktop.call('pairing:approve', { requestId })).result).toEqual({ ok: true })
      const collected = await post('/api/pair/poll', { requestId })
      expect(collected.json.status).toBe('approved')
      const token = String(collected.json.token)
      desktop.ws.close()

      // The token it collected opens a socket of its own.
      const phone = await Client.open(served.port, { token })
      expect((await phone.call('config:load')).error).toBeUndefined()
      phone.ws.close()
    })

    it('serves the web client, which no page may frame', async () => {
      const res = await fetch(`http://127.0.0.1:${served.port}/app/tasks`)
      expect(res.status).toBe(200)
      expect(res.headers.get('x-frame-options')).toBe('DENY')
      expect(await res.text()).toContain('<title>Vorn</title>')
      expect(await (await fetch(`http://127.0.0.1:${served.port}/health`)).json()).toEqual({
        status: 'ok'
      })
    })

    it('handed nothing on, there being nothing behind it', async () => {
      const res = await fetch(`http://127.0.0.1:${served.port}/vornd/health`)
      const health = (await res.json()) as {
        unexpectedForwards: Record<string, number>
        upstream: { address: string | null }
      }
      expect(health.unexpectedForwards).toEqual({})
      expect(health.upstream.address).toBeNull()
    })
  }
)
