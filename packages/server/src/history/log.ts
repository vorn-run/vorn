/**
 * The on-disk shape of a terminal's history, and nothing else.
 *
 * Pure functions over buffers: no files, no sessions, no timers. Everything that
 * can go wrong with this format can go wrong in a unit test, which matters more
 * here than usual because every failure mode is a *crash* — the input this reader
 * is designed for is a file that was being written when the process died.
 *
 * ## Why frames rather than raw bytes
 *
 * A crash tears the final append. Raw output would leave a reader no way to know
 * where the good part ends, and feeding an emulator half an escape sequence is
 * worse than feeding it nothing: it either swallows the text that follows or
 * prints it as characters. A length prefix makes the tear detectable, so replay
 * stops at the last whole frame. It is the same rule the in-memory byte buffer
 * already trims by — a boundary, never an arbitrary cut.
 *
 * ## Why a checksum as well as a length
 *
 * A length catches a torn tail. It does not catch a byte that changed in the
 * middle of a file that is otherwise the right size, and a wrong byte inside an
 * escape sequence is exactly the input that makes a terminal do something
 * inexplicable three screens later. SQLite's WAL and etcd's log both checksum
 * per record for this reason, and it costs a table lookup per byte.
 *
 * ## Layout
 *
 *     header = 'VRNL'  u8 formatVersion  u32le generation
 *     frame  = u8 kind  u32le payloadLength  u32le crc32  payload
 *                0x01 batch   u32le seq
 *                0x02 output  utf8 bytes
 *                0x03 resize  u16le cols  u16le rows
 *
 * There is no reset kind. One was drafted and removed: nothing in the server
 * clears a live terminal's history -- the only path that empties it is a PTY
 * exit, which deletes the files outright -- so it would have been a kind with a
 * reader and no writer, and an unreachable branch in replay. `formatVersion`
 * exists to add one the day something needs it.
 *
 * The generation ties a log to the checkpoint it was written for. A log found
 * beside a newer checkpoint is not a log of what happened after it, and replaying
 * one over the other would produce a screen that never existed.
 */

import zlib from 'zlib'

export const MAGIC = 'VRNL'
export const FORMAT_VERSION = 1

const HEADER_BYTES = 4 + 1 + 4
const FRAME_PREFIX_BYTES = 1 + 4 + 4

export const FrameKind = {
  Batch: 0x01,
  Output: 0x02,
  Resize: 0x03
} as const

export type Frame =
  | { kind: 'batch'; seq: number }
  | { kind: 'output'; data: string }
  | { kind: 'resize'; cols: number; rows: number }

/**
 * CRC-32, the IEEE polynomial every other implementation of this uses.
 *
 * Node's own, which has been in the standard library since 22.2 and is native.
 * An earlier version of this file wrote the table out by hand on the grounds
 * that the repo has no checksum anywhere and fifteen lines beats a dependency --
 * true, and it missed that this needs neither. Measured at about a hundred times
 * the throughput of the table-driven loop, on a function that runs once per
 * frame written and once per frame replayed.
 *
 * Wrapped rather than re-exported so the name stays this module's, and so the
 * tests that pin the published check value keep testing what the format
 * actually uses.
 */
export function crc32(bytes: Buffer): number {
  return zlib.crc32(bytes)
}

export function writeHeader(generation: number): Buffer {
  const buf = Buffer.alloc(HEADER_BYTES)
  buf.write(MAGIC, 0, 'ascii')
  buf.writeUInt8(FORMAT_VERSION, 4)
  buf.writeUInt32LE(generation >>> 0, 5)
  return buf
}

export interface Header {
  formatVersion: number
  generation: number
}

/**
 * Read the header, or say why not.
 *
 * Null rather than a throw: a file that is absent, empty, truncated to nothing
 * or written by a different version is an ordinary thing to find on disk after a
 * crash, and the caller's answer to all of them is the same — start again.
 */
export function readHeader(buf: Buffer): Header | null {
  if (buf.length < HEADER_BYTES) return null
  if (buf.subarray(0, 4).toString('ascii') !== MAGIC) return null
  // The version was written, returned, and never consulted, while the comment
  // above claimed a file from a different version was one of the cases this
  // covers. Refusing it is the conservative half of that promise: a reader that
  // does not know a layout should start again rather than guess at it.
  const formatVersion = buf.readUInt8(4)
  if (formatVersion !== FORMAT_VERSION) return null
  return { formatVersion, generation: buf.readUInt32LE(5) }
}

function frame(kind: number, payload: Buffer): Buffer {
  const buf = Buffer.alloc(FRAME_PREFIX_BYTES + payload.length)
  buf.writeUInt8(kind, 0)
  buf.writeUInt32LE(payload.length, 1)
  buf.writeUInt32LE(crc32(payload), 5)
  payload.copy(buf, FRAME_PREFIX_BYTES)
  return buf
}

export function frameBatch(seq: number): Buffer {
  const payload = Buffer.alloc(4)
  payload.writeUInt32LE(seq >>> 0, 0)
  return frame(FrameKind.Batch, payload)
}

export function frameOutput(data: string): Buffer {
  return frame(FrameKind.Output, Buffer.from(data, 'utf-8'))
}

export function frameResize(cols: number, rows: number): Buffer {
  const payload = Buffer.alloc(4)
  payload.writeUInt16LE(cols & 0xffff, 0)
  payload.writeUInt16LE(rows & 0xffff, 2)
  return frame(FrameKind.Resize, payload)
}

/** Why a read stopped where it did. `end` is the only one that is not damage. */
export type StopReason = 'end' | 'torn' | 'checksum' | 'unknown-kind' | 'malformed'

export interface ReadResult {
  frames: Frame[]
  /** Bytes consumed, so a caller can truncate the file to what was whole. */
  consumed: number
  reason: StopReason
}

/**
 * Read frames until one is not whole.
 *
 * Never throws, and never skips. A frame that fails its checksum ends the read
 * rather than being stepped over, because a file with one bad frame is a file
 * whose remaining bytes have no established meaning — the prefix is trustworthy
 * and the rest is a guess. A slightly stale screen is a small wrong; a screen
 * assembled from bytes nobody can vouch for is an unbounded one.
 */
export function readFrames(buf: Buffer, from = HEADER_BYTES): ReadResult {
  const frames: Frame[] = []
  let at = from

  for (;;) {
    if (at === buf.length) return { frames, consumed: at, reason: 'end' }
    if (at + FRAME_PREFIX_BYTES > buf.length) return { frames, consumed: at, reason: 'torn' }

    const kind = buf.readUInt8(at)
    const length = buf.readUInt32LE(at + 1)
    const expected = buf.readUInt32LE(at + 5)
    const start = at + FRAME_PREFIX_BYTES

    // Checked before it is used as a slice bound: a torn length field can read
    // as an enormous number, and `subarray` would answer with a short buffer
    // whose checksum then fails for the wrong reason.
    if (start + length > buf.length) return { frames, consumed: at, reason: 'torn' }

    const payload = buf.subarray(start, start + length)
    if (crc32(payload) !== expected) return { frames, consumed: at, reason: 'checksum' }

    const decoded = decode(kind, payload)
    if (!decoded) {
      return { frames, consumed: at, reason: KNOWN.has(kind) ? 'malformed' : 'unknown-kind' }
    }

    frames.push(decoded)
    at = start + length
  }
}

/**
 * Derived rather than written out again. A kind added above and forgotten here
 * would be reported as `unknown-kind` -- the right answer by accident before it
 * is implemented, and the wrong one afterwards.
 */
const KNOWN = new Set<number>(Object.values(FrameKind))

function decode(kind: number, payload: Buffer): Frame | null {
  switch (kind) {
    case FrameKind.Batch:
      return payload.length === 4 ? { kind: 'batch', seq: payload.readUInt32LE(0) } : null
    case FrameKind.Output:
      return { kind: 'output', data: payload.toString('utf-8') }
    case FrameKind.Resize:
      return payload.length === 4
        ? { kind: 'resize', cols: payload.readUInt16LE(0), rows: payload.readUInt16LE(2) }
        : null
    default:
      return null
  }
}
