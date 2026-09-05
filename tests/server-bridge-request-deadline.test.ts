import { describe, it, expect, afterEach } from 'vitest'
import { WebSocketServer } from 'ws'
import { ServerBridge } from '../src/main/server/server-bridge'

const servers: WebSocketServer[] = []
const bridges: ServerBridge[] = []
const timers: NodeJS.Timeout[] = []

afterEach(() => {
  for (const timer of timers) clearTimeout(timer)
  timers.length = 0
  for (const bridge of bridges) bridge.close()
  bridges.length = 0
  for (const server of servers) server.close()
  servers.length = 0
})

async function bridgeTo(answerAfterMs: number): Promise<ServerBridge> {
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' })
  servers.push(server)
  server.on('connection', (socket) => {
    socket.on('message', (raw) => {
      const { id } = JSON.parse(raw.toString()) as { id: number }
      timers.push(
        setTimeout(
          () => socket.send(JSON.stringify({ jsonrpc: '2.0', id, result: 'done' })),
          answerAfterMs
        )
      )
    })
  })
  await new Promise<void>((resolve) => server.once('listening', () => resolve()))
  const address = server.address()
  const port = typeof address === 'object' && address ? address.port : 0
  const bridge = new ServerBridge(`ws://127.0.0.1:${port}/ws`)
  bridges.push(bridge)
  bridge.connect()
  await new Promise<void>((resolve) => bridge.once('connected', () => resolve()))
  return bridge
}

describe('a request with no deadline', () => {
  it('waits for an answer that a deadline would have cut off', async () => {
    const bridge = await bridgeTo(120)
    await expect(bridge.request('script:execute', undefined, 50)).rejects.toThrow(/timed out/)
    await expect(bridge.request('script:execute', undefined, 0)).resolves.toBe('done')
  })

  it('still rejects when the server goes away', async () => {
    const bridge = await bridgeTo(10_000)
    const pending = bridge.request('script:execute', undefined, 0)
    for (const server of servers) for (const client of server.clients) client.terminate()
    await expect(pending).rejects.toThrow(/disconnected/i)
  })
})
