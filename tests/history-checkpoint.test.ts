import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
import fsp from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {
  writeCheckpoint,
  readCheckpoint,
  historyDir,
  CHECKPOINT_FILE,
  MAX_CHECKPOINT_BYTES,
  type Checkpoint
} from '../packages/server/src/history/checkpoint'

/**
 * A handle that counts its own syncs, and can be told to refuse them.
 *
 * The write is asynchronous, so the fsyncs happen through `FileHandle.sync()`
 * rather than a module function there is anything to spy on. Wrapping `open` is
 * the seam: every handle it hands back is this one.
 */
function watchSyncs(options: { refuse?: boolean; refuseDir?: string } = {}): { count: number } {
  const counter = { count: 0 }
  const real = fsp.open
  vi.spyOn(fsp, 'open').mockImplementation((async (...args: Parameters<typeof fsp.open>) => {
    if (options.refuseDir !== undefined && args[0] === options.refuseDir) {
      throw new Error('EISDIR')
    }
    const handle = await real(...args)
    const sync = handle.sync.bind(handle)
    handle.sync = async (): Promise<void> => {
      counter.count += 1
      if (options.refuse) throw new Error('EINVAL')
      await sync()
    }
    return handle
  }) as never)
  return counter
}

/**
 * Writing a file that a crash must not be able to catch half-done.
 *
 * The interesting assertions here are about what is *never* observable: a reader
 * seeing a partial checkpoint, a scratch file surviving a failure, a directory
 * left holding an entry that was never synced.
 */

let dir: string

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-history-'))
})

afterEach(() => {
  vi.restoreAllMocks()
  fs.rmSync(dir, { recursive: true, force: true })
})

const sample = (over: Partial<Checkpoint> = {}): Checkpoint => ({
  screen: '\x1b[31mred\x1b[0m',
  scrollback: 'earlier output\r\n',
  cols: 200,
  rows: 50,
  title: 'vorn — building',
  cwd: '/Users/x/dev/vorn',
  generation: 3,
  seq: 1180,
  ...over
})

describe('writing one', () => {
  it('round-trips everything a restore needs', async () => {
    expect(await writeCheckpoint(dir, sample())).toBe(true)
    expect(readCheckpoint(dir)).toEqual(sample())
  })

  it('creates the directory it was given', async () => {
    const nested = historyDir(dir, 'a-session-id')
    expect(await writeCheckpoint(nested, sample())).toBe(true)
    expect(readCheckpoint(nested)).toEqual(sample())
  })

  it('leaves no scratch file behind', async () => {
    await writeCheckpoint(dir, sample())
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('replaces the previous one atomically', async () => {
    await writeCheckpoint(dir, sample({ seq: 1 }))
    await writeCheckpoint(dir, sample({ seq: 2 }))

    expect(readCheckpoint(dir)?.seq).toBe(2)
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('syncs the directory, not only the file', async () => {
    // A rename is atomic against other processes and not durable against power
    // loss until the directory entry is synced. Two fsyncs: the file's contents
    // before the rename makes them reachable, and the directory after.
    const syncs = watchSyncs()

    await writeCheckpoint(dir, sample())

    expect(syncs.count, 'the directory fsync is the one that gets forgotten').toBe(2)
  })

  it('never blocks the event loop while it does any of that', async () => {
    // The whole point of a queue per session is that a slow disk under one
    // terminal does not stop another. A synchronous write would have made that
    // untrue of every terminal at once, and of every socket and request beside
    // them.
    let ticked = false
    const beat = setInterval(() => {
      ticked = true
    }, 1)

    await writeCheckpoint(dir, sample({ scrollback: 'x'.repeat(512 * 1024) }))
    clearInterval(beat)

    expect(ticked, 'nothing else ran while the checkpoint was being written').toBe(true)
  })
})

describe('a checkpoint that will not fit', () => {
  it('is skipped whole rather than truncated', async () => {
    // Half a JSON document is not a smaller checkpoint, it is an unreadable one.
    // Restoring from an older checkpoint and a longer log is strictly better.
    const huge = sample({ scrollback: 'x'.repeat(MAX_CHECKPOINT_BYTES + 1) })

    expect(await writeCheckpoint(dir, huge)).toBe(false)
    expect(fs.existsSync(path.join(dir, CHECKPOINT_FILE))).toBe(false)
  })

  it('leaves an earlier one in place', async () => {
    await writeCheckpoint(dir, sample({ seq: 1 }))
    await writeCheckpoint(dir, sample({ seq: 2, scrollback: 'x'.repeat(MAX_CHECKPOINT_BYTES + 1) }))

    expect(readCheckpoint(dir)?.seq, 'the older checkpoint was lost to a failed write').toBe(1)
  })
})

describe('a write that fails part way', () => {
  it('leaves nothing behind and says so', async () => {
    await writeCheckpoint(dir, sample({ seq: 1 }))
    vi.spyOn(fsp, 'rename').mockRejectedValue(new Error('no space left on device'))

    expect(await writeCheckpoint(dir, sample({ seq: 2 }))).toBe(false)
    // The old checkpoint survives, and no scratch file is left to be mistaken
    // for one later.
    expect(readCheckpoint(dir)?.seq).toBe(1)
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('still writes when the filesystem refuses to sync at all', async () => {
    // Some filesystems will not open a directory for this, and some refuse the
    // sync itself. Either way the rename is still atomic against other readers,
    // so a checkpoint that is merely not power-loss-durable beats none -- and
    // aborting here would throw away the very history this exists to keep.
    watchSyncs({ refuse: true })

    expect(await writeCheckpoint(dir, sample())).toBe(true)
    expect(readCheckpoint(dir)).toEqual(sample())
  })

  it('still writes when the directory cannot even be opened to sync', async () => {
    watchSyncs({ refuseDir: dir })

    expect(await writeCheckpoint(dir, sample())).toBe(true)
    expect(readCheckpoint(dir)).toEqual(sample())
  })
})

describe('reading one that is not there, or not right', () => {
  it.each([
    ['nothing at all', null],
    ['an empty file', ''],
    ['not JSON', 'VRNL\x01'],
    ['JSON that is not a checkpoint', '{"hello":"world"}'],
    ['a checkpoint missing its geometry', '{"screen":"","scrollback":"","title":"","cwd":""}'],
    ['a checkpoint with a fractional seq', JSON.stringify({ ...sample(), seq: 1.5 })]
  ])('answers null for %s', (_label, body) => {
    // Every one of these is ordinary residue from a crash, and the answer to all
    // of them is the same: there is nothing to restore from.
    if (body !== null) fs.writeFileSync(path.join(dir, CHECKPOINT_FILE), body)
    expect(readCheckpoint(dir)).toBeNull()
  })
})

describe('where history lives', () => {
  it('sits under a subdirectory, not beside the database', () => {
    // `config-manager` watches the data directory itself. A log written there
    // would wake that watcher on every chunk of terminal output.
    expect(historyDir('/data', 'abc')).toBe(path.join('/data', 'history', 'abc'))
  })

  it('encodes an id that is not a safe path segment', () => {
    const encoded = historyDir('/data', 'a/b?c:d')
    expect(path.dirname(encoded)).toBe(path.join('/data', 'history'))
    expect(path.basename(encoded)).not.toContain('/')
  })
})
