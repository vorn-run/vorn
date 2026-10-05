import { describe, it, expect } from 'vitest'
import { Terminal as Headless } from '@xterm/headless'
import type { Terminal } from '@xterm/xterm'
import { resizeInOrder } from '../src/renderer/lib/stream-resize'

/**
 * A resize from a session's record log lands between the output written for
 * the old size and the output after it, as the record does. Real xterm.js,
 * whose `write` parses later and whose `resize` does not.
 */

const OLD = 'a'.repeat(70) + '\r\n' + 'b'.repeat(70)
const NEW = '\r\n' + 'c'.repeat(45)

function lines(term: Headless): string[] {
  const buf = term.buffer.active
  const out: string[] = []
  for (let y = 0; y < buf.length; y++) out.push(buf.getLine(y)?.translateToString(true) ?? '')
  while (out.length && out[out.length - 1] === '') out.pop()
  return out
}

/** Written, parsed, resized, then the rest: what the record log means. */
async function reference(): Promise<string[]> {
  const term = new Headless({ cols: 80, rows: 10, allowProposedApi: true })
  await new Promise<void>((r) => term.write(OLD, r))
  term.resize(40, 10)
  await new Promise<void>((r) => term.write(NEW, r))
  return lines(term)
}

describe('a resize from the byte stream', () => {
  it('applies after the output before it and before the output after it', async () => {
    const term = new Headless({ cols: 80, rows: 10, allowProposedApi: true })
    term.write(OLD)
    let sized = false
    resizeInOrder(term as unknown as Terminal, 40, 10, () => (sized = true))
    await new Promise<void>((r) => term.write(NEW, r))
    expect(sized).toBe(true)
    expect(lines(term)).toEqual(await reference())
  })

  it('is what a resize at once gets wrong', async () => {
    const term = new Headless({ cols: 80, rows: 10, allowProposedApi: true })
    term.write(OLD)
    term.resize(40, 10)
    await new Promise<void>((r) => term.write(NEW, r))
    expect(lines(term)).not.toEqual(await reference())
  })
})
