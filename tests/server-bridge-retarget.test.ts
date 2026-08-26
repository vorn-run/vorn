import { describe, it, expect, afterEach } from 'vitest'
import { WebSocketServer, type WebSocket as WsSocket } from 'ws'
import { ServerBridge } from '../src/main/server/server-bridge'

/**
 * Moving the bridge to a different port without losing the connection it makes.
 *
 * A restarted server usually comes back on the port it had — that is the point
 * of remembering it — and the reconnect loop finds it unaided. This is for when
 * it does not, and the bridge would otherwise retry a stale address forever,
 * which looks exactly like a server that never came back.
 *
 * The subtlety is that `close()` is asynchronous. A first version closed the old
 * socket and connected the new one in the same tick, so the old socket's `close`
 * handler ran afterwards and set `this.ws = null` on the *new* connection — the
 * bridge holding no reference to a socket that was open, everything in flight
 * rejected, and a reconnect queued on top of the one just made. Detaching the
 * old listeners first is what stops that, and it is what this pins.
 */

const servers: WebSocketServer[] = []
const bridges: ServerBridge[] = []

afterEach(() => {
  for (const bridge of bridges) bridge.close()
  bridges.length = 0
  for (const server of servers) server.close()
  servers.length = 0
})

/** A socket server that answers nothing, which is all these need. */
async function listen(): Promise<{ port: number; connections: WsSocket[] }> {
  const connections: WsSocket[] = []
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' })
  servers.push(server)
  server.on('connection', (socket) => connections.push(socket))
  await new Promise<void>((resolve) => server.once('listening', () => resolve()))
  const address = server.address()
  return { port: typeof address === 'object' && address ? address.port : 0, connections }
}

function connected(bridge: ServerBridge): Promise<void> {
  return new Promise((resolve) => bridge.once('connected', () => resolve()))
}

describe('retargeting the bridge', () => {
  it('ends up connected to the new address, and stays that way', async () => {
    const first = await listen()
    const second = await listen()

    const bridge = new ServerBridge(`ws://127.0.0.1:${first.port}/ws`)
    bridges.push(bridge)
    bridge.connect()
    await connected(bridge)
    expect(first.connections).toHaveLength(1)

    const arrived = connected(bridge)
    bridge.retarget(`ws://127.0.0.1:${second.port}/ws`)
    await arrived

    expect(bridge.target()).toBe(`ws://127.0.0.1:${second.port}/ws`)
    expect(second.connections).toHaveLength(1)

    // The assertion the bug was hiding behind. The old socket's close handler
    // fires around now; give it room, then check the bridge still holds the new
    // connection rather than having been nulled out by a dead socket's listener.
    await new Promise((resolve) => setTimeout(resolve, 150))
    expect(second.connections[0]?.readyState).toBe(1) // OPEN
    await expect(bridge.request('anything', undefined, 200)).rejects.toThrow(/timeout|timed out/i)
  })

  it('does nothing when asked for the address it already has', async () => {
    const only = await listen()
    const bridge = new ServerBridge(`ws://127.0.0.1:${only.port}/ws`)
    bridges.push(bridge)
    bridge.connect()
    await connected(bridge)

    bridge.retarget(`ws://127.0.0.1:${only.port}/ws`)
    await new Promise((resolve) => setTimeout(resolve, 150))

    // One connection, not a second one made for no reason.
    expect(only.connections).toHaveLength(1)
  })
})
