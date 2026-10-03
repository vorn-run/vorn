import { describe, it, expect, vi, afterEach } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  ClientRegistry,
  parseTopics,
  RESYNC_HIGH_WATER,
  RESYNC_LOW_WATER
} from '../packages/server/src/broadcast'
import { decodeTerminalFrame } from '../packages/shared/src/terminal-frame'

function mockWs(open = true) {
  const ws = {
    readyState: open ? 1 : 3,
    OPEN: 1,
    send: vi.fn()
  }
  return ws as unknown as import('ws').WebSocket
}

describe('ClientRegistry', () => {
  it('add increases size', () => {
    const reg = new ClientRegistry()
    reg.add(mockWs())
    expect(reg.size).toBe(1)
  })

  it('remove decreases size', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)
    reg.remove(ws)
    expect(reg.size).toBe(0)
  })

  it('broadcast sends JSON to all open clients', () => {
    const reg = new ClientRegistry()
    const ws1 = mockWs()
    const ws2 = mockWs()
    reg.add(ws1)
    reg.add(ws2)
    reg.broadcast('test:event', { data: 1 })
    expect(ws1.send).toHaveBeenCalledOnce()
    expect(ws2.send).toHaveBeenCalledOnce()
    const sent = JSON.parse((ws1.send as ReturnType<typeof vi.fn>).mock.calls[0][0])
    expect(sent.method).toBe('test:event')
    expect(sent.params).toEqual({ data: 1 })
  })

  it('broadcast skips closed clients', () => {
    const reg = new ClientRegistry()
    const open = mockWs(true)
    const closed = mockWs(false)
    reg.add(open)
    reg.add(closed)
    reg.broadcast('test:event', {})
    expect(open.send).toHaveBeenCalledOnce()
    expect(closed.send).not.toHaveBeenCalled()
  })

  it('broadcast on empty registry does nothing', () => {
    const reg = new ClientRegistry()
    expect(() => reg.broadcast('test:event', {})).not.toThrow()
  })
})

/**
 * Every client used to receive every notification. That is right for a desktop and
 * wrong for a phone: `terminal:data` alone is every byte of every PTY on the
 * machine, over cellular, to a client that renders none of it.
 */
describe('what one client wants', () => {
  const sentMethods = (ws: import('ws').WebSocket): string[] =>
    (ws.send as unknown as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => JSON.parse(c[0] as string).method
    )

  it('sends everything to a client that asked for nothing', () => {
    // The default, and what every client shipped before this existed relies on.
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)

    reg.broadcast('terminal:data', { id: 'a', data: 'x' })
    reg.broadcast('config:changed', {})

    expect(sentMethods(ws)).toEqual(['terminal:data', 'config:changed'])
  })

  it('withholds a notification outside the list', () => {
    const reg = new ClientRegistry()
    const phone = mockWs()
    reg.add(phone, ['session:*', 'config:changed'])

    reg.broadcast('session:updated', {})
    reg.broadcast('terminal:data', { id: 'a', data: 'x' })
    reg.broadcast('config:changed', {})

    expect(sentMethods(phone)).toEqual(['session:updated', 'config:changed'])
  })

  it('lets a wildcard cover a notification added later', () => {
    // The point of a namespace over a list of names: a client already on a phone
    // keeps working when the namespace grows.
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws, ['session:*'])

    reg.broadcast('session:somethingAddedNextYear', {})

    expect(sentMethods(ws)).toEqual(['session:somethingAddedNextYear'])
  })

  it('does not let an exact name match by prefix', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws, ['session:updated'])

    reg.broadcast('session:updatedSomethingElse', {})

    expect(sentMethods(ws)).toEqual([])
  })

  it('filters each client independently', () => {
    const reg = new ClientRegistry()
    const desktop = mockWs()
    const phone = mockWs()
    reg.add(desktop)
    reg.add(phone, ['session:*'])

    reg.broadcast('terminal:data', { id: 'a', data: 'x' })

    expect(sentMethods(desktop)).toEqual(['terminal:data'])
    expect(sentMethods(phone)).toEqual([])
  })

  it('serialises nothing when no one wants it', () => {
    // Not a micro-optimisation: with only a phone attached, this is the whole PTY
    // firehose that no longer gets JSON-encoded.
    const reg = new ClientRegistry()
    reg.add(mockWs(), ['session:*'])
    const payload = { id: 'a' }
    const toJSON = vi.fn(() => ({}))

    reg.broadcast('terminal:data', Object.assign(payload, { toJSON }))

    expect(toJSON).not.toHaveBeenCalled()
  })

  it('widens back to everything when the list is set empty', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws, ['session:*'])

    reg.setTopics(ws, [])
    reg.broadcast('terminal:data', {})

    expect(sentMethods(ws)).toEqual(['terminal:data'])
  })

  it('ignores a topic list for a socket that was never admitted', () => {
    // Or `setTopics` would be a way for an unauthenticated socket to add itself.
    const reg = new ClientRegistry()
    const stranger = mockWs()

    reg.setTopics(stranger, ['session:*'])
    reg.broadcast('session:updated', {})

    expect(stranger.send).not.toHaveBeenCalled()
    expect(reg.size).toBe(0)
  })

  it('falls back to everything on a malformed list', () => {
    // Failing open costs bandwidth. Failing closed silences the client, which
    // reads as a broken app rather than a bad parameter.
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws, [42 as unknown as string])

    reg.broadcast('terminal:data', {})

    expect(sentMethods(ws)).toEqual(['terminal:data'])
  })
})

describe('the topic list on the socket URL', () => {
  it('splits a comma-separated list', () => {
    expect(parseTopics({ topics: 'session:*,config:changed' })).toEqual([
      'session:*',
      'config:changed'
    ])
  })

  it('tolerates spacing around the commas', () => {
    expect(parseTopics({ topics: ' session:* , config:changed ' })).toEqual([
      'session:*',
      'config:changed'
    ])
  })

  it.each([
    ['no query at all', undefined],
    ['no topics parameter', {}],
    ['an empty value', { topics: '' }],
    ['only separators', { topics: ',,' }],
    ['a repeated parameter, which Fastify gives as an array', { topics: ['a', 'b'] }]
  ])('reads %s as no filter', (_label, query) => {
    // Undefined means everything, so an unparseable list leaves the client exactly
    // as it was before this feature existed rather than silencing it.
    expect(parseTopics(query)).toBeUndefined()
  })
})

/**
 * Subscribing to `terminal:data` by name means every byte of every terminal on
 * the machine. On a phone showing one session that is the wrong unit entirely.
 */
describe('subscribing to one terminal', () => {
  const sentMethods = (ws: import('ws').WebSocket): string[] =>
    (ws.send as unknown as ReturnType<typeof vi.fn>).mock.calls.map(
      (c) => JSON.parse(c[0] as string).method
    )

  it('delivers the terminal that was asked for', () => {
    const reg = new ClientRegistry()
    const phone = mockWs()
    reg.add(phone, ['terminal:data#term-1'])

    reg.broadcast('terminal:data', { id: 'term-1', data: 'x' }, 'term-1')

    expect(sentMethods(phone)).toEqual(['terminal:data'])
  })

  it('withholds every other terminal', () => {
    const reg = new ClientRegistry()
    const phone = mockWs()
    reg.add(phone, ['terminal:data#term-1'])

    reg.broadcast('terminal:data', { id: 'term-2', data: 'x' }, 'term-2')

    expect(sentMethods(phone)).toEqual([])
  })

  it('still gives every terminal to a client that asked by name', () => {
    // A desktop renders all of them, and must not be narrowed by the instance
    // form existing.
    const reg = new ClientRegistry()
    const desktop = mockWs()
    reg.add(desktop, ['terminal:data'])

    reg.broadcast('terminal:data', { id: 'term-1', data: 'x' }, 'term-1')
    reg.broadcast('terminal:data', { id: 'term-2', data: 'y' }, 'term-2')

    expect(sentMethods(desktop)).toEqual(['terminal:data', 'terminal:data'])
  })

  it('does not match an instance subscription against a scopeless broadcast', () => {
    const reg = new ClientRegistry()
    const phone = mockWs()
    reg.add(phone, ['terminal:data#term-1'])

    reg.broadcast('terminal:data', { data: 'x' })

    expect(sentMethods(phone)).toEqual([])
  })

  it('lets one client follow a terminal while another follows nothing', () => {
    const reg = new ClientRegistry()
    const watching = mockWs()
    const listing = mockWs()
    reg.add(watching, ['session:*', 'terminal:data#term-1'])
    reg.add(listing, ['session:*'])

    reg.broadcast('terminal:data', { id: 'term-1', data: 'x' }, 'term-1')
    reg.broadcast('session:updated', { id: 'term-1' }, 'term-1')

    expect(sentMethods(watching)).toEqual(['terminal:data', 'session:updated'])
    expect(sentMethods(listing)).toEqual(['session:updated'])
  })
})

// Two wire forms; each socket gets the one it asked for, and one that never asked sees no change.
describe('terminal output as bytes', () => {
  const sent = (ws: import('ws').WebSocket): unknown[] =>
    (ws.send as ReturnType<typeof vi.fn>).mock.calls.map((call) => call[0])

  it('is text for a socket that never asked', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)
    reg.broadcast('terminal:data', { id: 'a', data: 'hi', seq: 1 }, 'a')

    expect(JSON.parse(sent(ws)[0] as string)).toEqual({
      jsonrpc: '2.0',
      method: 'terminal:data',
      params: { id: 'a', data: 'hi', seq: 1 }
    })
  })

  it('is a frame for a socket that asked, and text for its neighbour', () => {
    const reg = new ClientRegistry()
    const bytes = mockWs()
    const text = mockWs()
    reg.add(bytes)
    reg.add(text)
    reg.setTopics(bytes, undefined, true)
    reg.broadcast('terminal:data', { id: 'a', data: '\u001b[1mx', seq: 9 }, 'a')

    expect(decodeTerminalFrame(sent(bytes)[0] as Uint8Array)).toEqual({
      id: 'a',
      seq: 9,
      data: new TextEncoder().encode('\u001b[1mx')
    })
    expect(typeof sent(text)[0]).toBe('string')
  })

  it('keeps the choice when the topics change without mentioning it', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)
    reg.setTopics(ws, undefined, true)
    // The web client pushes its topics whole on every scroll.
    reg.setTopics(ws, ['terminal:data#a'])
    reg.broadcast('terminal:data', { id: 'a', data: 'x', seq: 1 }, 'a')

    expect(sent(ws)[0]).toBeInstanceOf(Uint8Array)
  })

  it('still honours the topic filter for bytes', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)
    reg.setTopics(ws, ['terminal:data#a'], true)
    reg.broadcast('terminal:data', { id: 'b', data: 'x', seq: 1 }, 'b')

    expect(ws.send).not.toHaveBeenCalled()
  })

  it('cannot be asked for by a socket that was never admitted', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.setTopics(ws, undefined, true)
    reg.broadcast('terminal:data', { id: 'a', data: 'x', seq: 1 }, 'a')

    expect(ws.send).not.toHaveBeenCalled()
  })

  it('leaves the topics alone when a request does not mention them', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws, ['session:*'])
    // The desktop asks for bytes and nothing else; the phone's URL filter must survive it.
    reg.setTopics(ws, undefined, true)
    reg.broadcast('terminal:data', { id: 'a', data: 'x', seq: 1 }, 'a')
    reg.broadcast('session:updated', { id: 'a' })

    expect(ws.send).toHaveBeenCalledOnce()
    expect(typeof sent(ws)[0]).toBe('string')
  })

  it('takes only a plain true as asking for bytes', () => {
    const reg = new ClientRegistry()
    const ws = mockWs()
    reg.add(ws)
    reg.setTopics(ws, undefined, 'yes')
    reg.broadcast('terminal:data', { id: 'a', data: 'x', seq: 1 }, 'a')

    expect(typeof sent(ws)[0]).toBe('string')
  })
})

/**
 * A client that cannot keep up with a burst.
 *
 * Every flush queued for a socket that is not draining only lengthens the wait
 * before that client sees the present, and grows the server by however fast the
 * program prints. A client that promised to re-attach has its output withheld
 * past the high mark and is told to re-attach once it has drained.
 */
describe('a client that falls behind', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  /** A socket whose queue is whatever the test says it is. */
  function slowWs(): import('ws').WebSocket & { bufferedAmount: number } {
    const ws = mockWs() as unknown as import('ws').WebSocket & { bufferedAmount: number }
    ws.bufferedAmount = 0
    return ws
  }
  const output = (id: string, seq: number) => ({ id, data: `${id}${seq}`, seq })
  /** What the socket was sent, as `method id` or `data <text>`. */
  const said = (ws: import('ws').WebSocket): string[] =>
    (ws.send as unknown as ReturnType<typeof vi.fn>).mock.calls.map((c) => {
      const msg = JSON.parse(c[0] as string)
      return msg.method === 'terminal:data'
        ? `data ${msg.params.data}`
        : `${msg.method} ${msg.params.id}`
    })

  function behind() {
    const reg = new ClientRegistry()
    const ws = slowWs()
    reg.add(ws)
    reg.setTopics(ws, undefined, undefined, true)
    return { reg, ws }
  }

  it('withholds output past the high mark, and only terminal output', () => {
    const { reg, ws } = behind()
    reg.broadcast('terminal:data', output('a', 1), 'a')
    ws.bufferedAmount = RESYNC_HIGH_WATER + 1
    reg.broadcast('terminal:data', output('a', 2), 'a')
    reg.broadcast('session:updated', { id: 'a' })

    expect(said(ws)).toEqual(['data a1', 'session:updated a'])
  })

  it('keeps withholding until the queue has drained well below the mark', () => {
    // Between the marks it stays behind: resuming at the high mark would flap
    // on every flush, each one costing the client a whole re-attach.
    const { reg, ws } = behind()
    ws.bufferedAmount = RESYNC_HIGH_WATER + 1
    reg.broadcast('terminal:data', output('a', 1), 'a')
    ws.bufferedAmount = RESYNC_LOW_WATER + 1
    reg.broadcast('terminal:data', output('a', 2), 'a')
    ws.bufferedAmount = RESYNC_LOW_WATER
    reg.broadcast('terminal:data', output('a', 3), 'a')

    // The resync goes first, so the client is holding when the flush arrives.
    expect(said(ws)).toEqual(['terminal:resync a', 'data a3'])
  })

  it('tells a client it caught up on a terminal that went quiet', () => {
    vi.useFakeTimers()
    const { reg, ws } = behind()
    ws.bufferedAmount = RESYNC_HIGH_WATER + 1
    reg.broadcast('terminal:data', output('a', 1), 'a')
    reg.broadcast('terminal:data', output('b', 1), 'b')
    vi.advanceTimersByTime(100)
    expect(said(ws)).toEqual([])

    ws.bufferedAmount = 0
    vi.advanceTimersByTime(100)
    expect(said(ws)).toEqual(['terminal:resync a', 'terminal:resync b'])
    // Once, not on every tick after.
    vi.advanceTimersByTime(100)
    expect(said(ws)).toHaveLength(2)
  })

  it('does not hold back other clients', () => {
    const { reg, ws } = behind()
    const fast = slowWs()
    reg.add(fast)
    reg.setTopics(fast, undefined, undefined, true)
    ws.bufferedAmount = RESYNC_HIGH_WATER + 1
    reg.broadcast('terminal:data', output('a', 1), 'a')

    expect(said(ws)).toEqual([])
    expect(said(fast)).toEqual(['data a1'])
  })

  it('never withholds from a client that did not promise to re-attach', () => {
    // Its screen would stay wrong for good: nothing would ever tell it to ask again.
    const reg = new ClientRegistry()
    const ws = slowWs()
    reg.add(ws)
    ws.bufferedAmount = RESYNC_HIGH_WATER * 8
    reg.broadcast('terminal:data', output('a', 1), 'a')

    expect(said(ws)).toEqual(['data a1'])
  })

  it('owes a client that withdraws the promise what it missed, at once', () => {
    const { reg, ws } = behind()
    ws.bufferedAmount = RESYNC_HIGH_WATER + 1
    reg.broadcast('terminal:data', output('a', 1), 'a')
    reg.setTopics(ws, undefined, undefined, false)
    reg.broadcast('terminal:data', output('a', 2), 'a')

    expect(said(ws)).toEqual(['terminal:resync a', 'data a2'])
  })
})
