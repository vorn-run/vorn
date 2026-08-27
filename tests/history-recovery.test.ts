import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { recoverHistory } from '../packages/server/src/history/recovery'
import {
  historyDir,
  writeCheckpoint,
  readCheckpoint,
  CHECKPOINT_FILE,
  LOG_FILE,
  type Checkpoint
} from '../packages/server/src/history/checkpoint'
import {
  writeHeader,
  frameBatch,
  frameOutput,
  frameResize
} from '../packages/server/src/history/log'
import {
  configureHistory,
  startHistory,
  recordOutput,
  recordResize,
  flushHistory,
  resetHistory
} from '../packages/server/src/history/writer'
import {
  createScreen,
  feedScreen,
  serializeScreen,
  resizeScreen,
  resetScreens
} from '../packages/server/src/terminal-screen'
import {
  appendScrollback,
  readScrollback,
  resetScrollback
} from '../packages/server/src/terminal-scrollback'

/**
 * Reading back what the last process left.
 *
 * The end of this file is the one that matters: write history with the real
 * writer, throw away every model the way a crash does, and restore. Everything
 * above it is a shape a crash produces that the round trip cannot reach --
 * a log from before the checkpoint beside it, a tail cut mid-frame, a directory
 * for a session nothing can name.
 */

const ID = 'a-session'
let dir: string

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-recovery-'))
  resetScreens()
  resetScrollback()
  resetHistory()
})

afterEach(() => {
  vi.restoreAllMocks()
  resetHistory()
  resetScreens()
  resetScrollback()
  fs.rmSync(dir, { recursive: true, force: true })
})

const sample = (over: Partial<Checkpoint> = {}): Checkpoint => ({
  screen: 'from the checkpoint',
  scrollback: 'from the checkpoint',
  cols: 100,
  rows: 30,
  title: '',
  cwd: '',
  generation: 4,
  seq: 9,
  ...over
})

async function put(
  checkpoint: Checkpoint | null,
  logGeneration?: number,
  ...frames: Buffer[]
): Promise<void> {
  const at = historyDir(dir, ID)
  fs.mkdirSync(at, { recursive: true })
  if (checkpoint) await writeCheckpoint(at, checkpoint)
  if (logGeneration !== undefined) {
    fs.writeFileSync(
      path.join(at, LOG_FILE),
      Buffer.concat([writeHeader(logGeneration), ...frames])
    )
  }
}

const only = [{ id: ID }]

const ESC = '\x1b'
/** Built rather than a literal, which the control-character rule refuses. */
const SGR = new RegExp(`${ESC}\\[[0-9;]*m`, 'g')

/** What the restored screen draws, with the colour taken back out of it. */
async function screenText(id = ID): Promise<string> {
  const snapshot = await serializeScreen(id)
  return (snapshot?.screen ?? '').replace(SGR, '')
}

describe('a checkpoint and the log that follows it', () => {
  it('replays the log on top of the checkpoint', async () => {
    await put(sample(), 4, frameBatch(1), frameOutput(' and then the log'))

    const report = await recoverHistory(dir, only)

    expect(report.recovered).toEqual([
      { id: ID, replayed: 2, stopped: 'end', fromCheckpoint: true }
    ])
    expect(readScrollback(ID)).toBe('from the checkpoint and then the log')
    expect(await screenText()).toContain('from the checkpoint and then the log')
  })

  it('rebuilds at the geometry the checkpoint was taken at', async () => {
    // The screen is being rebuilt from bytes that wrapped at those columns. Any
    // other width moves every line after the first wrap.
    await put(sample({ cols: 100, rows: 30 }), 4)

    await recoverHistory(dir, only)

    expect(await serializeScreen(ID)).toMatchObject({ cols: 100, rows: 30 })
  })

  it('follows a resize that happened after it', async () => {
    await put(sample(), 4, frameBatch(1), frameResize(132, 43))

    await recoverHistory(dir, only)

    expect(await serializeScreen(ID)).toMatchObject({ cols: 132, rows: 43 })
  })
})

describe('a log that does not belong to the checkpoint beside it', () => {
  it('is refused rather than replayed twice', async () => {
    // The crash window the writer cannot close: the checkpoint landed and the
    // log had not been replaced yet, so the log holds bytes the checkpoint
    // already contains.
    await put(sample({ generation: 5 }), 4, frameBatch(1), frameOutput('from the checkpoint'))

    const report = await recoverHistory(dir, only)

    expect(report.recovered[0]?.replayed).toBe(0)
    expect(readScrollback(ID)).toBe('from the checkpoint')
  })

  it('still restores the checkpoint itself', async () => {
    await put(sample({ generation: 5 }), 4, frameBatch(1), frameOutput('stale'))

    await recoverHistory(dir, only)

    expect(await screenText()).toContain('from the checkpoint')
    expect(readScrollback(ID)).not.toContain('stale')
  })
})

describe('a log with no checkpoint', () => {
  it('is rebuilt at the size a PTY is spawned at, which is where the log begins', async () => {
    // Not a guess and not a default. A log with no checkpoint opens at the
    // spawn, and a PTY is spawned at 80 by 24; any resize the terminal saw is a
    // frame further down that same log.
    await put(null, 1, frameBatch(1), frameOutput('from the very beginning'))

    await recoverHistory(dir, only)

    expect(await serializeScreen(ID)).toMatchObject({ cols: 80, rows: 24 })
  })

  it('is replayed from nothing, because it starts from nothing', async () => {
    // A session that crashed before its first checkpoint has a complete log --
    // exactly the short-lived session an interval was never going to cover.
    await put(null, 1, frameBatch(1), frameOutput('everything this terminal ever printed'))

    const report = await recoverHistory(dir, only)

    expect(report.recovered[0]).toMatchObject({ fromCheckpoint: false, replayed: 2 })
    expect(readScrollback(ID)).toBe('everything this terminal ever printed')
  })
})

describe('a file the crash was in the middle of', () => {
  it('replays its whole prefix and says where it stopped', async () => {
    const whole = Buffer.concat([frameBatch(1), frameOutput('kept'), frameOutput('torn away')])
    const at = historyDir(dir, ID)
    fs.mkdirSync(at, { recursive: true })
    await writeCheckpoint(at, sample())
    fs.writeFileSync(
      path.join(at, LOG_FILE),
      Buffer.concat([writeHeader(4), whole.subarray(0, whole.length - 4)])
    )

    const report = await recoverHistory(dir, only)

    expect(report.recovered[0]?.stopped).toBe('torn')
    expect(readScrollback(ID)).toBe('from the checkpointkept')
  })

  it('does not step over a frame that failed its checksum', async () => {
    const at = historyDir(dir, ID)
    fs.mkdirSync(at, { recursive: true })
    await writeCheckpoint(at, sample())
    const body = Buffer.concat([
      writeHeader(4),
      frameOutput('good'),
      frameOutput('corrupted'),
      frameOutput('after')
    ])
    body[writeHeader(4).length + frameOutput('good').length + 9 + 2] ^= 0x20
    fs.writeFileSync(path.join(at, LOG_FILE), body)

    const report = await recoverHistory(dir, only)

    expect(report.recovered[0]?.stopped).toBe('checksum')
    expect(readScrollback(ID)).toBe('from the checkpointgood')
  })
})

describe('what is left of sessions that are gone', () => {
  it('is removed, because nothing can name it any more', async () => {
    // History is keyed by session id and `getPreviousSessions` is the only way a
    // pane ever names one. A directory with no session behind it is not history
    // somebody might want, it is history nobody can ask for.
    await put(sample(), 4)
    fs.mkdirSync(historyDir(dir, 'a-session-that-ended'), { recursive: true })
    await writeCheckpoint(historyDir(dir, 'a-session-that-ended'), sample())

    const report = await recoverHistory(dir, only)

    expect(report.swept).toBe(1)
    expect(fs.existsSync(historyDir(dir, 'a-session-that-ended'))).toBe(false)
    expect(fs.existsSync(historyDir(dir, ID))).toBe(true)
  })

  it('leaves a session with no history alone', async () => {
    const report = await recoverHistory(dir, [{ id: 'never-recorded' }])
    expect(report).toEqual({ recovered: [], swept: 0 })
  })

  it('answers nothing on a first run, where there is no history directory at all', async () => {
    expect(await recoverHistory(path.join(dir, 'nowhere'), only)).toEqual({
      recovered: [],
      swept: 0
    })
  })
})

describe('residue that is not history', () => {
  it.each([
    ['an empty directory', () => fs.mkdirSync(historyDir(dir, ID), { recursive: true })],
    [
      'a checkpoint that will not parse',
      () => {
        fs.mkdirSync(historyDir(dir, ID), { recursive: true })
        fs.writeFileSync(path.join(historyDir(dir, ID), CHECKPOINT_FILE), 'not json')
      }
    ],
    [
      'a log that is not ours',
      () => {
        fs.mkdirSync(historyDir(dir, ID), { recursive: true })
        fs.writeFileSync(path.join(historyDir(dir, ID), LOG_FILE), 'SQLite format 3\0')
      }
    ]
  ])('restores nothing from %s rather than failing the startup', async (_label, leave) => {
    leave()
    const report = await recoverHistory(dir, only)
    expect(report.recovered).toEqual([])
  })
})

describe('more terminals than are rebuilt at once', () => {
  it('stops at the cap, keeps the rest, and says it did', async () => {
    // Each rebuilt terminal is a headless emulator held for the life of the
    // process, because nothing has attached to one yet and so nothing disposes
    // it. A cap nobody is told about reads as "everything was restored" on the
    // one start where it was not.
    const many = []
    for (let i = 0; i < 55; i++) {
      const id = `session-${i}`
      many.push({ id })
      fs.mkdirSync(historyDir(dir, id), { recursive: true })
      await writeCheckpoint(historyDir(dir, id), sample())
    }

    const report = await recoverHistory(dir, many)

    expect(report.recovered).toHaveLength(50)
    expect(report.swept).toBe(0)
    // Nothing was removed to make room; a later start can have them.
    expect(fs.readdirSync(path.join(dir, 'history'))).toHaveLength(55)
  })
})

describe('a session list that could not be read', () => {
  it('sweeps nothing, because absence and failure look the same otherwise', async () => {
    // `getPreviousSessions` answers an empty list both when there are none and
    // when the database would not answer. Acting on the first is a sweep; acting
    // on the second is every terminal's history removed for one transient error.
    await put(sample(), 4)

    const report = await recoverHistory(dir, null)

    expect(report).toEqual({ recovered: [], swept: 0 })
    expect(fs.existsSync(historyDir(dir, ID))).toBe(true)
  })
})

describe('a scratch file a crash left half-written', () => {
  it('is cleared rather than kept for ever under a name nothing reuses', async () => {
    // `writeCheckpoint` removes its scratch file only when the write fails. A
    // process that dies mid-write removes nothing, and up to two megabytes sits
    // there for the life of the session directory.
    await put(sample(), 4)
    const leftover = path.join(historyDir(dir, ID), `.${CHECKPOINT_FILE}.deadbeefcafe`)
    fs.writeFileSync(leftover, 'half a checkpoint')

    await recoverHistory(dir, only)

    expect(fs.existsSync(leftover)).toBe(false)
    expect(readScrollback(ID)).toContain('from the checkpoint')
  })
})

describe('the whole round trip', () => {
  it('brings a terminal back through a crash that ran nothing', async () => {
    // The test the rest of this file exists to support. Write with the real
    // writer, throw away every in-memory model the way a SIGKILL does, and ask
    // what a fresh process can rebuild.
    configureHistory(dir, { tickMs: 5, quiesceMs: 500, checkpointMs: 60_000 })
    createScreen(ID, 120, 40)
    startHistory(ID)

    const said = '\x1b[32m✓\x1b[0m tests passed, 402 of them\r\n'
    for (const chunk of [said, 'and then some more output\r\n']) {
      appendScrollback(ID, chunk)
      feedScreen(ID, chunk)
      recordOutput(ID, chunk)
    }
    // Both halves, the way `resizePty` does it: the model follows the program
    // and the frame records that it did.
    await resizeScreen(ID, 132, 43)
    recordResize(ID, 132, 43)
    await flushHistory()

    // Everything this process was holding, gone.
    resetScreens()
    resetScrollback()
    resetHistory()

    const report = await recoverHistory(dir, only)

    expect(report.recovered[0]?.fromCheckpoint).toBe(true)
    expect(readScrollback(ID)).toContain('402 of them')
    expect(await screenText()).toContain('402 of them')
    expect(await screenText()).toContain('and then some more output')
    // The colour survives, which is the reason none of this stores stripped text.
    expect((await serializeScreen(ID))?.screen).toContain(`${ESC}[32m`)
    expect(await serializeScreen(ID)).toMatchObject({ cols: 132, rows: 43 })
  })

  it('leaves the files it recovered from in place', async () => {
    configureHistory(dir, { tickMs: 5, quiesceMs: 500, checkpointMs: 60_000 })
    createScreen(ID, 80, 24)
    startHistory(ID)
    feedScreen(ID, 'output')
    recordOutput(ID, 'output')
    await flushHistory()
    resetScreens()
    resetScrollback()
    resetHistory()

    await recoverHistory(dir, only)

    // A restart that crashes again should have the same thing to read. Nothing
    // here is consumed by being read.
    expect(readCheckpoint(historyDir(dir, ID))?.screen).toContain('output')
  })
})
