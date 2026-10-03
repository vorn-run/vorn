import { describe, it, expect } from 'vitest'
import {
  holdOutput,
  takeOutput,
  MAX_FLUSH_UNITS,
  type HeldOutput
} from '../packages/server/src/output-buffer'

/**
 * The flush buffer between node-pty's reads and the 64 KB flushes that send
 * them. Everything here is about taking a front of the right size without
 * losing, repeating or splitting anything.
 */

function held(...chunks: string[]): HeldOutput {
  let h: HeldOutput | undefined
  for (const c of chunks) h = holdOutput(h, c)
  return h!
}

/** Take until empty, the way a draining flush does. */
function drain(h: HeldOutput, cap: number): string[] {
  const out: string[] = []
  while (h.units > 0) out.push(takeOutput(h, cap))
  return out
}

describe('takeOutput', () => {
  it('takes everything when it fits, leaving nothing held', () => {
    const h = held('ab', 'cd')
    expect(takeOutput(h, 10)).toBe('abcd')
    expect(h).toEqual({ chunks: [], units: 0 })
  })

  it('takes whole chunks, then the front of the next, and keeps the rest in order', () => {
    const h = held('abc', 'defgh', 'ij')
    expect(takeOutput(h, 5)).toBe('abcde')
    expect(h).toEqual({ chunks: ['fgh', 'ij'], units: 5 })
    expect(takeOutput(h, 5)).toBe('fghij')
  })

  it('loses and repeats nothing however the cap falls', () => {
    const chunks = ['héllo ', 'wörld ', '😀😀', ' and ', 'x'.repeat(40)]
    for (let cap = 1; cap <= 20; cap++) {
      const pieces = drain(held(...chunks), cap)
      expect(pieces.join(''), `cap ${cap}`).toBe(chunks.join(''))
    }
  })

  it('never ends a flush between the two halves of a surrogate pair', () => {
    for (let cap = 1; cap <= 9; cap++) {
      for (const piece of drain(held('ab😀cd😀😀', '😀e'), cap)) {
        expect(/[\uD800-\uDBFF]$/.test(piece), `cap ${cap}: ${JSON.stringify(piece)}`).toBe(false)
        expect(/^[\uDC00-\uDFFF]/.test(piece), `cap ${cap}: ${JSON.stringify(piece)}`).toBe(false)
      }
    }
  })

  it('moves a pair whole when the cap cannot hold it, rather than stalling', () => {
    const h = held('😀x')
    expect(takeOutput(h, 1)).toBe('😀')
    expect(takeOutput(h, 1)).toBe('x')
  })

  it('caps a flush at 64 KB of output', () => {
    expect(MAX_FLUSH_UNITS).toBe(65_536)
    const h = held('x'.repeat(200_000))
    expect(takeOutput(h, MAX_FLUSH_UNITS)).toHaveLength(MAX_FLUSH_UNITS)
    expect(h.units).toBe(200_000 - MAX_FLUSH_UNITS)
  })
})
