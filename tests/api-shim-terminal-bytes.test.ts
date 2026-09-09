import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { encodeTerminalFrame } from '../packages/shared/src/terminal-frame'

/**
 * The web client's half of terminal output as bytes.
 *
 * A browser socket has to be told to hand over an ArrayBuffer rather than a
 * Blob, has to ask for bytes with every filter it sets -- the server keeps the
 * choice, but the client cannot know which server it has -- and has to give a
 * frame to the same listeners a JSON notification reaches.
 */

const sockets: FakeSocket[] = []

class FakeSocket {
  static OPEN = 1
  binaryType = 'blob'
  onopen: (() => void) | null = null
  onmessage: ((e: { data: string | ArrayBuffer }) => void) | null = null
  onclose: ((e: { code: number }) => void) | null = null
  onerror: (() => void) | null = null
  readyState = 1
  sent: string[] = []

  constructor(public url: string) {
    sockets.push(this)
  }

  send(data: string): void {
    this.sent.push(data)
  }
  close(): void {}

  open(): void {
    this.onopen?.()
  }
  authOk(): void {
    this.onmessage?.({ data: JSON.stringify({ jsonrpc: '2.0', method: 'auth:ok', params: {} }) })
  }
  /** What the socket hands over for a binary message, once `binaryType` is set. */
  binary(bytes: Uint8Array): void {
    this.onmessage?.({
      data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
    })
  }
}

let createApiShim: typeof import('../packages/web/src/api-shim').createApiShim

beforeEach(async () => {
  sockets.length = 0
  vi.stubGlobal('WebSocket', FakeSocket)
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k)
  })
  vi.resetModules()
  createApiShim = (await import('../packages/web/src/api-shim')).createApiShim
})

afterEach(() => {
  vi.unstubAllGlobals()
})

const sentMethods = (socket: FakeSocket): Array<{ method: string; params: unknown }> =>
  socket.sent.map((raw) => JSON.parse(raw))

describe('terminal output as bytes, in the browser', () => {
  it('opens the socket for ArrayBuffers', () => {
    createApiShim('ws://x/ws')

    expect(sockets[0].binaryType).toBe('arraybuffer')
  })

  it('asks for bytes as soon as it is let in, and again with every filter', async () => {
    const api = createApiShim('ws://x/ws')
    sockets[0].open()
    sockets[0].authOk()

    const onAuth = sentMethods(sockets[0]).find((m) => m.method === 'subscribe:set')
    expect(onAuth?.params).toMatchObject({ terminalBytes: true })

    void api.setTopics?.(['session:*'])
    const withTopics = sentMethods(sockets[0])
      .filter((m) => m.method === 'subscribe:set')
      .at(-1)
    expect(withTopics?.params).toEqual({ topics: ['session:*'], terminalBytes: true })
  })

  it('gives a frame to whoever listens for terminal data', () => {
    const api = createApiShim('ws://x/ws')
    const seen: unknown[] = []
    api.onTerminalData((event) => seen.push(event))
    const data = new TextEncoder().encode('\u001b[32m$\u001b[0m ')

    sockets[0].binary(encodeTerminalFrame({ id: 'term-9', seq: 2, data }))

    expect(seen).toEqual([{ id: 'term-9', seq: 2, data }])
  })

  it('ignores binary that is not a frame', () => {
    const api = createApiShim('ws://x/ws')
    const seen: unknown[] = []
    api.onTerminalData((event) => seen.push(event))

    sockets[0].binary(Uint8Array.from([0, 1, 2]))

    expect(seen).toEqual([])
  })
})
