import { describe, it, expect, afterEach } from 'vitest'
import { WebSocketServer, type WebSocket as WsSocket } from 'ws'
import { ServerBridge } from '../src/main/server/server-bridge'
import { encodeTerminalFrame } from '../packages/shared/src/terminal-frame'

/**
 * The desktop's half of terminal output as bytes.
 *
 * Two things, both on every connection because the choice dies with the
 * socket: the bridge asks for bytes as soon as the server says it can send
 * them, and a binary frame comes out as the same notification a JSON one did,
 * so nothing downstream knows the wire changed.
 */

const servers: WebSocketServer[] = []
const bridges: ServerBridge[] = []

afterEach(() => {
  for (const bridge of bridges) bridge.close()
  bridges.length = 0
  for (const server of servers) server.close()
  servers.length = 0
})

async function listen(): Promise<{ port: number; sockets: WsSocket[] }> {
  const sockets: WsSocket[] = []
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' })
  servers.push(server)
  server.on('connection', (socket) => sockets.push(socket))
  await new Promise<void>((resolve) => server.once('listening', () => resolve()))
  const address = server.address()
  return { port: typeof address === 'object' && address ? address.port : 0, sockets }
}

async function connect(): Promise<{ bridge: ServerBridge; socket: WsSocket }> {
  const { port, sockets } = await listen()
  const bridge = new ServerBridge(`ws://127.0.0.1:${port}/ws`)
  bridges.push(bridge)
  bridge.connect()
  await new Promise<void>((resolve) => bridge.once('connected', () => resolve()))
  return { bridge, socket: sockets[0] }
}

function hello(capabilities: Record<string, number>): string {
  return JSON.stringify({
    jsonrpc: '2.0',
    method: 'server:hello',
    params: { protocolVersion: 1, capabilities }
  })
}

describe('terminal output as bytes, at the bridge', () => {
  it('asks for bytes when the server says it can send them', async () => {
    const { socket } = await connect()
    // The bridge identifies itself first; the request for bytes follows the hello.
    const asked = new Promise<string>((resolve) =>
      socket.on('message', (raw) => {
        if (raw.toString().includes('subscribe:set')) resolve(raw.toString())
      })
    )

    socket.send(hello({ auth: 1, subscribe: 1, terminalBytes: 1 }))

    expect(JSON.parse(await asked)).toEqual({
      jsonrpc: '2.0',
      method: 'subscribe:set',
      params: { terminalBytes: true }
    })
  })

  it('asks for nothing of a server that cannot', async () => {
    const { socket } = await connect()
    const received: string[] = []
    socket.on('message', (raw) => received.push(JSON.parse(raw.toString()).method))

    socket.send(hello({ auth: 1, subscribe: 1 }))
    await new Promise((r) => setTimeout(r, 50))

    expect(received).not.toContain('subscribe:set')
  })

  it('hands a frame on as the notification it stands for', async () => {
    const { bridge, socket } = await connect()
    const arrived = new Promise<[string, unknown]>((resolve) =>
      bridge.once('server-notification', (method: string, params: unknown) =>
        resolve([method, params])
      )
    )
    const data = new TextEncoder().encode('\u001b[1mhi\u001b[0m')

    socket.send(encodeTerminalFrame({ id: 'term-1', seq: 4, data }), { binary: true })

    const [method, params] = await arrived
    expect(method).toBe('terminal:data')
    expect(params).toEqual({ id: 'term-1', seq: 4, data })
  })

  it('drops binary it cannot read rather than passing it on', async () => {
    const { bridge, socket } = await connect()
    const notifications: string[] = []
    bridge.on('server-notification', (method: string) => notifications.push(method))

    socket.send(Uint8Array.from([9, 9, 9]), { binary: true })
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'session:updated', params: {} }))
    await new Promise((r) => setTimeout(r, 50))

    expect(notifications).toEqual(['session:updated'])
  })
})
