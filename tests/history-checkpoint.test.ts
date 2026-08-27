import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import fs from 'node:fs'
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
  it('round-trips everything a restore needs', () => {
    expect(writeCheckpoint(dir, sample())).toBe(true)
    expect(readCheckpoint(dir)).toEqual(sample())
  })

  it('creates the directory it was given', () => {
    const nested = historyDir(dir, 'a-session-id')
    expect(writeCheckpoint(nested, sample())).toBe(true)
    expect(readCheckpoint(nested)).toEqual(sample())
  })

  it('leaves no scratch file behind', () => {
    writeCheckpoint(dir, sample())
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('replaces the previous one atomically', () => {
    writeCheckpoint(dir, sample({ seq: 1 }))
    writeCheckpoint(dir, sample({ seq: 2 }))

    expect(readCheckpoint(dir)?.seq).toBe(2)
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('syncs the directory, not only the file', () => {
    // A rename is atomic against other processes and not durable against power
    // loss until the directory entry is synced. Two fsyncs: the file's contents
    // before the rename makes them reachable, and the directory after.
    const fsync = vi.spyOn(fs, 'fsyncSync')

    writeCheckpoint(dir, sample())

    expect(fsync.mock.calls.length, 'the directory fsync is the one that gets forgotten').toBe(2)
  })
})

describe('a checkpoint that will not fit', () => {
  it('is skipped whole rather than truncated', () => {
    // Half a JSON document is not a smaller checkpoint, it is an unreadable one.
    // Restoring from an older checkpoint and a longer log is strictly better.
    const huge = sample({ scrollback: 'x'.repeat(MAX_CHECKPOINT_BYTES + 1) })

    expect(writeCheckpoint(dir, huge)).toBe(false)
    expect(fs.existsSync(path.join(dir, CHECKPOINT_FILE))).toBe(false)
  })

  it('leaves an earlier one in place', () => {
    writeCheckpoint(dir, sample({ seq: 1 }))
    writeCheckpoint(dir, sample({ seq: 2, scrollback: 'x'.repeat(MAX_CHECKPOINT_BYTES + 1) }))

    expect(readCheckpoint(dir)?.seq, 'the older checkpoint was lost to a failed write').toBe(1)
  })
})

describe('a write that fails part way', () => {
  it('leaves nothing behind and says so', () => {
    writeCheckpoint(dir, sample({ seq: 1 }))
    vi.spyOn(fs, 'renameSync').mockImplementation(() => {
      throw new Error('no space left on device')
    })

    expect(writeCheckpoint(dir, sample({ seq: 2 }))).toBe(false)
    // The old checkpoint survives, and no scratch file is left to be mistaken
    // for one later.
    expect(readCheckpoint(dir)?.seq).toBe(1)
    expect(fs.readdirSync(dir)).toEqual([CHECKPOINT_FILE])
  })

  it('still writes when the filesystem refuses to sync at all', () => {
    // Some filesystems will not open a directory for this, and some refuse the
    // sync itself. Either way the rename is still atomic against other readers,
    // so a checkpoint that is merely not power-loss-durable beats none -- and
    // aborting here would throw away the very history this exists to keep.
    vi.spyOn(fs, 'fsyncSync').mockImplementation(() => {
      throw new Error('EINVAL')
    })

    expect(writeCheckpoint(dir, sample())).toBe(true)
    expect(readCheckpoint(dir)).toEqual(sample())
  })

  it('still writes when the directory cannot even be opened to sync', () => {
    const realOpen = fs.openSync
    vi.spyOn(fs, 'openSync').mockImplementation(((p: string, flags: string, mode?: number) => {
      if (p === dir) throw new Error('EISDIR')
      return realOpen(p, flags as never, mode)
    }) as never)

    expect(writeCheckpoint(dir, sample())).toBe(true)
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
    ['a checkpoint with a fractional seq', JSON.stringify({ ...sampleRaw(), seq: 1.5 })]
  ])('answers null for %s', (_label, body) => {
    // Every one of these is ordinary residue from a crash, and the answer to all
    // of them is the same: there is nothing to restore from.
    if (body !== null) fs.writeFileSync(path.join(dir, CHECKPOINT_FILE), body)
    expect(readCheckpoint(dir)).toBeNull()
  })
})

function sampleRaw(): Record<string, unknown> {
  return {
    screen: '',
    scrollback: '',
    cols: 80,
    rows: 24,
    title: '',
    cwd: '',
    generation: 1,
    seq: 1
  }
}

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
