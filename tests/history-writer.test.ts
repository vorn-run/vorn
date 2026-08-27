import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {
  configureHistory,
  startHistory,
  recordOutput,
  recordResize,
  stopHistory,
  flushHistory,
  settleHistory,
  historyState,
  resetHistory,
  MAX_LOG_BYTES
} from '../packages/server/src/history/writer'
import { recoverHistory } from '../packages/server/src/history/recovery'
import {
  historyDir,
  readCheckpoint,
  CHECKPOINT_FILE,
  LOG_FILE
} from '../packages/server/src/history/checkpoint'
import { readHeader, readFrames, type Frame } from '../packages/server/src/history/log'
import {
  createScreen,
  feedScreen,
  clearScreen,
  resetScreens
} from '../packages/server/src/terminal-screen'
import {
  appendScrollback,
  readScrollback,
  resetScrollback
} from '../packages/server/src/terminal-scrollback'

/**
 * Turning a running terminal into files a restart can read.
 *
 * The two modules under this one are pure -- one shapes buffers, one writes a
 * file -- so everything left to get wrong is *when*: what is superseded by a
 * checkpoint, what is carried past it, and what a shutdown must not undo. Those
 * are the tests here, and each was checked against the code with that specific
 * step removed.
 */

// A tick fast enough that a test does not wait on it, and a quiet window far
// enough past it that settling for a flush never trips a checkpoint by accident.
const TIMING = { tickMs: 5, quiesceMs: 500, checkpointMs: 60_000 }
const ID = 'a-session'

let dir: string

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-writer-'))
  resetScreens()
  resetScrollback()
  resetHistory()
  configureHistory(dir, TIMING)
})

afterEach(() => {
  vi.restoreAllMocks()
  resetHistory()
  resetScreens()
  resetScrollback()
  fs.rmSync(dir, { recursive: true, force: true })
})

/** What `flushBuffer` and `onData` do together, in the order they do it. */
function emit(id: string, data: string): void {
  appendScrollback(id, data)
  feedScreen(id, data)
  recordOutput(id, data)
}

function begin(id = ID, cols = 80, rows = 24): void {
  createScreen(id, cols, rows)
  startHistory(id)
}

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms))

/** Wait for the tick to have picked the work up, then for the work to finish. */
async function settle(rounds = 2): Promise<void> {
  for (let i = 0; i < rounds; i++) {
    await sleep(TIMING.tickMs * 3)
    await settleHistory()
  }
}

async function quiesce(): Promise<void> {
  await sleep(TIMING.quiesceMs)
  await settle()
}

/** Poll rather than settle, for the tests where one session is deliberately stuck. */
async function until(done: () => boolean, within = 2_000): Promise<void> {
  const give = Date.now() + within
  while (Date.now() < give) {
    // A predicate that reads a file the writer has not created yet is not
    // failure, it is "not yet".
    try {
      if (done()) return
    } catch {
      /* not yet */
    }
    await sleep(TIMING.tickMs)
  }
  throw new Error('the condition never became true')
}

function logOf(id = ID): { generation: number | null; frames: Frame[] } {
  const buf = fs.readFileSync(path.join(historyDir(dir, id), LOG_FILE))
  const header = readHeader(buf)
  return { generation: header?.generation ?? null, frames: readFrames(buf).frames }
}

const outputs = (frames: Frame[]): string =>
  frames
    .filter((f): f is Extract<Frame, { kind: 'output' }> => f.kind === 'output')
    .map((f) => f.data)
    .join('')

describe('what reaches the log', () => {
  it('writes a header before anything else, so a reader can refuse a file that is not ours', async () => {
    begin()
    await settleHistory()

    expect(logOf().generation).toBe(1)
    expect(logOf().frames).toEqual([])
  })

  it('carries output and resizes, in the order they happened', async () => {
    begin()
    emit(ID, 'first')
    recordResize(ID, 120, 40)
    emit(ID, 'second')
    await settle()

    expect(logOf().frames).toEqual<Frame[]>([
      { kind: 'batch', seq: 1 },
      { kind: 'output', data: 'first' },
      { kind: 'resize', cols: 120, rows: 40 },
      { kind: 'output', data: 'second' }
    ])
  })

  it('coalesces a tick of writes into one batch rather than one each', async () => {
    // A batch is what reaches the disk together, which is the unit a torn tail
    // cuts. Numbering per write would make that boundary meaningless and cost
    // thirteen bytes per keystroke.
    begin()
    for (let i = 0; i < 20; i++) emit(ID, `${i}`)
    await settle()

    const batches = logOf().frames.filter((f) => f.kind === 'batch')
    expect(batches).toEqual([{ kind: 'batch', seq: 1 }])
  })

  it('writes nothing at all before a data directory is configured', async () => {
    resetHistory()
    begin('unconfigured')
    emit('unconfigured', 'output')
    await settle()

    expect(historyState('unconfigured')).toBeNull()
    expect(fs.existsSync(path.join(dir, 'history'))).toBe(false)
  })
})

describe('the checkpoint', () => {
  it('is written once a terminal falls quiet, with everything a restore needs', async () => {
    begin(ID, 100, 30)
    emit(ID, '\x1b]0;vorn — building\x07')
    emit(ID, 'hello world')
    await quiesce()

    const written = readCheckpoint(historyDir(dir, ID))
    expect(written).not.toBeNull()
    expect(written?.screen).toContain('hello world')
    expect(written?.scrollback).toContain('hello world')
    expect(written).toMatchObject({ cols: 100, rows: 30, title: 'vorn — building' })
  })

  it('does not leave behind the frames it already contains', async () => {
    // The duplication this whole design turns on. A checkpoint holds the screen
    // as of the moment it was taken, so replaying the frames before it over the
    // top would print every byte twice.
    begin()
    emit(ID, 'hello world')
    await settle()
    expect(outputs(logOf().frames), 'they never reached the log, so this proves nothing').toBe(
      'hello world'
    )

    await quiesce()

    expect(outputs(logOf().frames)).toBe('')
  })

  it('moves the generation with it, so a log left over from before is refusable', async () => {
    begin()
    emit(ID, 'hello')
    await quiesce()

    const written = readCheckpoint(historyDir(dir, ID))
    expect(written?.generation).toBe(2)
    // The same number in both files is the whole tie. A crash between the two
    // writes leaves the old log beside the new checkpoint, and this is what lets
    // recovery notice rather than replay it.
    expect(logOf().generation).toBe(2)
  })

  it('keeps output that arrives after it', async () => {
    begin()
    emit(ID, 'before')
    await quiesce()
    emit(ID, 'after')
    await settle()

    expect(outputs(logOf().frames)).toBe('after')
    expect(readCheckpoint(historyDir(dir, ID))?.screen).toContain('before')
  })

  it('is not rewritten while nothing is happening', async () => {
    begin()
    emit(ID, 'hello')
    await quiesce()
    const first = fs.statSync(path.join(historyDir(dir, ID), CHECKPOINT_FILE)).mtimeMs

    await quiesce()

    expect(fs.statSync(path.join(historyDir(dir, ID), CHECKPOINT_FILE)).mtimeMs).toBe(first)
  })

  it('stops the timer once every session is written out', async () => {
    // Otherwise this ticks four times a second through every idle night, for
    // every server anyone leaves running.
    begin()
    emit(ID, 'hello')
    await quiesce()

    const before = process.getActiveResourcesInfo?.().filter((r) => r === 'Timeout').length ?? 0
    await sleep(TIMING.tickMs * 4)
    const after = process.getActiveResourcesInfo?.().filter((r) => r === 'Timeout').length ?? 0

    expect(after).toBeLessThanOrEqual(before)
    expect(historyState(ID)?.pendingBytes).toBe(0)
  })
})

describe('a session that ends', () => {
  it('takes its history with it', async () => {
    begin()
    emit(ID, 'hello')
    await quiesce()
    expect(fs.existsSync(historyDir(dir, ID))).toBe(true)

    stopHistory(ID)
    await settle()

    expect(fs.existsSync(historyDir(dir, ID))).toBe(false)
  })

  it('is refused once the server is shutting down', async () => {
    // `shutdown()` checkpoints every terminal and only then kills the PTYs, and
    // a killed PTY runs the same teardown as one that exited on its own. Without
    // the seal the last act of a clean shutdown is to delete what it just wrote.
    begin()
    emit(ID, 'hello')
    await flushHistory()

    stopHistory(ID)
    await settle()

    expect(readCheckpoint(historyDir(dir, ID))?.screen).toContain('hello')
  })

  it('orders its work behind the terminal it replaced, not beside it', async () => {
    // Two records over one directory. The replaced one is asked to remove its
    // files and the replacement to write fresh ones, and through separate queues
    // there is no order between those -- the removal can land after the new log
    // has been written and take it away. The symptom was a respawned session
    // whose history stopped until the next checkpoint rebuilt the directory, and
    // a test that failed about one run in five.
    //
    // The property, rather than the symptom: everything on this directory is in
    // one queue, so the replacement cannot start while the replaced session
    // still has an operation in flight.
    begin()
    let release: (() => void) | undefined
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    const real = fsp.appendFile
    vi.spyOn(fsp, 'appendFile').mockImplementationOnce((async (...args: [never, never]) => {
      await held
      return real(...args)
    }) as never)

    emit(ID, 'from the first process')
    await sleep(TIMING.tickMs * 3)

    begin()
    emit(ID, 'from the second process')

    let finished = false
    const settling = settleHistory().then(() => {
      finished = true
    })
    await sleep(TIMING.tickMs * 6)
    expect(finished, 'the replacement ran while the replaced session was mid-write').toBe(false)

    release?.()
    await settling
    await settle(3)

    expect(outputs(logOf().frames)).toBe('from the second process')
  })

  it('starts clean when the same id is recorded again', async () => {
    begin()
    emit(ID, 'from the first process')
    await quiesce()

    begin()
    await settle()

    expect(readCheckpoint(historyDir(dir, ID))).toBeNull()
    expect(logOf().generation).toBe(1)
  })
})

describe('shutting down', () => {
  it('writes every terminal, not only the ones that had fallen quiet', async () => {
    begin('one')
    begin('two')
    emit('one', 'first terminal')
    emit('two', 'second terminal')

    await flushHistory()

    expect(readCheckpoint(historyDir(dir, 'one'))?.screen).toContain('first terminal')
    expect(readCheckpoint(historyDir(dir, 'two'))?.screen).toContain('second terminal')
  })

  it('does not leave frames that never reached the log to be replayed over it', async () => {
    // The other half of the duplication. A shutdown checkpoints immediately, so
    // the frames are still in memory rather than on disk -- and they are inside
    // the screen just written, so carrying them past it would print twice.
    begin()
    emit(ID, 'hello world')
    expect(historyState(ID)?.pendingBytes, 'a tick got there first').toBeGreaterThan(0)

    await flushHistory()

    expect(outputs(logOf().frames)).toBe('')
  })
})

describe('a log that outgrows its cap', () => {
  const past = (): string => 'x'.repeat(MAX_LOG_BYTES + 1024)

  it('is folded into a checkpoint rather than left to grow', async () => {
    begin()
    emit(ID, past())
    await settle(3)

    expect(historyState(ID)?.logBytes).toBeLessThanOrEqual(MAX_LOG_BYTES)
    expect(readCheckpoint(historyDir(dir, ID))?.screen).toBeTruthy()
  })

  it('is bounded by size, not by waiting out the interval', async () => {
    // A terminal producing output continuously never falls quiet, so the quiet
    // window and the interval both pass it by. Nothing here waits for either.
    begin()
    emit(ID, past())
    await settle(3)

    expect(readCheckpoint(historyDir(dir, ID))?.generation).toBe(2)
  })

  it('is thrown away when no checkpoint can be written, and cannot be replayed after', async () => {
    // A session whose screen model has faulted has nothing to checkpoint from,
    // so the log is the only thing growing and nothing would ever bound it.
    // Dropping it loses history on purpose; the last good checkpoint remains.
    begin()
    emit(ID, 'this much was checkpointed')
    await quiesce()
    expect(readCheckpoint(historyDir(dir, ID))?.generation).toBe(2)

    clearScreen(ID)
    recordOutput(ID, past())
    await settle(3)

    expect(historyState(ID)?.logBytes).toBeLessThanOrEqual(MAX_LOG_BYTES)
    // The generation moved with the log and not with the checkpoint, which is
    // what stops a truncated log being replayed onto a screen it does not follow.
    expect(logOf().generation).toBe(3)
    expect(readCheckpoint(historyDir(dir, ID))?.generation).toBe(2)

    const report = await recoverHistory(dir, [{ id: ID, cols: 80, rows: 24 }])
    expect(report.recovered[0]?.replayed).toBe(0)
    expect(readScrollback(ID)).toBe('this much was checkpointed')
  })
})

describe('a disk that stops answering', () => {
  it('gives up on the log rather than growing without bound', async () => {
    begin()
    vi.spyOn(fsp, 'appendFile').mockRejectedValue(new Error('EIO'))

    emit(ID, 'x'.repeat(1024))
    await settle()

    expect(historyState(ID)?.broken).toBe(true)
    expect(historyState(ID)?.pendingBytes).toBe(0)
  })

  it('does not append past the gap it left', async () => {
    // A hole in the middle of a file that replays forwards is worse than a stale
    // screen: every byte after it is applied at the wrong point. The log stays
    // untouched until a checkpoint replaces it whole.
    begin()
    const append = vi.spyOn(fsp, 'appendFile').mockRejectedValueOnce(new Error('EIO'))

    emit(ID, 'lost')
    await settle()
    emit(ID, 'after')
    await settle()

    expect(outputs(logOf().frames)).toBe('')
    expect(append).toHaveBeenCalledTimes(1)
  })

  it('is repaired by the next checkpoint', async () => {
    begin()
    vi.spyOn(fsp, 'appendFile').mockRejectedValueOnce(new Error('EIO'))

    emit(ID, 'lost to the append')
    await settle()
    await quiesce()

    expect(historyState(ID)?.broken).toBe(false)
    // The bytes were never lost to the *screen*, only to the log, and the
    // checkpoint is the screen.
    expect(readCheckpoint(historyDir(dir, ID))?.screen).toContain('lost to the append')
  })
})

describe('one session against another', () => {
  it('does not make a healthy terminal wait behind a wedged checkpoint', async () => {
    // The rule the per-session queue exists for, on the operation it was written
    // about: a checkpoint holds one directory's write-and-rename pair, and that
    // exclusivity buys nothing across directories. A queue for the whole server
    // would make every terminal wait on the slowest disk under any of them.
    begin('wedged')
    begin('healthy')

    let release: (() => void) | undefined
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    const real = fsp.open
    vi.spyOn(fsp, 'open').mockImplementation((async (...args: Parameters<typeof fsp.open>) => {
      if (String(args[0]).includes('wedged')) await held
      return real(...args)
    }) as never)

    emit('wedged', 'this one is stuck')
    emit('healthy', 'this one is not')

    // Polled rather than settled: settling would wait on the session that is
    // deliberately stuck, which is the thing being shown not to matter.
    await until(() =>
      Boolean(readCheckpoint(historyDir(dir, 'healthy'))?.screen.includes('this one is not'))
    )
    expect(readCheckpoint(historyDir(dir, 'wedged'))).toBeNull()

    release?.()
    await settle()
  })

  it('does not make a healthy terminal wait behind a wedged append', async () => {
    begin('wedged')
    begin('healthy')

    let release: (() => void) | undefined
    const held = new Promise<void>((resolve) => {
      release = resolve
    })
    const real = fsp.appendFile
    vi.spyOn(fsp, 'appendFile').mockImplementation((async (file: string, data: Buffer) => {
      if (String(file).includes('wedged')) await held
      return real(file, data)
    }) as never)

    emit('wedged', 'this one is stuck')
    emit('healthy', 'this one is not')

    // Polled rather than settled: settling would wait on the session that is
    // deliberately stuck, which is the thing being shown not to matter.
    await until(() => outputs(logOf('healthy').frames) === 'this one is not')
    expect(logOf('wedged').frames).toEqual([])

    release?.()
    await settle()
  })
})
