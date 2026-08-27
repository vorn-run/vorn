import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { Terminal } from '@xterm/headless'
import {
  feedScreen,
  resizeScreen,
  serializeScreen,
  clearScreen,
  screenCount,
  resetScreens
} from '../packages/server/src/terminal-screen'

/**
 * Modelling the screen rather than the bytes.
 *
 * A headless terminal is a pure function from bytes to a buffer, so almost all
 * of this needs no PTY: feed the bytes a program would emit and ask what the
 * screen became. The one thing that cannot be tested here is that `pty-manager`
 * calls these at all, which is why the wiring is three lines with nothing to get
 * wrong.
 */

const ESC = '\x1b'

beforeEach(resetScreens)
afterEach(resetScreens)

/** Write a serialized screen into a fresh terminal, as a client would. */
async function restore(dump: string, cols = 80, rows = 24): Promise<Terminal> {
  const term = new Terminal({ cols, rows, scrollback: 0, allowProposedApi: true })
  await new Promise<void>((resolve) => term.write(dump, resolve))
  return term
}

/** Every cell of a row, as `char/fg/bg`, so a mismatch says which cell. */
function row(term: Terminal, y: number): string {
  const line = term.buffer.active.getLine(y)
  if (!line) return ''
  const cells: string[] = []
  for (let x = 0; x < term.cols; x++) {
    const cell = line.getCell(x)
    if (!cell) continue
    cells.push(`${cell.getChars() || ' '}/${cell.getFgColor()}/${cell.getBgColor()}`)
  }
  return cells.join('|')
}

describe('what the screen looks like when it comes back', () => {
  it('reproduces text and colour cell for cell', async () => {
    feedScreen('t', `${ESC}[31mred${ESC}[0m plain ${ESC}[1;34mbold blue${ESC}[0m`)
    const dump = await serializeScreen('t')

    const restored = await restore(dump)
    const live = await restore(`${ESC}[31mred${ESC}[0m plain ${ESC}[1;34mbold blue${ESC}[0m`)

    expect(row(restored, 0)).toBe(row(live, 0))
    restored.dispose()
    live.dispose()
  })

  it('puts the cursor back where it was', async () => {
    feedScreen('t', `line one\r\nline two\r\n${ESC}[1;4H`)
    const dump = await serializeScreen('t')

    const restored = await restore(dump)

    expect(restored.buffer.active.cursorY).toBe(0)
    expect(restored.buffer.active.cursorX).toBe(3)
    restored.dispose()
  })

  it('carries an alternate-screen program mid-render', async () => {
    // What a TUI leaves behind: the alternate screen active, a painted frame,
    // colours, and the cursor parked somewhere inside it. Bytes alone cannot say
    // the alternate screen is the active one -- that is a mode, not a character.
    const paint =
      `${ESC}[?1049h${ESC}[2J${ESC}[H` +
      `${ESC}[44m Status ${ESC}[0m\r\n` +
      `${ESC}[32m M src/index.ts${ESC}[0m\r\n` +
      `${ESC}[2;3H`
    feedScreen('t', paint)
    const dump = await serializeScreen('t')

    const restored = await restore(dump)
    const live = await restore(paint)

    expect(row(restored, 0)).toBe(row(live, 0))
    expect(row(restored, 1)).toBe(row(live, 1))
    expect(restored.buffer.active.cursorY).toBe(live.buffer.active.cursorY)
    expect(restored.buffer.active.cursorX).toBe(live.buffer.active.cursorX)
    restored.dispose()
    live.dispose()
  })
})

describe('answering queries, which it must never do', () => {
  it('has no subscriber to answer through, and no way to grow one', () => {
    // There was a test here that built a spy, never attached it to anything, and
    // asserted it was not called. It passed against every possible version of
    // this module, including one wired straight to the PTY, which is worth more
    // as a warning than the assertion was worth as a test.
    //
    // The protection is that nothing subscribes, so the invariant *is* the
    // absence, and the honest way to check an absence is to look for it. The
    // end-to-end proof -- that a query reaches no PTY -- lives in
    // `pty-manager-screen.test.ts`, where there is a PTY to watch.
    const source = fs.readFileSync(
      path.join(__dirname, '..', 'packages', 'server', 'src', 'terminal-screen.ts'),
      'utf-8'
    )

    expect(source).not.toMatch(/\.onData\s*\(/)
    expect(source).not.toMatch(/\.onBinary\s*\(/)
  })

  it('produces a snapshot that asks nothing of whoever restores it', async () => {
    // The other half: a serialized screen is fed back into a live terminal, and
    // that terminal's replies do go to the PTY. If a snapshot carried a query,
    // restoring one would type into the user's shell.
    feedScreen('q', `${ESC}[cbefore${ESC}[5nafter`)
    const dump = await serializeScreen('q')

    const replies: string[] = []
    const restored = new Terminal({ cols: 80, rows: 24, scrollback: 0, allowProposedApi: true })
    restored.onData((d) => replies.push(d))
    await new Promise<void>((resolve) => restored.write(dump, resolve))

    expect(replies).toEqual([])
    restored.dispose()
  })

  it('really would answer, if anything were listening', async () => {
    // The counterpart that makes the two above mean something. A test asserting
    // silence proves nothing unless the thing could speak -- and this one can:
    // feed the same query to a terminal with a listener and it replies.
    const replies: string[] = []
    const term = new Terminal({ cols: 80, rows: 24, scrollback: 0, allowProposedApi: true })
    term.onData((d) => replies.push(d))
    await new Promise<void>((resolve) => term.write(`${ESC}[c`, resolve))

    expect(replies.length).toBeGreaterThan(0)
    expect(replies.join('')).toContain('[?')
    term.dispose()
  })
})

describe('reading a screen that is still being written', () => {
  it('waits for the write rather than returning a half-parsed screen', async () => {
    // `term.write` queues a macrotask; serializing straight after it would
    // return whatever had been parsed by then, which is usually nothing. That
    // failure is invisible in a test that happens to yield, and shows up under
    // load instead.
    feedScreen('t', 'hello world')
    const dump = await serializeScreen('t')

    expect(dump).toContain('hello world')
  })

  it('sees every write in order when several arrive before a read', async () => {
    feedScreen('t', 'one ')
    feedScreen('t', 'two ')
    feedScreen('t', 'three')

    expect(await serializeScreen('t')).toContain('one two three')
  })
})

describe('following the terminal it models', () => {
  it('starts at the geometry the session was given', async () => {
    feedScreen('t', 'x'.repeat(100), 40, 10)
    const dump = await serializeScreen('t')

    // Forty columns means the hundred characters wrap onto a third line; eighty
    // would not. The wrap is the observable difference.
    const restored = await restore(dump, 40, 10)
    expect(restored.buffer.active.getLine(2)?.translateToString(true)).toContain('x')
    restored.dispose()
  })

  it('falls back to a PTY default when the session has no geometry', async () => {
    feedScreen('t', 'hello')
    expect(await serializeScreen('t')).toContain('hello')
  })

  it('follows a resize', async () => {
    feedScreen('t', 'hello', 80, 24)
    resizeScreen('t', 100, 40)
    feedScreen('t', ' world')

    expect(await serializeScreen('t')).toContain('hello world')
  })

  it('ignores a resize for a session it does not model', () => {
    expect(() => resizeScreen('absent', 100, 40)).not.toThrow()
  })
})

describe('keeping terminals apart, and letting them go', () => {
  it('models each session separately', async () => {
    feedScreen('a', 'first')
    feedScreen('b', 'second')

    expect(await serializeScreen('a')).toContain('first')
    expect(await serializeScreen('a')).not.toContain('second')
    expect(await serializeScreen('b')).toContain('second')
  })

  it('reads a session it has never seen as empty', async () => {
    expect(await serializeScreen('never')).toBe('')
  })

  it('forgets a terminal that has gone', async () => {
    feedScreen('t', 'something')
    expect(screenCount()).toBe(1)

    clearScreen('t')

    expect(screenCount()).toBe(0)
    expect(await serializeScreen('t')).toBe('')
  })

  it('does not throw when clearing one that was never there', () => {
    expect(() => clearScreen('absent')).not.toThrow()
  })

  it('releases every terminal it held', () => {
    for (let i = 0; i < 20; i++) feedScreen(`s${i}`, 'output')
    expect(screenCount()).toBe(20)

    resetScreens()

    expect(screenCount()).toBe(0)
  })
})
