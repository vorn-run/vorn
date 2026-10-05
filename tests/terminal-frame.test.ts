import { describe, it, expect } from 'vitest'
import {
  decodeTerminalFrame,
  decodeTerminalFrameV2,
  encodeTerminalFrame,
  encodeTerminalFrameV2,
  frameResume,
  terminalFrameVersion,
  MAX_FRAME_ID_BYTES
} from '../packages/shared/src/terminal-frame'

// The output must come back untouched, and bytes that are not a frame must be refused.

const bytes = (...values: number[]): Uint8Array => Uint8Array.from(values)

describe('a terminal frame', () => {
  it('carries the output back exactly, escapes and all', () => {
    const data = new TextEncoder().encode('\x1b[32mok\x1b[0m\r\n\x07')
    const frame = decodeTerminalFrame(encodeTerminalFrame({ id: 'abc-123', seq: 42, data }))

    expect(frame).toEqual({ id: 'abc-123', seq: 42, data })
  })

  it('does not care whether the bytes are valid text', () => {
    // A flush can end halfway through an emoji; the emulator reassembles it.
    const data = bytes(0xf0, 0x9f)
    const frame = decodeTerminalFrame(encodeTerminalFrame({ id: 't', seq: 1, data }))

    expect(frame?.data).toEqual(data)
  })

  it('keeps the whole range of sequence numbers', () => {
    const frame = decodeTerminalFrame(
      encodeTerminalFrame({ id: 't', seq: 0xfffffffe, data: new Uint8Array() })
    )

    expect(frame?.seq).toBe(0xfffffffe)
  })

  it('views the output when the buffer is exactly the frame', () => {
    // A browser socket hands over one ArrayBuffer per message; copying that would be for nothing.
    const wire = encodeTerminalFrame({ id: 't', seq: 1, data: bytes(1, 2, 3) })
    const frame = decodeTerminalFrame(wire)

    expect(frame?.data.buffer).toBe(wire.buffer)
    expect(frame?.data.byteLength).toBe(3)
  })

  it('reads a frame that sits inside a larger buffer', () => {
    const wire = encodeTerminalFrame({ id: 'xyz', seq: 7, data: bytes(9) })
    const pooled = new Uint8Array(wire.length + 8)
    pooled.set(wire, 4)

    const frame = decodeTerminalFrame(pooled.subarray(4, 4 + wire.length))

    expect(frame).toEqual({ id: 'xyz', seq: 7, data: bytes(9) })
    // Copied out: a view would cross to the renderer with the whole pool attached.
    expect(frame?.data.buffer).not.toBe(pooled.buffer)
  })

  it('refuses bytes that are not a frame', () => {
    expect(decodeTerminalFrame(new Uint8Array())).toBeNull()
    expect(decodeTerminalFrame(bytes(2, 0, 0, 0, 1, 1, 65))).toBeNull()
    // Header says three bytes of id; only one follows.
    expect(decodeTerminalFrame(bytes(1, 0, 0, 0, 1, 3, 65))).toBeNull()
    expect(decodeTerminalFrame(bytes(1, 0, 0, 0, 1, 0))).toBeNull()
  })

  it('refuses an id the header cannot describe', () => {
    expect(() =>
      encodeTerminalFrame({
        id: 'x'.repeat(MAX_FRAME_ID_BYTES + 1),
        seq: 1,
        data: new Uint8Array()
      })
    ).toThrow(/id/)
    expect(() => encodeTerminalFrame({ id: '', seq: 1, data: new Uint8Array() })).toThrow(/id/)
  })
})

// Version 2 names the records a frame holds, so a client always knows where its screen ends.
describe('a version 2 terminal frame', () => {
  const v2 = {
    id: 'abc-123',
    epoch: 3,
    firstRseq: 2 ** 40 + 7,
    lastRseq: 2 ** 40 + 9,
    startOffset: 2 ** 52 + 100,
    data: new TextEncoder().encode('\u001b[31mhi\u001b[0m')
  }

  it('carries the records and offsets back exactly, past 32 bits', () => {
    const frame = decodeTerminalFrameV2(encodeTerminalFrameV2(v2))
    expect(frame).toEqual(v2)
    expect(frameResume(frame!)).toEqual({
      epoch: 3,
      nextRseq: 2 ** 40 + 10,
      nextOffset: 2 ** 52 + 100 + v2.data.length
    })
  })

  it('reads the bytes vornd writes', () => {
    // vorn-term-proto's encoding of {s, epoch 1, records 2..=3, offset 5, "ab"}.
    const wire = bytes(
      2,
      1,
      0x73,
      0,
      0,
      0,
      1,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      2,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      3,
      0,
      0,
      0,
      0,
      0,
      0,
      0,
      5,
      0x61,
      0x62
    )
    expect(decodeTerminalFrameV2(wire)).toEqual({
      id: 's',
      epoch: 1,
      firstRseq: 2,
      lastRseq: 3,
      startOffset: 5,
      data: bytes(0x61, 0x62)
    })
  })

  it('leaves version 1 to its own reader, and each refuses the other', () => {
    const one = encodeTerminalFrame({ id: 't', seq: 1, data: bytes(1) })
    const two = encodeTerminalFrameV2(v2)
    expect(terminalFrameVersion(one)).toBe(1)
    expect(terminalFrameVersion(two)).toBe(2)
    expect(decodeTerminalFrameV2(one)).toBeNull()
    expect(decodeTerminalFrame(two)).toBeNull()
  })

  it('refuses a frame that is cut short or ends before it starts', () => {
    const wire = encodeTerminalFrameV2(v2)
    for (let n = 0; n < wire.length - v2.data.length; n++) {
      expect(decodeTerminalFrameV2(wire.subarray(0, n))).toBeNull()
    }
    expect(
      decodeTerminalFrameV2(encodeTerminalFrameV2({ ...v2, lastRseq: 1, firstRseq: 2 }))
    ).toBeNull()
  })
})
