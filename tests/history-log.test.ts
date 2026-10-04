import { describe, it, expect } from 'vitest'
import {
  crc32,
  cursorAfter,
  writeHeader,
  readHeader,
  readFrames,
  frameData,
  frameResize,
  FORMAT_VERSION,
  type LogRecord
} from '../packages/server/src/history/log'

/**
 * Reading a file that was being written when the process died.
 *
 * That is not an edge case here, it is the design input: the whole reason this
 * format exists is that a crash runs nothing, so the last thing on disk is
 * whatever the kernel had flushed at the moment the process stopped. Every test
 * below is a shape a real crash produces.
 *
 * Written before anything wrote a file, because a format that is wrong is
 * discovered months later as a terminal that redraws strangely, and by then the
 * bad files already exist.
 */

const START = { epoch: 9, nextRseq: 0, nextOffset: 0 }
const HEADER = writeHeader(1, START)

/** Records numbered from the start of the log, as the PTY reader numbers them. */
function numbered(...items: Array<string | [number, number]>): Buffer[] {
  let rseq = 0
  let offset = 0
  return items.map((item) => {
    const at = { rseq: rseq++, startOffset: offset }
    if (typeof item !== 'string') return frameResize(at, item[0], item[1])
    const bytes = Buffer.from(item, 'utf-8')
    offset += bytes.length
    return frameData(at, bytes)
  })
}

const log = (...frames: Buffer[]): Buffer => Buffer.concat([HEADER, ...frames])
const read = (buf: Buffer): ReturnType<typeof readFrames> => readFrames(buf, readHeader(buf)!)
const texts = (records: LogRecord[]): string[] =>
  records.map((r) => (r.kind === 'data' ? r.data : `${r.cols}x${r.rows}`))

/** Our magic, a version we do not speak. The shape a future writer leaves. */
function versioned(version: number): Buffer {
  const buf = Buffer.from(HEADER)
  buf.writeUInt8(version, 4)
  return buf
}

describe('the header', () => {
  it('round-trips, with the cursor the log starts at', () => {
    const start = { epoch: 0xfffffffe, nextRseq: 2 ** 40, nextOffset: 2 ** 52 + 3 }
    expect(readHeader(writeHeader(7, start))).toEqual({
      formatVersion: FORMAT_VERSION,
      generation: 7,
      start,
      bytes: writeHeader(7, start).length
    })
  })

  it.each([
    ['an empty file', Buffer.alloc(0)],
    ['a file shorter than the header', Buffer.from('VRN')],
    ['a file that is not ours', Buffer.from('SQLite format 3\0')],
    ['a file from a version this one does not know', versioned(FORMAT_VERSION + 1)],
    ['a version 2 header cut short', HEADER.subarray(0, HEADER.length - 1)]
  ])('refuses %s rather than guessing', (_label, buf) => {
    // All of these are ordinary things to find after a crash, and the answer to
    // all of them is the same: there is no history here, start again.
    expect(readHeader(buf)).toBeNull()
  })
})

describe('records', () => {
  it('round-trips every kind, in order, with its place', () => {
    const { records, reason } = read(log(...numbered('\x1b[31mred\x1b[0m', [200, 50], 'after')))

    expect(reason).toBe('end')
    expect(records).toEqual<LogRecord[]>([
      { kind: 'data', rseq: 0, startOffset: 0, stream: 0, data: '\x1b[31mred\x1b[0m' },
      { kind: 'resize', rseq: 1, startOffset: 12, cols: 200, rows: 50, pxWidth: 0, pxHeight: 0 },
      { kind: 'data', rseq: 2, startOffset: 12, stream: 0, data: 'after' }
    ])
  })

  it('counts offsets in bytes, not in UTF-16 units', () => {
    const { records } = read(log(...numbered('▁▂▃ 日本語 🙂', 'next')))
    expect(records[0]).toMatchObject({ data: '▁▂▃ 日本語 🙂', startOffset: 0 })
    expect(records[1]).toMatchObject({ startOffset: Buffer.byteLength('▁▂▃ 日本語 🙂') })
  })

  it('names the cursor after a record: the first record and byte it does not include', () => {
    // The worked example from the Session Recovery Contract: record 7 starts at
    // byte 100 and carries 20 bytes, so a state that includes it resumes at
    // record 8, byte 120 -- never at 100, which would send bytes 100-119 twice.
    const record: LogRecord = {
      kind: 'data',
      rseq: 7,
      startOffset: 100,
      stream: 0,
      data: 'x'.repeat(20)
    }
    expect(cursorAfter(3, record)).toEqual({ epoch: 3, nextRseq: 8, nextOffset: 120 })
    const resize: LogRecord = {
      ...record,
      kind: 'resize',
      cols: 1,
      rows: 1,
      pxWidth: 0,
      pxHeight: 0
    }
    expect(cursorAfter(3, resize)).toEqual({ epoch: 3, nextRseq: 8, nextOffset: 100 })
  })

  it('carries an empty write without losing its place', () => {
    expect(texts(read(log(...numbered('', 'after'))).records)).toEqual(['', 'after'])
  })
})

describe('a version 1 log, written before records had places', () => {
  /** The old layout, byte for byte, so an update can still restore what it finds. */
  function legacy(...frames: Array<[number, Buffer]>): Buffer {
    const header = Buffer.alloc(9)
    header.write('VRNL', 0, 'ascii')
    header.writeUInt8(1, 4)
    header.writeUInt32LE(4, 5)
    const body = frames.map(([kind, payload]) => {
      const frame = Buffer.alloc(9 + payload.length)
      frame.writeUInt8(kind, 0)
      frame.writeUInt32LE(payload.length, 1)
      frame.writeUInt32LE(crc32(payload), 5)
      payload.copy(frame, 9)
      return frame
    })
    return Buffer.concat([header, ...body])
  }
  const batch = (seq: number): [number, Buffer] => {
    const b = Buffer.alloc(4)
    b.writeUInt32LE(seq, 0)
    return [0x01, b]
  }
  const output = (text: string): [number, Buffer] => [0x02, Buffer.from(text, 'utf-8')]
  const resize = (cols: number, rows: number): [number, Buffer] => {
    const b = Buffer.alloc(4)
    b.writeUInt16LE(cols, 0)
    b.writeUInt16LE(rows, 2)
    return [0x03, b]
  }

  it('is still read, with batch markers dropped and records numbered in order', () => {
    const buf = legacy(batch(1), output('héllo'), resize(100, 30), batch(2), output('!'))
    const header = readHeader(buf)!
    expect(header).toEqual({ formatVersion: 1, generation: 4, bytes: 9 })
    expect(readFrames(buf, header)).toMatchObject({
      reason: 'end',
      records: [
        { kind: 'data', rseq: 0, startOffset: 0, data: 'héllo' },
        { kind: 'resize', rseq: 1, startOffset: 6, cols: 100, rows: 30 },
        { kind: 'data', rseq: 2, startOffset: 6, data: '!' }
      ]
    })
  })

  it('reports one of its kinds with the wrong payload size as malformed', () => {
    const buf = legacy(output('ok'), [0x03, Buffer.alloc(2)])
    expect(read(buf)).toMatchObject({ reason: 'malformed', records: [{ data: 'ok' }] })
  })
})

describe('a file the crash was in the middle of', () => {
  it('replays its complete prefix and nothing else', () => {
    const frames = numbered('first', 'second', 'third')
    const whole = log(...frames)

    // Cut inside the last frame's payload, which is where a crash lands.
    const torn = whole.subarray(0, whole.length - 3)
    const { records, reason, consumed } = read(torn)

    expect(texts(records)).toEqual(['first', 'second'])
    expect(reason).toBe('torn')
    // Consumed points at the start of the incomplete frame, so a caller can
    // truncate the file to exactly what was whole and append from there.
    expect(consumed).toBe(whole.length - frames[2]!.length)
  })

  it('survives a cut inside the frame header itself', () => {
    const frames = numbered('first', 'second')
    const whole = log(...frames)
    const torn = whole.subarray(0, whole.length - frames[1]!.length + 3)

    const { records, reason } = read(torn)
    expect(texts(records)).toEqual(['first'])
    expect(reason).toBe('torn')
  })

  it('refuses a length that runs past the end rather than trusting it', () => {
    // A torn length field can read as an enormous number. Slicing on it would
    // answer with a short buffer whose checksum then fails, reporting corruption
    // where the truth is a tear.
    const [frame] = numbered('x')
    const buf = log(frame!)
    buf.writeUInt32LE(0xffffff, buf.length - frame!.length + 1)

    expect(read(buf).reason).toBe('torn')
  })
})

describe('a byte that changed', () => {
  /** Flip a bit inside the middle record's text. */
  function corrupted(): Buffer {
    const frames = numbered('good', 'corrupted', 'after')
    const flipped = Buffer.from(log(...frames))
    // Past the frame prefix (9), the record header (16) and the stream byte.
    flipped[HEADER.length + frames[0]!.length + 9 + 17 + 2] ^= 0x20
    return flipped
  }

  it('is caught, and ends the replay there', () => {
    const { records, reason } = read(corrupted())
    expect(texts(records)).toEqual(['good'])
    expect(reason).toBe('checksum')
  })

  it('does not step over the bad frame to reach the good one after it', () => {
    // The bytes after a frame nobody can vouch for have no established meaning.
    // A slightly stale screen is a small wrong; one assembled from unverified
    // bytes is an unbounded one.
    expect(read(corrupted()).records).toHaveLength(1)
  })

  it('catches a flip in a record number, which the checksum covers', () => {
    const flipped = Buffer.from(log(...numbered('hello', 'world')))
    flipped[HEADER.length + 9] ^= 0x01
    expect(read(flipped)).toMatchObject({ reason: 'checksum', records: [] })
  })

  it('catches a flip in the length field too', () => {
    const flipped = Buffer.from(log(...numbered('hello', 'world')))
    flipped[HEADER.length + 1] ^= 0x01

    expect(read(flipped).reason).not.toBe('end')
  })
})

describe('a frame this version does not know', () => {
  it('stops rather than skipping into the middle of something', () => {
    // A kind from a future version -- the contract's Gap or Exit, say -- with a
    // valid length and checksum.
    const alien = Buffer.alloc(9)
    alien.writeUInt8(0x7f, 0)
    alien.writeUInt32LE(0, 1)
    alien.writeUInt32LE(crc32(Buffer.alloc(0)), 5)

    const { records, reason } = read(Buffer.concat([log(...numbered('known')), alien]))

    expect(texts(records)).toEqual(['known'])
    expect(reason).toBe('unknown-kind')
  })

  it.each([
    ['a resize with the wrong body size', 0x11, 16 + 4],
    ['a record too short to hold its place', 0x10, 8],
    ['data without its stream byte', 0x10, 16]
  ])('reports %s as malformed', (_label, kind, size) => {
    const bad = Buffer.alloc(9 + size)
    bad.writeUInt8(kind, 0)
    bad.writeUInt32LE(size, 1)
    bad.writeUInt32LE(crc32(Buffer.alloc(size)), 5)

    expect(read(Buffer.concat([HEADER, bad])).reason).toBe('malformed')
  })
})

describe('the checksum itself', () => {
  it('matches the published CRC-32 of "123456789"', () => {
    // The check value every IEEE CRC-32 implementation is tested against. Worth
    // pinning: a subtly wrong table produces checksums that are perfectly
    // self-consistent and agree with nothing else, including a future reader.
    expect(crc32(Buffer.from('123456789'))).toBe(0xcbf43926)
  })

  it('is zero-length safe', () => {
    expect(crc32(Buffer.alloc(0))).toBe(0)
  })

  it('changes when any single byte does', () => {
    const base = Buffer.from('the quick brown fox')
    for (let i = 0; i < base.length; i++) {
      const altered = Buffer.from(base)
      altered[i] ^= 0x01
      expect(crc32(altered), `byte ${i} went unnoticed`).not.toBe(crc32(base))
    }
  })
})
