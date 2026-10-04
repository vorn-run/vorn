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

/** What is still held, without the taken chunks waiting to be dropped. */
function stillHeld(h: HeldOutput): { chunks: string[]; units: number } {
  return { chunks: h.chunks.slice(h.head), units: h.units }
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
    expect(stillHeld(h)).toEqual({ chunks: [], units: 0 })
  })

  it('takes whole chunks, then the front of the next, and keeps the rest in order', () => {
    const h = held('abc', 'defgh', 'ij')
    expect(takeOutput(h, 5)).toBe('abcde')
    expect(stillHeld(h)).toEqual({ chunks: ['fgh', 'ij'], units: 5 })
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

  it('takes a backlog of many small reads in time linear in the reads', () => {
    const reads = 1_000_000
    let h: HeldOutput | undefined
    for (let i = 0; i < reads; i++) h = holdOutput(h, 'ab')
    let out = 0
    const start = performance.now()
    while (h!.units > 0) out += takeOutput(h!, 200).length
    expect(out).toBe(2 * reads)
    // Copying what is left on every take would be 10 K takes x 500 K reads.
    expect(performance.now() - start).toBeLessThan(2000)
    expect(h!.chunks.length - h!.head).toBe(0)
  })

  it('caps a flush at 64 KB of output', () => {
    expect(MAX_FLUSH_UNITS).toBe(65_536)
    const h = held('x'.repeat(200_000))
    expect(takeOutput(h, MAX_FLUSH_UNITS)).toHaveLength(MAX_FLUSH_UNITS)
    expect(h.units).toBe(200_000 - MAX_FLUSH_UNITS)
  })
})
