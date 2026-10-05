import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'events'

/**
 * Who may become the server's process backend: only a vornd holding the
 * credential the app started the server with, speaking this link protocol,
 * while the Native daemon switch is on. Once linked, its frames are the link's
 * and reach no method handler.
 */

const GOOD_TOKEN = 'valid-credential'
const DEVICE_TOKEN = 'device-credential'

vi.mock('../packages/server/src/broadcast', () => ({
  clientRegistry: { add: vi.fn(), remove: vi.fn(), setTopics: vi.fn(), touch: vi.fn() }
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))
vi.mock('../packages/server/src/ws-auth', () => ({
  authenticateCredential: (raw?: string) =>
    raw === GOOD_TOKEN
      ? { userId: 'owner-1', kind: 'bootstrap' }
      : raw === DEVICE_TOKEN
        ? { userId: 'owner-1', kind: 'device', tokenId: 'tok-1' }
        : null,
  AUTH_TIMEOUT_MS: 10_000
}))

import { handleConnection, setVorndLinkAllowed } from '../packages/server/src/ws-handler'
import { clientRegistry } from '../packages/server/src/broadcast'
import { vorndLink } from '../packages/server/src/vornd-link'

function socket() {
  const ws = Object.assign(new EventEmitter(), {
    sent: [] as Array<Record<string, unknown>>,
    send(text: string) {
      this.sent.push(JSON.parse(text))
    },
    close: vi.fn(function (this: EventEmitter) {
      this.emit('close')
    }),
    readyState: 1,
    OPEN: 1
  })
  return ws
}

type Ws = ReturnType<typeof socket>

async function identify(
  ws: Ws,
  params: Record<string, unknown> = { protocol: 1 }
): Promise<unknown> {
  ws.emit(
    'message',
    Buffer.from(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'vornd:identify', params }))
  )
  await new Promise((r) => setImmediate(r))
  return ws.sent.find((f) => f.id === 1)
}

beforeEach(() => {
  vorndLink.reset()
  setVorndLinkAllowed(() => true)
  vi.mocked(clientRegistry.remove).mockClear()
})

describe('vornd:identify', () => {
  it('links the app’s own vornd, which then hears no broadcasts', async () => {
    const ws = socket()
    handleConnection(ws as never, GOOD_TOKEN)
    expect(await identify(ws)).toMatchObject({ result: { ok: true } })
    expect(vorndLink.owns(ws as never)).toBe(true)
    expect(vorndLink.linked()).toBe(true)
    expect(clientRegistry.remove).toHaveBeenCalledWith(ws)
  })

  it('gives the link its frames, never a method handler', async () => {
    const ws = socket()
    handleConnection(ws as never, GOOD_TOKEN)
    await identify(ws)
    const listed = vorndLink.list()
    const call = ws.sent.at(-1) as { id: number; method: string }
    expect(call.method).toBe('vornd:list')
    ws.emit(
      'message',
      Buffer.from(
        JSON.stringify({
          jsonrpc: '2.0',
          id: call.id,
          result: { connected: true, sessions: [], ended: [] }
        })
      )
    )
    await expect(listed).resolves.toEqual({ connected: true, sessions: [], ended: [] })
    // A method from the link is the link's, not a call this server answers.
    ws.emit(
      'message',
      Buffer.from(JSON.stringify({ jsonrpc: '2.0', id: 9, method: 'config:load' }))
    )
    await new Promise((r) => setImmediate(r))
    expect(ws.sent.some((f) => f.id === 9)).toBe(false)
  })

  it('refuses a device token', async () => {
    const ws = socket()
    handleConnection(ws as never, DEVICE_TOKEN)
    expect(await identify(ws)).toMatchObject({ error: { message: /app’s own vornd/ } })
    expect(vorndLink.linked()).toBe(false)
  })

  it('refuses while the Native daemon switch is off', async () => {
    setVorndLinkAllowed(() => false)
    const ws = socket()
    handleConnection(ws as never, GOOD_TOKEN)
    expect(await identify(ws)).toMatchObject({ error: { message: /switch is off/ } })
    expect(vorndLink.linked()).toBe(false)
  })

  it('refuses a link protocol it does not speak', async () => {
    const ws = socket()
    handleConnection(ws as never, GOOD_TOKEN)
    expect(await identify(ws, { protocol: 2 })).toMatchObject({ error: { message: /protocol 2/ } })
    expect(vorndLink.linked()).toBe(false)
  })

  it('lets a restarted vornd replace the one before it', async () => {
    const first = socket()
    handleConnection(first as never, GOOD_TOKEN)
    await identify(first)
    const second = socket()
    handleConnection(second as never, GOOD_TOKEN)
    await identify(second)
    expect(vorndLink.owns(second as never)).toBe(true)
    expect(first.close).toHaveBeenCalled()
    // The old socket going away does not take the new link with it.
    first.emit('close')
    expect(vorndLink.linked()).toBe(true)
  })
})
