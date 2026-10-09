import http from 'node:http'
import net from 'node:net'
import { describe, expect, it } from 'vitest'
import type { Transport } from '@modelcontextprotocol/sdk/shared/transport.js'
import type { JSONRPCMessage } from '@modelcontextprotocol/sdk/types.js'
import type { VorndStatus } from '../packages/shared/src/types'
import {
  relay,
  relayHeaders,
  vorndMcpUrl,
  type Backoff,
  type Upstream
} from '../packages/mcp/src/relay'

function health(status: number): typeof fetch {
  return (async () =>
    new Response(JSON.stringify({ ok: status === 200 }), { status })) as typeof fetch
}

const on = async (): Promise<VorndStatus> => ({ state: 'on', port: 4321 })

describe('whether to relay to vornd', () => {
  it('relays only when vornd answers its health check', async () => {
    const up = health(200)
    expect((await vorndMcpUrl({ vorndStatus: on, fetch: up }))?.href).toBe(
      'http://127.0.0.1:4321/mcp'
    )
    expect(await vorndMcpUrl({ vorndStatus: async () => ({ state: 'off' }), fetch: up })).toBeNull()
    expect(await vorndMcpUrl({ vorndStatus: on, fetch: health(503) })).toBeNull()
  })

  it('serves the tools itself when the server or vornd cannot be asked', async () => {
    const failing = async (): Promise<never> => {
      throw new Error('Method not found: server:vornd')
    }
    expect(await vorndMcpUrl({ vorndStatus: failing, fetch: health(200) })).toBeNull()
    const unreachable = (async () => {
      throw new TypeError('fetch failed')
    }) as typeof fetch
    expect(await vorndMcpUrl({ vorndStatus: on, fetch: unreachable })).toBeNull()
  })
})

describe('relayHeaders', () => {
  it("carries the credential, the agent's directory and its session", () => {
    expect(relayHeaders('tok', '/home/me/my app', { VORN_SESSION_ID: 's-1' })).toEqual({
      Authorization: 'Bearer tok',
      'Vorn-Cwd': '%2Fhome%2Fme%2Fmy%20app',
      'Vorn-Session-Id': 's-1'
    })
    expect(relayHeaders('tok', '/', { VORN_SESSION_ID: '' })).not.toHaveProperty('Vorn-Session-Id')
  })
})

/** A transport the test plays the agent's side of. */
class Agent implements Transport {
  onmessage?: (message: JSONRPCMessage) => void
  onclose?: () => void
  received: JSONRPCMessage[] = []
  async start(): Promise<void> {}
  async send(message: JSONRPCMessage): Promise<void> {
    this.received.push(message)
  }
  async close(): Promise<void> {
    this.onclose?.()
  }
}

/** A port nothing listens on. */
async function closedPort(): Promise<number> {
  const server = net.createServer()
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = (server.address() as net.AddressInfo).port
  await new Promise<void>((resolve) => server.close(() => resolve()))
  return port
}

describe('relay', () => {
  it('answers a request vornd cannot be reached for, and ends with the agent', async () => {
    const agent = new Agent()
    const url = new URL(`http://127.0.0.1:${await closedPort()}/mcp`)
    const done = relay(
      agent,
      { url, headers: relayHeaders('tok', '/', {}) },
      { locate: async () => null, fetch, backoff: { firstMs: 5, maxMs: 10, giveUpAfterMs: 50 } }
    )
    await new Promise((resolve) => setImmediate(resolve))
    agent.onmessage!({ jsonrpc: '2.0', method: 'notifications/initialized' })
    agent.onmessage!({ jsonrpc: '2.0', id: 9, method: 'ping' })
    await expect.poll(() => agent.received.length).toBe(1)
    const [answer] = agent.received as Array<{
      id: number
      error: { code: number; message: string }
    }>
    expect(answer.id).toBe(9)
    expect(answer.error.code).toBe(-32603)
    expect(answer.error.message).toMatch(/^vornd's MCP server did not answer: /)
    await agent.close()
    await done
  })
})

const FAST: Backoff = { firstMs: 10, maxMs: 50, giveUpAfterMs: 10_000 }
const PROTOCOL = '2025-06-18'

interface Seen {
  method: string
  rpc?: string
  session?: string
  protocol?: string
}

/** vornd's `/mcp` as far as a relay can tell: sessions in memory, gone when it stops. */
class FakeVornd {
  seen: Seen[] = []
  private server?: http.Server
  port = 0

  constructor(
    private credential: string,
    private session: string,
    /** Hangs up instead of answering, for the first `n` requests of a method. */
    private hangUp: Record<string, number> = {}
  ) {}

  static async start(credential: string, session: string, port = 0): Promise<FakeVornd> {
    const vornd = new FakeVornd(credential, session)
    await vornd.listen(port)
    return vornd
  }

  async listen(port: number): Promise<this> {
    this.server = http.createServer((req, res) => void this.answer(req, res))
    await new Promise<void>((resolve) => this.server!.listen(port, '127.0.0.1', resolve))
    this.port = (this.server.address() as net.AddressInfo).port
    return this
  }

  get upstream(): Upstream {
    return {
      url: new URL(`http://127.0.0.1:${this.port}/mcp`),
      headers: relayHeaders(this.credential, '/', {})
    }
  }

  async stop(): Promise<void> {
    this.server!.closeAllConnections()
    await new Promise<void>((resolve) => this.server!.close(() => resolve()))
  }

  posted(rpc: string): Seen[] {
    return this.seen.filter((s) => s.rpc === rpc)
  }

  private async answer(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    let raw = ''
    for await (const chunk of req) raw += chunk
    const body = raw ? (JSON.parse(raw) as { id?: number; method?: string }) : {}
    const header = (name: string): string | undefined => req.headers[name] as string | undefined
    this.seen.push({
      method: req.method!,
      rpc: body.method,
      session: header('mcp-session-id'),
      protocol: header('mcp-protocol-version')
    })
    if (header('authorization') !== relayHeaders(this.credential, '/', {}).Authorization) {
      res.writeHead(401).end('who are you')
      return
    }
    if (body.method && (this.hangUp[body.method] ?? 0) > 0) {
      this.hangUp[body.method]--
      req.socket.destroy()
      return
    }
    if (req.method === 'DELETE') {
      res.writeHead(200).end()
      return
    }
    const json = (value: unknown, headers: Record<string, string> = {}): void => {
      res.writeHead(200, { 'content-type': 'application/json', ...headers })
      res.end(JSON.stringify(value))
    }
    if (body.method === 'initialize') {
      json(
        { jsonrpc: '2.0', id: body.id, result: { protocolVersion: PROTOCOL, from: this.session } },
        { 'mcp-session-id': this.session }
      )
    } else if (header('mcp-session-id') !== this.session) {
      res.writeHead(404).end('Session not found')
    } else if (body.id === undefined) {
      res.writeHead(202).end()
    } else {
      json({ jsonrpc: '2.0', id: body.id, result: { from: this.session } })
    }
  }
}

type Answer = { id: number; result?: { from: string }; error?: { code: number; message: string } }

/** The agent's side of a running relay. */
async function connect(
  first: Upstream,
  locate: () => Promise<Upstream | null>,
  backoff = FAST
): Promise<{ agent: Agent; done: Promise<void>; ask: (id: number, method: string) => void }> {
  const agent = new Agent()
  const done = relay(agent, first, { locate, fetch, backoff })
  await new Promise((resolve) => setImmediate(resolve))
  const ask = (id: number, method: string): void =>
    agent.onmessage!({ jsonrpc: '2.0', id, method, params: {} })
  return { agent, done, ask }
}

async function handshake(agent: Agent, ask: (id: number, method: string) => void): Promise<void> {
  ask(0, 'initialize')
  await expect.poll(() => agent.received.length).toBe(1)
  agent.onmessage!({ jsonrpc: '2.0', method: 'notifications/initialized' })
}

const answers = (agent: Agent): Answer[] =>
  (agent.received as unknown as Answer[]).slice().sort((a, b) => a.id - b.id)

describe('relay across a vornd restart', () => {
  it('reopens the session when vornd comes back on the same port', async () => {
    const before = await FakeVornd.start('tok', 's1')
    const { agent, done, ask } = await connect(before.upstream, async () => before.upstream)
    await handshake(agent, ask)
    ask(1, 'tools/list')
    await expect.poll(() => agent.received.length).toBe(2)
    expect(answers(agent)[1].result).toEqual({ from: 's1' })

    await before.stop()
    ask(2, 'tools/list')
    ask(3, 'resources/list')
    await new Promise((resolve) => setTimeout(resolve, 150))
    const after = await new FakeVornd('tok', 's2').listen(before.port)

    await expect.poll(() => agent.received.length).toBe(4)
    expect(answers(agent).slice(2)).toEqual([
      { jsonrpc: '2.0', id: 2, result: { from: 's2' } },
      { jsonrpc: '2.0', id: 3, result: { from: 's2' } }
    ])
    const inits = after.posted('initialize')
    expect(inits, 'one reconnect for both requests').toHaveLength(1)
    expect(inits[0].session).toBeUndefined()
    expect(after.posted('notifications/initialized')).toEqual([
      { method: 'POST', rpc: 'notifications/initialized', session: 's2', protocol: PROTOCOL }
    ])
    expect(after.posted('tools/list').at(-1)).toMatchObject({ session: 's2', protocol: PROTOCOL })

    await agent.close()
    await done
    // The replayed initialize was answered to the relay, not the agent.
    expect(agent.received).toHaveLength(4)
    expect(after.seen.at(-1)).toMatchObject({ method: 'DELETE', session: 's2' })
    await after.stop()
  })

  it('follows vornd to a new port and credential', async () => {
    const before = await FakeVornd.start('old-token', 's1')
    let where: Upstream | null = before.upstream
    const { agent, done, ask } = await connect(before.upstream, async () => where)
    await handshake(agent, ask)

    await before.stop()
    where = null
    ask(1, 'tools/list')
    await new Promise((resolve) => setTimeout(resolve, 100))
    const after = await FakeVornd.start('new-token', 's2')
    where = after.upstream

    await expect.poll(() => agent.received.length).toBe(2)
    expect(answers(agent)[1]).toEqual({ jsonrpc: '2.0', id: 1, result: { from: 's2' } })
    expect(after.posted('initialize')).toHaveLength(1)
    expect(before.posted('tools/list')).toHaveLength(0)

    await agent.close()
    await done
    await after.stop()
  })

  it('fails what waited but stays open when vornd does not come back in time', async () => {
    const vornd = await FakeVornd.start('tok', 's1')
    let where: Upstream | null = vornd.upstream
    const { agent, done, ask } = await connect(vornd.upstream, async () => where, {
      firstMs: 10,
      maxMs: 20,
      giveUpAfterMs: 150
    })
    await handshake(agent, ask)

    await vornd.stop()
    where = null
    ask(1, 'tools/list')
    await expect.poll(() => agent.received.length).toBe(2)
    const [, failed] = answers(agent)
    expect(failed.id).toBe(1)
    expect(failed.error?.code).toBe(-32603)
    expect(failed.error?.message).toMatch(/did not come back within 0\.15s/)

    const back = await FakeVornd.start('tok', 's2')
    where = back.upstream
    ask(2, 'tools/list')
    await expect.poll(() => agent.received.length).toBe(3)
    expect(answers(agent)[2]).toEqual({ jsonrpc: '2.0', id: 2, result: { from: 's2' } })

    await agent.close()
    await done
    await back.stop()
  })

  it('never sends a tool call twice, but sends again what is harmless to repeat', async () => {
    const vornd = await new FakeVornd('tok', 's1', { 'tools/call': 1, 'tools/list': 1 }).listen(0)
    const { agent, done, ask } = await connect(vornd.upstream, async () => vornd.upstream)
    await handshake(agent, ask)

    ask(1, 'tools/call')
    await expect.poll(() => agent.received.length).toBe(2)
    expect(answers(agent)[1].error?.message).toMatch(/^vornd's MCP server did not answer: /)
    expect(vornd.posted('tools/call')).toHaveLength(1)

    ask(2, 'tools/list')
    await expect.poll(() => agent.received.length).toBe(3)
    expect(answers(agent)[2]).toEqual({ jsonrpc: '2.0', id: 2, result: { from: 's1' } })
    expect(vornd.posted('tools/list')).toHaveLength(2)

    await agent.close()
    await done
    await vornd.stop()
  })

  it('fails what vornd keeps turning away after a bounded number of resends', async () => {
    const vornd = await new FakeVornd('tok', 's1', { 'resources/list': 10 }).listen(0)
    const { agent, done, ask } = await connect(vornd.upstream, async () => vornd.upstream)
    await handshake(agent, ask)
    const started = Date.now()
    ask(1, 'resources/list')
    await expect.poll(() => agent.received.length).toBe(2)
    expect(answers(agent)[1].error?.message).toMatch(/kept turning the request away/)
    expect(vornd.posted('resources/list')).toHaveLength(4)
    expect(vornd.posted('initialize')).toHaveLength(1 + 3)
    expect(Date.now() - started).toBeLessThan(FAST.giveUpAfterMs / 2)

    await agent.close()
    await done
    await vornd.stop()
  })

  it('says at once when the credential was never taken', async () => {
    const vornd = await FakeVornd.start('tok', 's1')
    const wrong = { ...vornd.upstream, headers: relayHeaders('another-token', '/', {}) }
    const { agent, done, ask } = await connect(wrong, async () => wrong)
    const started = Date.now()
    ask(0, 'initialize')
    await expect.poll(() => agent.received.length).toBe(1)
    expect(Date.now() - started).toBeLessThan(FAST.giveUpAfterMs / 2)
    expect(answers(agent)[0].error?.message).toMatch(/401/)
    expect(vornd.seen).toHaveLength(1)

    await agent.close()
    await done
    await vornd.stop()
  })
})
