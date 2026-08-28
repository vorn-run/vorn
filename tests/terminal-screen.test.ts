import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { Terminal } from '@xterm/headless'
import {
  createScreen,
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

const started = new Set<string>()

beforeEach(() => {
  resetScreens()
  started.clear()
})
afterEach(resetScreens)

/**
 * Start a screen if this test has not already, then feed it.
 *
 * Production creates once, at the spawn, and feeds on every flush thereafter --
 * so the geometry is stated once rather than carried on every chunk. These read
 * the same way.
 */
function feed(id: string, data: string, cols = 80, rows = 24): void {
  if (!started.has(id)) {
    createScreen(id, cols, rows)
    started.add(id)
  }
  feedScreen(id, data)
}

/** Write a serialized screen into a fresh terminal, as a client would. */
async function restore(dump: string, cols = 80, rows = 24): Promise<Terminal> {
  const term = new Terminal({ cols, rows, scrollback: 0, allowProposedApi: true })
  await new Promise<void>((resolve) => term.write(dump, resolve))
  return term
}

/** Every cell of every row, so a mismatch says which cell of which row. */
function screenOf(term: Terminal): string[] {
  const rows: string[] = []
  for (let y = 0; y < term.rows; y++) rows.push(row(term, y))
  return rows
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

describe('a terminal that goes away while it is being read', () => {
  it.each([
    ['a snapshot', (id: string) => serializeScreen(id)],
    ['a resize', (id: string) => resizeScreen(id, 132, 43)]
  ])(
    'lets %s finish rather than leaving it pending for ever',
    async (_label, start) => {
      // Both read the buffer from inside xterm's write callback, which is the only
      // point that means what they need it to mean. That leaves a question with a
      // consequence: if `dispose()` dropped a pending callback, the promise would
      // never settle -- and since every disk operation for a session runs behind
      // the last, one unsettled promise would wedge that session's queue for the
      // life of the server, so its history would never be written and never be
      // removed.
      //
      // It does not: disposing does not cancel the parse xterm has already
      // scheduled, so the callback still runs. That is a fact about this pinned
      // version rather than a documented guarantee, which is exactly why it is
      // asserted here -- a version bump is where it would change.
      createScreen('going', 200, 50)
      for (let i = 0; i < 4_000; i++) feedScreen('going', `line ${i} of output\r\n`)

      const pending = start('going')
      clearScreen('going')

      const outcome = await Promise.race([
        pending.then(() => 'settled'),
        new Promise((resolve) => setTimeout(() => resolve('still pending'), 3_000))
      ])
      expect(outcome).toBe('settled')
    },
    10_000
  )
})

describe('where the shell says it is', () => {
  it("follows the sequence Vorn's own integration emits", async () => {
    // The handler this model started with listens on OSC 7, which is what other
    // terminals use. Vorn's shell integration has always emitted `5522;cwd;`
    // instead -- so the cwd was empty for every Vorn shell, and the checkpoint
    // has been carrying an empty field since it was written.
    createScreen('shell', 80, 24)
    feedScreen('shell', `${ESC}]5522;cwd;/Users/x/dev/vorn\x07`)

    expect((await serializeScreen('shell'))?.cwd).toBe('/Users/x/dev/vorn')
  })

  it('still follows the one other terminals use', async () => {
    createScreen('shell', 80, 24)
    feedScreen('shell', `${ESC}]7;file://host/Users/x/dev/other\x07`)

    expect((await serializeScreen('shell'))?.cwd).toBe('/Users/x/dev/other')
  })

  it('survives a directory with a percent in its name', async () => {
    // The path is not percent-encoded on this sequence, so it is taken as-is --
    // and a directory called `100%` is a directory somebody has.
    createScreen('shell', 80, 24)
    feedScreen('shell', `${ESC}]5522;cwd;/tmp/100%\x07`)

    expect((await serializeScreen('shell'))?.cwd).toBe('/tmp/100%')
  })

  it("ignores the integration's other reports", async () => {
    createScreen('shell', 80, 24)
    feedScreen('shell', `${ESC}]5522;cmd;bHM=\x07`)
    feedScreen('shell', `${ESC}]5522;dur;120\x07`)

    expect((await serializeScreen('shell'))?.cwd).toBe('')
  })

  it('keeps it bounded like the title', async () => {
    createScreen('shell', 80, 24)
    feedScreen('shell', `${ESC}]5522;cwd;/${'p'.repeat(5_000_000)}\x07`)

    expect(((await serializeScreen('shell'))?.cwd.length ?? 0) <= 512).toBe(true)
  })
})

describe('a screen rebuilt from a checkpoint', () => {
  it('is given back the title and directory the serializer cannot carry', async () => {
    // Neither is an escape sequence: they arrive as notifications and the
    // serializer does not round-trip them. The checkpoint stores both, and
    // recovery used to drop them on the floor.
    createScreen('restored', 100, 30, { title: 'vorn — building', cwd: '/Users/x/dev/vorn' })

    expect(await serializeScreen('restored')).toMatchObject({
      title: 'vorn — building',
      cwd: '/Users/x/dev/vorn'
    })
  })
})

describe('what a program can make the model hold', () => {
  it('keeps a title bounded, whatever length the program sends', async () => {
    // Both the title and the cwd live for as long as the terminal does and both
    // travel in the checkpoint, and xterm will hand over an OSC payload of ten
    // million characters. One long enough puts every checkpoint for that session
    // over its size cap -- permanently, since the field never goes back. An
    // agent printing a file it was asked to read can produce this without
    // anybody intending it.
    createScreen('shouty', 80, 24)
    feedScreen('shouty', `${ESC}]0;${'t'.repeat(5_000_000)}\x07`)

    const snapshot = await serializeScreen('shouty')

    expect(snapshot?.title.length ?? 0).toBeLessThanOrEqual(512)
    expect(snapshot?.title.startsWith('tt')).toBe(true)
  })

  it('keeps a cwd bounded the same way', async () => {
    createScreen('shouty', 80, 24)
    feedScreen('shouty', `${ESC}]7;file:///${'p'.repeat(5_000_000)}\x07`)

    expect((await serializeScreen('shouty'))?.cwd.length ?? 0).toBeLessThanOrEqual(512)
  })
})

describe('the moment a snapshot describes', () => {
  it('holds what was written before it was asked for, and nothing written after', async () => {
    // The whole correctness argument for the checkpoint rests on this. xterm
    // invokes a write's callback from inside its own parse loop and then keeps
    // consuming the queue for up to twelve milliseconds before returning, so
    // anything that reads the buffer in a `then` reads it a loop later --
    // holding output that arrived after the snapshot was asked for.
    //
    // Downstream that is not a stale screen, it is a duplicated one: those same
    // bytes are recorded as log frames written after the checkpoint, so a
    // restore applies them on top of a screen that already has them.
    createScreen('later', 80, 24)
    feedScreen('later', 'before the snapshot\r\n')

    const pending = serializeScreen('later')
    feedScreen('later', 'after the snapshot\r\n')
    const snapshot = await pending

    expect(snapshot?.screen).toContain('before the snapshot')
    expect(snapshot?.screen ?? '', 'the snapshot ran a parse loop late').not.toContain(
      'after the snapshot'
    )
  })

  it('still waits for everything that was written before it', async () => {
    // The other half, and the reason this cannot simply serialize synchronously:
    // `term.write` returns before anything is parsed at all.
    createScreen('earlier', 80, 24)
    feedScreen('earlier', 'queued but not yet parsed\r\n')

    expect((await serializeScreen('earlier'))?.screen).toContain('queued but not yet parsed')
  })
})

describe('what the screen looks like when it comes back', () => {
  it('reproduces text and colour cell for cell', async () => {
    feed('t', `${ESC}[31mred${ESC}[0m plain ${ESC}[1;34mbold blue${ESC}[0m`)
    const snapshot = await serializeScreen('t')
    const dump = snapshot?.screen ?? ''

    const restored = await restore(dump)
    const live = await restore(`${ESC}[31mred${ESC}[0m plain ${ESC}[1;34mbold blue${ESC}[0m`)

    // Every cell of every row, not just the one with text on it. A screen that
    // matched on row zero and diverged on row nine would be exactly the quiet
    // wrongness this is meant to rule out.
    expect(screenOf(restored)).toEqual(screenOf(live))
    restored.dispose()
    live.dispose()
  })

  it('puts the cursor back where it was', async () => {
    feed('t', `line one\r\nline two\r\n${ESC}[1;4H`)
    const snapshot = await serializeScreen('t')
    const dump = snapshot?.screen ?? ''

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
    feed('t', paint)
    const snapshot = await serializeScreen('t')
    const dump = snapshot?.screen ?? ''

    const restored = await restore(dump)
    const live = await restore(paint)

    expect(screenOf(restored)).toEqual(screenOf(live))
    expect(restored.buffer.active.type, 'not on the alternate screen').toBe('alternate')
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
    feed('q', `${ESC}[cbefore${ESC}[5nafter`)
    const snapshot = await serializeScreen('q')
    const dump = snapshot?.screen ?? ''

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
    feed('t', 'hello world')
    const snapshot = await serializeScreen('t')
    const dump = snapshot?.screen ?? ''

    expect(dump).toContain('hello world')
  })

  it('sees every write in order when several arrive before a read', async () => {
    feed('t', 'one ')
    feed('t', 'two ')
    feed('t', 'three')

    expect((await serializeScreen('t'))?.screen).toContain('one two three')
  })
})

describe('following the terminal it models', () => {
  it('starts at the geometry the session was given', async () => {
    feed('t', 'x'.repeat(100), 40, 10)
    const snapshot = await serializeScreen('t')
    const dump = snapshot?.screen ?? ''

    // Forty columns means the hundred characters wrap onto a third line; eighty
    // would not. The wrap is the observable difference.
    const restored = await restore(dump, 40, 10)
    expect(restored.buffer.active.getLine(2)?.translateToString(true)).toContain('x')
    restored.dispose()
  })

  it('falls back to a PTY default when the session has no geometry', async () => {
    feed('t', 'hello')
    expect((await serializeScreen('t'))?.screen).toContain('hello')
  })

  it('follows a resize', async () => {
    feed('t', 'hello', 80, 24)
    await resizeScreen('t', 100, 40)
    feed('t', ' world')

    expect((await serializeScreen('t'))?.screen).toContain('hello world')
  })

  it('ignores a resize for a session it does not model', async () => {
    await expect(resizeScreen('absent', 100, 40)).resolves.toBeUndefined()
  })
})

describe('keeping terminals apart, and letting them go', () => {
  it('models each session separately', async () => {
    feed('a', 'first')
    feed('b', 'second')

    expect((await serializeScreen('a'))?.screen).toContain('first')
    expect((await serializeScreen('a'))?.screen).not.toContain('second')
    expect((await serializeScreen('b'))?.screen).toContain('second')
  })

  it('reads a session it has never seen as empty', async () => {
    expect(await serializeScreen('never')).toBeNull()
  })

  it('forgets a terminal that has gone', async () => {
    feed('t', 'something')
    expect(screenCount()).toBe(1)

    clearScreen('t')

    expect(screenCount()).toBe(0)
    expect(await serializeScreen('t')).toBeNull()
  })

  it('does not throw when clearing one that was never there', () => {
    expect(() => clearScreen('absent')).not.toThrow()
  })

  it('releases every terminal it held', () => {
    for (let i = 0; i < 20; i++) feed(`s${i}`, 'output')
    expect(screenCount()).toBe(20)

    resetScreens()

    expect(screenCount()).toBe(0)
  })
})

describe('what travels beside the screen', () => {
  it('carries the geometry it has to be replayed at', async () => {
    // Without it a caller cannot know what size to restore into, and a screen
    // replayed at the wrong width wraps in different places -- which is the
    // whole failure this model exists to avoid.
    createScreen('t', 132, 43)
    feedScreen('t', 'hello')

    const snapshot = await serializeScreen('t')

    expect(snapshot).toMatchObject({ cols: 132, rows: 43 })
  })

  it('follows the geometry through a resize', async () => {
    createScreen('t', 80, 24)
    feedScreen('t', 'hello')
    await resizeScreen('t', 100, 40)

    expect(await serializeScreen('t')).toMatchObject({ cols: 100, rows: 40 })
  })

  it('carries the title the program set, which the screen does not', async () => {
    feed('t', `${ESC}]0;vorn — building${ESC}\\`)

    expect((await serializeScreen('t'))?.title).toBe('vorn — building')
  })

  it('carries the working directory the shell reported', async () => {
    feed('t', `${ESC}]7;file://host/Users/x/dev/vorn${ESC}\\`)

    expect((await serializeScreen('t'))?.cwd).toBe('/Users/x/dev/vorn')
  })

  it('decodes a working directory with spaces in it', async () => {
    feed('t', `${ESC}]7;file://host/Users/x/my%20projects${ESC}\\`)

    expect((await serializeScreen('t'))?.cwd).toBe('/Users/x/my projects')
  })

  it('survives a working directory that is not valid percent-encoding', async () => {
    // `cd /tmp/100%` and the shell reports exactly this. `decodeURIComponent`
    // throws on a stray `%`, and the OSC handler runs inside xterm's own timer
    // -- so the throw reaches the top of the process, which installs no handler
    // for it. A directory somebody named would have killed the server and every
    // session on it.
    feed('t', `${ESC}]7;file://host/tmp/100%${ESC}\\after`)

    const snapshot = await serializeScreen('t')

    expect(snapshot?.cwd, 'left encoded rather than lost').toBe('/tmp/100%')
    expect(snapshot?.screen).toContain('after')
  })

  it('reports neither when the program never said', async () => {
    feed('t', 'just output')

    expect(await serializeScreen('t')).toMatchObject({ title: '', cwd: '' })
  })
})

describe('output arriving faster than it can be parsed', () => {
  it('skips ahead rather than holding it all, and keeps the model', async () => {
    // xterm parses on a timer and holds what it has not reached. A `cat` of
    // something large outruns that, and xterm's own answer at fifty megabytes is
    // to throw -- which would cost the session its model for good, having held
    // fifty megabytes to get there. A model missing part of a flood is repaired
    // by the next repaint; a model that no longer exists is not.
    createScreen('flood', 80, 24)
    const chunk = 'x'.repeat(64 * 1024)
    for (let i = 0; i < 200; i++) feedScreen('flood', chunk)

    // Still modelled: the flood cost it chunks, not its existence.
    expect(screenCount()).toBe(1)

    // And usable again once the parser has caught up, which is what a session
    // between two bursts of output looks like. Serializing drains the queue.
    await serializeScreen('flood')
    feedScreen('flood', `${ESC}[2J${ESC}[Hrepainted`)

    expect((await serializeScreen('flood'))?.screen).toContain('repainted')
  })
})
