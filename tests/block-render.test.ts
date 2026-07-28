// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest'
import { Terminal } from '@xterm/xterm'
import {
  blockToText,
  extractBlock,
  type BufferLike,
  type LineLike
} from '../src/renderer/lib/block-render'

/**
 * Drives a real xterm so the extraction is verified against actual VT
 * parsing, not a hand-built fake. A fake buffer would only prove the walker
 * agrees with my assumptions about xterm, which is the thing in question.
 */

// jsdom has no matchMedia, which xterm's browser services require on open().
if (typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {}
    })
  })
}

let term: Terminal | null = null

function open(cols = 40, rows = 10): Terminal {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const t = new Terminal({ cols, rows, allowProposedApi: true })
  t.open(el)
  term = t
  return t
}

/** xterm parses asynchronously; write() resolves once the data is applied. */
function write(t: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve))
}

function bufferOf(t: Terminal): BufferLike {
  return {
    getLine: (y) => t.buffer.active.getLine(y) as unknown as LineLike | undefined
  }
}

afterEach(() => {
  term?.dispose()
  term = null
  document.body.innerHTML = ''
})

describe('extractBlock against a real terminal', () => {
  it('reads plain text back out', async () => {
    const t = open()
    await write(t, 'hello world')
    const rows = extractBlock(bufferOf(t), 0, 0)
    expect(blockToText(rows)).toBe('hello world')
  })

  it('drops the grid padding that follows the content', async () => {
    // Every row is `cols` wide in the buffer; the blanks are not content.
    const t = open(40)
    await write(t, 'hi')
    const rows = extractBlock(bufferOf(t), 0, 0)
    expect(rows[0].runs.map((r) => r.text).join('')).toBe('hi')
  })

  it('splits a row into runs at style boundaries', async () => {
    const t = open()
    // bold "BOLD", reset, plain " plain"
    await write(t, '\x1b[1mBOLD\x1b[0m plain')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    expect(row.runs).toHaveLength(2)
    expect(row.runs[0]).toMatchObject({ text: 'BOLD', bold: true })
    expect(row.runs[1]).toMatchObject({ text: ' plain', bold: false })
  })

  it('carries the attributes a block needs to look right', async () => {
    const t = open()
    await write(t, '\x1b[3mit\x1b[0m\x1b[4mun\x1b[0m\x1b[2mdim\x1b[0m\x1b[7minv\x1b[0m')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    const byText = Object.fromEntries(row.runs.map((r) => [r.text, r]))
    expect(byText['it'].italic).toBe(true)
    expect(byText['un'].underline).toBe(true)
    expect(byText['dim'].dim).toBe(true)
    expect(byText['inv'].inverse).toBe(true)
  })

  it('distinguishes palette from truecolor', async () => {
    const t = open()
    // 31 = palette red, then a 24-bit colour
    await write(t, '\x1b[31mpal\x1b[0m\x1b[38;2;10;20;30mrgb\x1b[0m')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    const pal = row.runs.find((r) => r.text === 'pal')!
    const rgb = row.runs.find((r) => r.text === 'rgb')!
    expect(pal.fg).toEqual({ kind: 'palette', index: 1 })
    expect(rgb.fg).toEqual({ kind: 'rgb', value: (10 << 16) | (20 << 8) | 30 })
  })

  it('keeps a background band rather than trimming it as padding', async () => {
    const t = open(20)
    // A trailing run with a background is visible; only unstyled blanks go.
    await write(t, 'x\x1b[41m   \x1b[0m')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    const last = row.runs[row.runs.length - 1]
    expect(last.bg).toEqual({ kind: 'palette', index: 1 })
    expect(last.text).toBe('   ')
  })

  it('emits one glyph for a wide character, not two', async () => {
    const t = open()
    await write(t, '日本語')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    expect(blockToText([row])).toBe('日本語')
  })

  it('reads a multi-row block spanning command and output', async () => {
    const t = open()
    await write(t, 'ls\r\nLICENSE\r\nREADME.md\r\n')
    const rows = extractBlock(bufferOf(t), 0, 2)
    expect(blockToText(rows)).toBe('ls\nLICENSE\nREADME.md')
  })

  it('preserves the styling a coloured tool emits', async () => {
    const t = open()
    // The shape `git status --short` or a test runner produces.
    await write(t, ' \x1b[32m✓\x1b[0m tests/spine.test.ts \x1b[90m(7)\x1b[0m')
    const [row] = extractBlock(bufferOf(t), 0, 0)
    const tick = row.runs.find((r) => r.text.includes('✓'))!
    expect(tick.fg).toEqual({ kind: 'palette', index: 2 })
    expect(blockToText([row])).toBe(' ✓ tests/spine.test.ts (7)')
  })
})
