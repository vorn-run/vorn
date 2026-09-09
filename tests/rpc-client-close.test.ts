import { describe, it, expect, vi, beforeEach } from 'vitest'

interface Fake {
  sent: string[]
  emit(event: string, ...args: unknown[]): void
}

/** Every socket the client opened, so a test can answer or close it by hand. */
const sockets = vi.hoisted(() => [] as Fake[])

vi.mock('ws', () => {
  class FakeSocket implements Fake {
    private handlers: Record<string, ((...args: unknown[]) => void)[]> = {}
    sent: string[] = []

    constructor() {
      sockets.push(this)
      setTimeout(() => this.emit('open'), 0)
    }

    on(event: string, handler: (...args: unknown[]) => void): this {
      ;(this.handlers[event] ??= []).push(handler)
      return this
    }

    emit(event: string, ...args: unknown[]): void {
      for (const handler of this.handlers[event] ?? []) handler(...args)
    }

    send(data: string): void {
      this.sent.push(data)
    }

    // Hostile on purpose: a socket that reports the close synchronously is what
    // would let the close handler settle the call before its own answer did.
    close(): void {
      this.emit('close', 1000)
    }
  }
  return { WebSocket: FakeSocket }
})

vi.mock('node:fs', () => {
  const fs = {
    readFileSync: (file: string) =>
      String(file).endsWith('ws-port')
        ? JSON.stringify({ port: 50091, pid: process.pid })
        : 'vorn_token',
    existsSync: () => true,
    mkdirSync: () => undefined,
    writeFileSync: () => undefined
  }
  return { default: fs, ...fs }
})

import { rpcCall } from '../packages/server/src/rpc-client'

/** The socket for the call just made, once it has sent its request. */
async function sent(): Promise<Fake> {
  await vi.waitFor(() => expect(sockets.at(-1)?.sent.length).toBe(1))
  return sockets.at(-1)!
}

beforeEach(() => {
  sockets.length = 0
})

describe('a method the server does not have', () => {
  it('reads as an older server, not as a mystery about a name', async () => {
    const call = rpcCall('workflow:run')
    await vi.waitFor(() => expect(sockets.at(-1)?.sent.length).toBe(1))
    const socket = sockets.at(-1)!
    const { id } = JSON.parse(socket.sent[0]) as { id: number }

    socket.emit(
      'message',
      Buffer.from(JSON.stringify({ id, error: { message: 'Method not found: workflow:run' } }))
    )

    await expect(call).rejects.toThrow(/older than the vorn command/)
  })

  it('leaves every other error exactly as the server put it', async () => {
    const call = rpcCall('workflow:run')
    await vi.waitFor(() => expect(sockets.at(-1)?.sent.length).toBe(1))
    const socket = sockets.at(-1)!
    const { id } = JSON.parse(socket.sent[0]) as { id: number }

    socket.emit(
      'message',
      Buffer.from(JSON.stringify({ id, error: { message: 'Workflow not found' } }))
    )

    await expect(call).rejects.toThrow('Workflow not found')
  })
})

describe('a call the server never answers', () => {
  it('names a refused credential rather than blaming the clock', async () => {
    const call = rpcCall('terminal:listActive')
    // 4002 is CLOSE_CREDENTIAL_REJECTED: another server holds this port.
    ;(await sent()).emit('close', 4002)

    await expect(call).rejects.toThrow(/refused the credential/)
  })

  it('says the connection closed when the code means nothing in particular', async () => {
    const call = rpcCall('terminal:listActive')
    ;(await sent()).emit('close', 1006)

    await expect(call).rejects.toThrow(/closed the connection before answering \(code 1006\)/)
  })

  it('still resolves when the answer arrives before the close', async () => {
    const call = rpcCall('terminal:listActive')
    const socket = await sent()
    const { id } = JSON.parse(socket.sent[0]) as { id: number }

    socket.emit('message', Buffer.from(JSON.stringify({ id, result: [] })))
    socket.emit('close', 1000)

    await expect(call).resolves.toEqual([])
  })
})
