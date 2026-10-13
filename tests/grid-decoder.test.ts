import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { CLOSE_CREDENTIAL_REJECTED } from '@vornrun/shared/protocol'
import { CborWriter, decodeCbor } from '../packages/web/src/grid/cbor'
import {
  CAPS,
  ClientKind,
  FrameReader,
  KIND,
  ProtocolError,
  decodeRow,
  decodeServer,
  encodeClient,
  type ClientMsg,
  type ServerMsg
} from '../packages/web/src/grid/protocol'
import { GridMirror } from '../packages/web/src/grid/mirror'
import { GridConnection, type GridSocket } from '../packages/web/src/grid/client'

/**
 * The web client's grid-mode decoder against frames a real vornd sent over
 * `/grid` (recorded by packages/core/crates/vornd/tests/web_grid.rs with
 * VORN_RECORD_GRID_FRAMES): a shell attached at 80x24, `echo grid-$((6*7))`
 * typed, then the view resized to 100x30 and its size taken.
 */

type Fixture = { sent: string[]; received: string[] }
const fixturePath = join(__dirname, 'fixtures', 'grid-frames.json')
const fixture: Fixture = JSON.parse(readFileSync(fixturePath, 'utf8'))

const hex = (h: string): Uint8Array => Uint8Array.from(Buffer.from(h, 'hex'))
const sent = fixture.sent.map(hex)
const received = fixture.received.map(hex)

function readAll(messages: Uint8Array[]): ServerMsg[] {
  const reader = new FrameReader()
  const out: ServerMsg[] = []
  for (const m of messages) {
    reader.push(m)
    for (let f = reader.next(); f; f = reader.next()) {
      const msg = decodeServer(f.kind, f.payload)
      if (msg) out.push(msg)
    }
  }
  return out
}

function frames(messages: Uint8Array[]): Uint8Array[] {
  const all = Buffer.concat(messages)
  const out: Uint8Array[] = []
  for (let at = 0; at < all.length; ) {
    const len = all.readUInt32LE(at)
    out.push(Uint8Array.from(all.subarray(at, at + 4 + len)))
    at += 4 + len
  }
  return out
}

const msgs = readAll(received)
const of = <T extends ServerMsg['type']>(t: T) =>
  msgs.filter((m): m is Extract<ServerMsg, { type: T }> => m.type === t)

describe('grid frames from vornd', () => {
  it('open with Welcome, then Attached and a Snapshot of the 80x24 screen', () => {
    expect(msgs[0]).toMatchObject({ type: 'welcome', major: 1, caps: CAPS.ALL })
    const attached = of('attached')
    expect(attached).toHaveLength(1)
    expect(attached[0].session).toMatch(/\S/)
    const [snap] = of('snapshot')
    expect(snap.sid).toBe(attached[0].sid)
    expect(snap.snap.rowFmt).toBe(1)
    expect(snap.snap.term).toMatchObject({ cols: 80, rows: 24, screen: 0 })
  })

  it('carry the input ack, the deltas and the resize', () => {
    expect(of('inputAck').map((m) => m.inputSeq)).toEqual([1])
    expect(of('delta').length).toBeGreaterThan(0)
    expect(of('event').length).toBeGreaterThan(0)
    const resized = of('resized')
    expect(resized.at(-1)).toMatchObject({ cols: 100, rows: 30 })
  })

  it('build a mirror that shows the command output, then resizes it', () => {
    const [snap] = of('snapshot')
    const mirror = new GridMirror(snap.snap)
    for (const d of of('delta')) expect(mirror.apply(d.delta)).toBeNull()
    expect(mirror.text()).toContain('grid-42')
    expect(mirror.term.cols).toBe(100)
    expect(mirror.rows).toHaveLength(30)
  })

  it('refuse a delta whose base is not the mirror revision', () => {
    const [snap] = of('snapshot')
    const mirror = new GridMirror(snap.snap)
    const deltas = of('delta')
    expect(mirror.apply(deltas[deltas.length - 1].delta)).toBe('base')
  })

  it('decode the same however the socket splits them', () => {
    const all = Buffer.concat(received)
    const byByte = Array.from(all, (b) => Uint8Array.of(b))
    expect(readAll(byByte)).toEqual(msgs)
  })

  it('skip an optional kind and refuse an unknown one', () => {
    expect(decodeServer(KIND.OPTIONAL | 0x40, Uint8Array.of(0xbf, 0xff))).toBeNull()
    expect(() => decodeServer(0x40, Uint8Array.of(0xbf, 0xff))).toThrow(ProtocolError)
    const reader = new FrameReader()
    reader.push(Uint8Array.of(0, 0, 0, 0))
    expect(() => reader.next()).toThrow(ProtocolError)
  })
})

describe('grid frames to vornd', () => {
  it('encode byte for byte what the native client sent', () => {
    const out = frames(sent)
    const hello = out[0]
    const [attached] = of('attached')
    const [snap] = of('snapshot')
    const deltas = of('delta')
    const expected: ClientMsg[] = [
      {
        type: 'hello',
        major: 1,
        minor: 0,
        caps: CAPS.ALL,
        client: ClientKind.Web,
        build: 'web-test'
      },
      {
        type: 'attach',
        session: attached.session,
        view: { cols: 80, rows: 24 },
        visible: true,
        historyTail: 0
      },
      { type: 'ack', sid: attached.sid, rev: snap.snap.rev },
      {
        type: 'input',
        sid: attached.sid,
        inputSeq: 1,
        event: { type: 'text', utf8: 'echo grid-$((6*7))\r' }
      }
    ]
    for (const [i, m] of expected.entries()) expect(encodeClient(m)).toEqual(out[i])
    expect(hello[4]).toBe(KIND.HELLO)

    const rest = out.slice(expected.length)
    const sid = attached.sid
    const acks = deltas.map((d) => encodeClient({ type: 'ack', sid, rev: d.delta.rev }))
    const viewport = encodeClient({ type: 'viewport', sid, size: { cols: 100, rows: 30 } })
    const take = encodeClient({ type: 'takeSize', sid })
    const same = (a: Uint8Array) => (b: Uint8Array) => Buffer.from(a).equals(Buffer.from(b))
    for (const f of rest) expect([...acks, viewport, take].some(same(f))).toBe(true)
    expect(rest.some(same(viewport))).toBe(true)
    expect(rest.some(same(take))).toBe(true)
  })

  it('round-trip every input event through the CBOR reader', () => {
    const events = [
      {
        type: 'key',
        action: 'press',
        key: 30,
        mods: 1,
        consumedMods: 0,
        text: 'A',
        unshifted: 97,
        composing: false
      },
      { type: 'paste', utf8: 'π', confirmed: true },
      { type: 'mouse', action: 'motion', mods: 0, x: 1.5, y: 2.25 },
      { type: 'wheel', dx: 0, dy: -3, mods: 0, x: 4, y: 5 },
      { type: 'focus', focused: true },
      { type: 'raw', bytes: Uint8Array.of(0x1b, 0x5b) }
    ] as const
    for (const event of events) {
      const frame = encodeClient({ type: 'input', sid: 300, inputSeq: 2 ** 40, event })
      expect(frame[4]).toBe(KIND.INPUT)
      const v = decodeCbor(frame.subarray(5)) as Map<number, unknown>
      expect(v.get(1)).toBe(300)
      expect(v.get(2)).toBe(2 ** 40)
    }
  })
})

describe('row cells', () => {
  it('read ascii, empty and wide cells', () => {
    // An ascii run "a", empty, "b"; then a cluster run of one wide cell.
    const wide = new TextEncoder().encode('界')
    const row = Uint8Array.of(0, 0, 6, 0x61, 0, 0x62, 1, 0, 3, 0x40 | wide.length, ...wide)
    const runs = decodeRow(row)
    expect(runs.map((r) => r.style)).toEqual([0, 1])
    expect(runs[0].cells.map((c) => c.text)).toEqual(['a', '', 'b'])
    expect(runs[1].cells).toEqual([{ text: '界', wide: true }])
  })

  it('refuse what vornd would', () => {
    expect(() => decodeRow(Uint8Array.of(0, 0, 4, 0x61))).toThrow(ProtocolError)
    expect(() => decodeRow(Uint8Array.of(0, 0, 2, 0x07))).toThrow(ProtocolError)
    expect(() => decodeRow(Uint8Array.of(0, 0, 3, 0x80))).toThrow(ProtocolError)
    expect(() => decodeRow(Uint8Array.of(0, 0, 3, 2, 0xff, 0xfe))).toThrow(ProtocolError)
    expect(() => decodeRow(new Uint8Array(11).fill(0x80))).toThrow(ProtocolError)
  })
})

describe('CBOR', () => {
  it('reads definite maps, negatives, floats and u64 past 2^53', () => {
    const big = Uint8Array.of(0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff)
    expect(decodeCbor(big)).toBe(2n ** 64n - 1n)
    expect(decodeCbor(Uint8Array.of(0xa1, 0x01, 0x20))).toEqual(new Map([[1, { neg: 0 }]]))
    expect(decodeCbor(Uint8Array.of(0xf9, 0x3c, 0x00))).toEqual({ float: 1 })
    expect(decodeCbor(Uint8Array.of(0x9f, 0x01, 0xff))).toEqual([1])
    expect(() => decodeCbor(Uint8Array.of(0x5a, 0xff, 0xff, 0xff, 0xff))).toThrow()
  })

  it('writes minimal heads', () => {
    const w = new CborWriter().uint(23).uint(24).uint(256).uint(65536).uint(2n ** 32n)
    expect(Buffer.from(w.finish()).toString('hex')).toBe(
      '17' + '1818' + '190100' + '1a00010000' + '1b0000000100000000'
    )
  })
})

class FakeSocket implements GridSocket {
  binaryType = 'blob'
  readyState = 1
  sent: (string | Uint8Array)[] = []
  closedWith: number | undefined
  onopen: ((ev: unknown) => void) | null = null
  onmessage: ((ev: { data: unknown }) => void) | null = null
  onclose: ((ev: { code: number; reason: string }) => void) | null = null
  onerror: ((ev: unknown) => void) | null = null
  url: string

  constructor(url: string) {
    this.url = url
  }
  send(data: string | Uint8Array): void {
    this.sent.push(data)
  }
  close(code?: number): void {
    this.closedWith = code
  }
  binary(bytes: Uint8Array): void {
    this.onmessage?.({ data: bytes.slice().buffer })
  }
}

describe('GridConnection', () => {
  type Opened = { conn: GridConnection; ws: FakeSocket; got: ServerMsg[]; auth: string[] }

  function open(): Opened {
    let ws!: FakeSocket
    const got: ServerMsg[] = []
    const auth: string[] = []
    const conn = new GridConnection('ws://host/grid', {
      token: 'device-token',
      build: 'web-test',
      onMessage: (m) => got.push(m),
      onAuthRequired: () => auth.push('asked'),
      openSocket: (url) => (ws = new FakeSocket(url))
    })
    return { conn, ws, got, auth }
  }

  it('authenticates first, says hello on auth:ok, and acks every frame', () => {
    const { conn, ws, got } = open()
    expect(ws.binaryType).toBe('arraybuffer')
    ws.onopen?.({})
    expect(JSON.parse(ws.sent[0] as string)).toMatchObject({
      method: 'auth:authenticate',
      params: { token: 'device-token' }
    })
    expect(conn.ready).toBe(false)
    expect(() => conn.takeSize(1)).toThrow()

    ws.onmessage?.({ data: JSON.stringify({ jsonrpc: '2.0', method: 'auth:ok', params: {} }) })
    expect(conn.ready).toBe(true)
    expect(ws.sent[1]).toEqual(frames(sent)[0])

    for (const m of received) ws.binary(m)
    expect(got).toEqual(msgs)
    const acks = ws.sent.slice(2).map((f) => (f as Uint8Array)[4])
    expect(acks).toEqual(Array(of('snapshot').length + of('delta').length).fill(KIND.ACK))

    expect(conn.input(1, { type: 'text', utf8: 'x' })).toBe(1)
    expect(conn.input(1, { type: 'text', utf8: 'y' })).toBe(2)
  })

  it('asks for a token when vornd refuses this one', () => {
    const { conn, ws, auth } = open()
    ws.onclose?.({ code: CLOSE_CREDENTIAL_REJECTED, reason: '' })
    expect(auth).toEqual(['asked'])
    expect(conn.ready).toBe(false)
  })

  it('closes on a broken frame', () => {
    const { ws } = open()
    ws.onopen?.({})
    ws.onmessage?.({ data: JSON.stringify({ method: 'auth:ok' }) })
    ws.binary(Uint8Array.of(0, 0, 0, 0))
    expect(ws.closedWith).toBe(1002)
  })
})
