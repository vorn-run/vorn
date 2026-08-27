import { describe, it, expect } from 'vitest'
import {
  crc32,
  writeHeader,
  readHeader,
  readFrames,
  frameBatch,
  frameOutput,
  frameResize,
  FORMAT_VERSION,
  type Frame
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

const log = (...frames: Buffer[]): Buffer => Buffer.concat([writeHeader(1), ...frames])

/** Our magic, a version we do not speak. The shape a future writer leaves. */
function versioned(version: number): Buffer {
  const buf = Buffer.from(writeHeader(1))
  buf.writeUInt8(version, 4)
  return buf
}

describe('the header', () => {
  it('round-trips', () => {
    expect(readHeader(writeHeader(7))).toEqual({ formatVersion: FORMAT_VERSION, generation: 7 })
  })

  it.each([
    ['an empty file', Buffer.alloc(0)],
    ['a file shorter than the header', Buffer.from('VRN')],
    ['a file that is not ours', Buffer.from('SQLite format 3\0')],
    ['a file from a version this one does not know', versioned(FORMAT_VERSION + 1)]
  ])('refuses %s rather than guessing', (_label, buf) => {
    // All three are ordinary things to find after a crash, and the answer to all
    // three is the same: there is no history here, start again.
    expect(readHeader(buf)).toBeNull()
  })
})

describe('frames', () => {
  it('round-trips every kind, in order', () => {
    const buf = log(frameBatch(1180), frameOutput('\x1b[31mred\x1b[0m'), frameResize(200, 50))

    const { frames, reason } = readFrames(buf)

    expect(reason).toBe('end')
    expect(frames).toEqual<Frame[]>([
      { kind: 'batch', seq: 1180 },
      { kind: 'output', data: '\x1b[31mred\x1b[0m' },
      { kind: 'resize', cols: 200, rows: 50 }
    ])
  })

  it('carries bytes that are not ASCII', () => {
    const buf = log(frameOutput('▁▂▃ 日本語 🙂'))
    expect(readFrames(buf).frames).toEqual([{ kind: 'output', data: '▁▂▃ 日本語 🙂' }])
  })

  it('carries an empty write without losing its place', () => {
    const buf = log(frameOutput(''), frameOutput('after'))
    expect(readFrames(buf).frames).toEqual([
      { kind: 'output', data: '' },
      { kind: 'output', data: 'after' }
    ])
  })
})

describe('a file the crash was in the middle of', () => {
  it('replays its complete prefix and nothing else', () => {
    const whole = log(frameOutput('first'), frameOutput('second'), frameOutput('third'))

    // Cut inside the last frame's payload, which is where a crash lands.
    const torn = whole.subarray(0, whole.length - 3)
    const { frames, reason, consumed } = readFrames(torn)

    expect(frames).toEqual([
      { kind: 'output', data: 'first' },
      { kind: 'output', data: 'second' }
    ])
    expect(reason).toBe('torn')
    // Consumed points at the start of the incomplete frame, so a caller can
    // truncate the file to exactly what was whole and append from there.
    expect(consumed).toBe(whole.length - frameOutput('third').length)
  })

  it('survives a cut inside the frame header itself', () => {
    const whole = log(frameOutput('first'), frameOutput('second'))
    const torn = whole.subarray(0, whole.length - frameOutput('second').length + 3)

    expect(readFrames(torn)).toMatchObject({
      frames: [{ kind: 'output', data: 'first' }],
      reason: 'torn'
    })
  })

  it('refuses a length that runs past the end rather than trusting it', () => {
    // A torn length field can read as an enormous number. Slicing on it would
    // answer with a short buffer whose checksum then fails, reporting corruption
    // where the truth is a tear.
    const buf = log(frameOutput('x'))
    buf.writeUInt32LE(0xffffff, buf.length - frameOutput('x').length + 1)

    expect(readFrames(buf).reason).toBe('torn')
  })
})

describe('a byte that changed', () => {
  it('is caught, and ends the replay there', () => {
    const whole = log(frameOutput('good'), frameOutput('corrupted'), frameOutput('after'))
    const flipped = Buffer.from(whole)
    // Somewhere inside the middle frame's payload.
    const target = writeHeader(1).length + frameOutput('good').length + 9 + 2
    flipped[target] ^= 0x20

    const { frames, reason } = readFrames(flipped)

    expect(frames).toEqual([{ kind: 'output', data: 'good' }])
    expect(reason).toBe('checksum')
  })

  it('does not step over the bad frame to reach the good one after it', () => {
    // The bytes after a frame nobody can vouch for have no established meaning.
    // A slightly stale screen is a small wrong; one assembled from unverified
    // bytes is an unbounded one.
    const whole = log(frameOutput('good'), frameOutput('corrupted'), frameOutput('after'))
    const flipped = Buffer.from(whole)
    flipped[writeHeader(1).length + frameOutput('good').length + 9 + 2] ^= 0x20

    expect(readFrames(flipped).frames).toHaveLength(1)
  })

  it('catches a flip in the length field too', () => {
    const whole = log(frameOutput('hello'), frameOutput('world'))
    const flipped = Buffer.from(whole)
    flipped[writeHeader(1).length + 1] ^= 0x01

    expect(readFrames(flipped).reason).not.toBe('end')
  })
})

describe('a frame this version does not know', () => {
  it('stops rather than skipping into the middle of something', () => {
    const buf = log(frameOutput('known'))
    // A kind from a future version, with a valid length and checksum.
    const alien = Buffer.alloc(9)
    alien.writeUInt8(0x7f, 0)
    alien.writeUInt32LE(0, 1)
    alien.writeUInt32LE(crc32(Buffer.alloc(0)), 5)

    const { frames, reason } = readFrames(Buffer.concat([buf, alien]))

    expect(frames).toEqual([{ kind: 'output', data: 'known' }])
    expect(reason).toBe('unknown-kind')
  })

  it('reports a known kind with the wrong payload size as malformed', () => {
    const bad = Buffer.alloc(9 + 2)
    bad.writeUInt8(0x03, 0) // resize, which needs four bytes
    bad.writeUInt32LE(2, 1)
    bad.writeUInt32LE(crc32(Buffer.alloc(2)), 5)

    expect(readFrames(Buffer.concat([writeHeader(1), bad])).reason).toBe('malformed')
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
