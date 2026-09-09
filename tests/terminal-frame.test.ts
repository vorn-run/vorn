import { describe, it, expect } from 'vitest'
import {
  decodeTerminalFrame,
  encodeTerminalFrame,
  MAX_FRAME_ID_BYTES
} from '../packages/shared/src/terminal-frame'

/**
 * Terminal bytes on the wire.
 *
 * What matters is that the output comes back untouched -- escapes, NUL, a
 * multibyte sequence cut in half by the flush -- and that the decoder refuses
 * bytes that are not a frame rather than reading garbage out of them.
 */

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

  it('copies the output out of the buffer it arrived in', () => {
    const wire = encodeTerminalFrame({ id: 't', seq: 1, data: bytes(1, 2, 3) })
    const frame = decodeTerminalFrame(wire)

    // A view would clone the whole socket buffer when it crosses to the renderer.
    expect(frame?.data.buffer).not.toBe(wire.buffer)
    expect(frame?.data.byteLength).toBe(3)
  })

  it('reads a frame that sits inside a larger buffer', () => {
    const wire = encodeTerminalFrame({ id: 'xyz', seq: 7, data: bytes(9) })
    const pooled = new Uint8Array(wire.length + 8)
    pooled.set(wire, 4)

    expect(decodeTerminalFrame(pooled.subarray(4, 4 + wire.length))).toEqual({
      id: 'xyz',
      seq: 7,
      data: bytes(9)
    })
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
