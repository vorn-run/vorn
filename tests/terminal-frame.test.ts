import { describe, it, expect } from 'vitest'
import {
  decodeTerminalFrame,
  encodeTerminalFrame,
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
