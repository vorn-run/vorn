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
 *              u32le epoch  u64le nextRseq  u64le nextOffset      (version 2)
 *     frame  = u8 kind  u32le payloadLength  u32le crc32  payload
 *     payload = u64le rseq  u64le startOffset  body
 *                0x10 data    u8 stream  bytes
 *                0x11 resize  u16le cols  u16le rows  u16le pxWidth  u16le pxHeight
 *
 * Version 2 is the Session Recovery Contract's record log. Every record carries
 * its place: `rseq` numbers it within the session and `startOffset` counts the
 * bytes of output before it, so a checkpoint, a log and a client all name a
 * point in the stream the same way (a `RecordCursor`, the first record and byte
 * *not* included). The header says where this log starts, and the writer never
 * appends a record below what it already holds, so writing a record twice is a
 * no-op rather than a duplicate. The contract's Gap and Exit records are later
 * kinds: nothing writes them yet, and a reader that meets a kind it does not
 * know stops there, which is the safe answer for a newer file.
 *
 * Version 1 had no positions -- a batch marker per flush, then output and
 * resize frames -- and is still read, so the first start after an update can
 * restore what the previous build wrote. Nothing writes it any more.
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

import type { RecordCursor } from '@vornrun/shared/types'

export const MAGIC = 'VRNL'
export const FORMAT_VERSION = 2
const LEGACY_VERSION = 1

const LEGACY_HEADER_BYTES = 4 + 1 + 4
const HEADER_BYTES = LEGACY_HEADER_BYTES + 4 + 8 + 8
const FRAME_PREFIX_BYTES = 1 + 4 + 4
const RECORD_HEADER_BYTES = 8 + 8

export const FrameKind = {
  Data: 0x10,
  Resize: 0x11
} as const

/** Version 1's kinds, read and never written. */
const LegacyKind = {
  Batch: 0x01,
  Output: 0x02,
  Resize: 0x03
} as const

/** Where one record sits: its number, and the bytes of output before it. */
export interface RecordHeader {
  rseq: number
  startOffset: number
}

/** Which of a process's outputs a Data record came from. A PTY has one. */
export const Stream = { Pty: 0, Stdout: 1, Stderr: 2 } as const

export type LogRecord =
  | (RecordHeader & { kind: 'data'; stream: number; data: string })
  | (RecordHeader & {
      kind: 'resize'
      cols: number
      rows: number
      pxWidth: number
      pxHeight: number
    })

/** Where a record leaves the stream: the cursor of a state that includes it. */
export function cursorAfter(epoch: number, record: LogRecord): RecordCursor {
  const bytes = record.kind === 'data' ? Buffer.byteLength(record.data, 'utf-8') : 0
  return { epoch, nextRseq: record.rseq + 1, nextOffset: record.startOffset + bytes }
}

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

/** A log that starts at `start`: every record in it is at or after that cursor. */
export function writeHeader(generation: number, start: RecordCursor): Buffer {
  const buf = Buffer.alloc(HEADER_BYTES)
  buf.write(MAGIC, 0, 'ascii')
  buf.writeUInt8(FORMAT_VERSION, 4)
  buf.writeUInt32LE(generation >>> 0, 5)
  buf.writeUInt32LE(start.epoch >>> 0, 9)
  buf.writeBigUInt64LE(BigInt(start.nextRseq), 13)
  buf.writeBigUInt64LE(BigInt(start.nextOffset), 21)
  return buf
}

export interface Header {
  formatVersion: number
  generation: number
  /** Where the log starts. Absent in a version 1 log, which had no positions. */
  start?: RecordCursor
  /** Bytes the header takes, where the first frame begins. */
  bytes: number
}

/**
 * Read the header, or say why not.
 *
 * Null rather than a throw: a file that is absent, empty, truncated to nothing
 * or written by a different version is an ordinary thing to find on disk after a
 * crash, and the caller's answer to all of them is the same — start again.
 */
export function readHeader(buf: Buffer): Header | null {
  if (buf.length < LEGACY_HEADER_BYTES) return null
  if (buf.subarray(0, 4).toString('ascii') !== MAGIC) return null
  // The version was written, returned, and never consulted, while the comment
  // above claimed a file from a different version was one of the cases this
  // covers. Refusing it is the conservative half of that promise: a reader that
  // does not know a layout should start again rather than guess at it.
  const formatVersion = buf.readUInt8(4)
  const generation = buf.readUInt32LE(5)
  if (formatVersion === LEGACY_VERSION) {
    return { formatVersion, generation, bytes: LEGACY_HEADER_BYTES }
  }
  if (formatVersion !== FORMAT_VERSION || buf.length < HEADER_BYTES) return null
  const start = {
    epoch: buf.readUInt32LE(9),
    nextRseq: Number(buf.readBigUInt64LE(13)),
    nextOffset: Number(buf.readBigUInt64LE(21))
  }
  return { formatVersion, generation, start, bytes: HEADER_BYTES }
}

function frame(kind: number, payload: Buffer): Buffer {
  const buf = Buffer.alloc(FRAME_PREFIX_BYTES + payload.length)
  buf.writeUInt8(kind, 0)
  buf.writeUInt32LE(payload.length, 1)
  buf.writeUInt32LE(crc32(payload), 5)
  payload.copy(buf, FRAME_PREFIX_BYTES)
  return buf
}

function recordPayload(at: RecordHeader, bodyBytes: number): Buffer {
  const payload = Buffer.alloc(RECORD_HEADER_BYTES + bodyBytes)
  payload.writeBigUInt64LE(BigInt(at.rseq), 0)
  payload.writeBigUInt64LE(BigInt(at.startOffset), 8)
  return payload
}

/** Output, already encoded: the caller measured these bytes to number the next record. */
export function frameData(at: RecordHeader, bytes: Buffer, stream: number = Stream.Pty): Buffer {
  const payload = recordPayload(at, 1 + bytes.length)
  payload.writeUInt8(stream, RECORD_HEADER_BYTES)
  bytes.copy(payload, RECORD_HEADER_BYTES + 1)
  return frame(FrameKind.Data, payload)
}

export function frameResize(at: RecordHeader, cols: number, rows: number): Buffer {
  const payload = recordPayload(at, 8)
  payload.writeUInt16LE(cols & 0xffff, RECORD_HEADER_BYTES)
  payload.writeUInt16LE(rows & 0xffff, RECORD_HEADER_BYTES + 2)
  return frame(FrameKind.Resize, payload)
}

/** Why a read stopped where it did. `end` is the only one that is not damage. */
export type StopReason = 'end' | 'torn' | 'checksum' | 'unknown-kind' | 'malformed'

export interface ReadResult {
  records: LogRecord[]
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
export function readFrames(buf: Buffer, header: Header): ReadResult {
  const records: LogRecord[] = []
  const legacy = header.formatVersion === LEGACY_VERSION ? { rseq: 0, offset: 0 } : null
  let at = header.bytes

  for (;;) {
    if (at === buf.length) return { records, consumed: at, reason: 'end' }
    if (at + FRAME_PREFIX_BYTES > buf.length) return { records, consumed: at, reason: 'torn' }

    const kind = buf.readUInt8(at)
    const length = buf.readUInt32LE(at + 1)
    const expected = buf.readUInt32LE(at + 5)
    const start = at + FRAME_PREFIX_BYTES

    // Checked before it is used as a slice bound: a torn length field can read
    // as an enormous number, and `subarray` would answer with a short buffer
    // whose checksum then fails for the wrong reason.
    if (start + length > buf.length) return { records, consumed: at, reason: 'torn' }

    const payload = buf.subarray(start, start + length)
    if (crc32(payload) !== expected) return { records, consumed: at, reason: 'checksum' }

    const decoded = legacy ? decodeLegacy(kind, payload, legacy) : decode(kind, payload)
    if (decoded === null) {
      const known = legacy ? LEGACY_KNOWN : KNOWN
      return { records, consumed: at, reason: known.has(kind) ? 'malformed' : 'unknown-kind' }
    }

    if (decoded !== SKIP) records.push(decoded)
    at = start + length
  }
}

/**
 * Derived rather than written out again. A kind added above and forgotten here
 * would be reported as `unknown-kind` -- the right answer by accident before it
 * is implemented, and the wrong one afterwards.
 */
const KNOWN = new Set<number>(Object.values(FrameKind))
const LEGACY_KNOWN = new Set<number>(Object.values(LegacyKind))

/** A frame that is whole and understood but is not a record: version 1's batch marker. */
const SKIP = Symbol('skip')

function decode(kind: number, payload: Buffer): LogRecord | null {
  if (payload.length < RECORD_HEADER_BYTES) return null
  const rseq = Number(payload.readBigUInt64LE(0))
  const startOffset = Number(payload.readBigUInt64LE(8))
  const body = payload.subarray(RECORD_HEADER_BYTES)
  switch (kind) {
    case FrameKind.Data:
      return body.length >= 1
        ? {
            kind: 'data',
            rseq,
            startOffset,
            stream: body.readUInt8(0),
            data: body.subarray(1).toString('utf-8')
          }
        : null
    case FrameKind.Resize:
      return body.length === 8
        ? {
            kind: 'resize',
            rseq,
            startOffset,
            cols: body.readUInt16LE(0),
            rows: body.readUInt16LE(2),
            pxWidth: body.readUInt16LE(4),
            pxHeight: body.readUInt16LE(6)
          }
        : null
    default:
      return null
  }
}

/**
 * A version 1 frame as a record, numbered in the order read.
 *
 * Its positions are made up -- the file never had any -- so they mean
 * something only within this one replay, which is all a version 1 log is
 * ever used for.
 */
function decodeLegacy(
  kind: number,
  payload: Buffer,
  next: { rseq: number; offset: number }
): LogRecord | typeof SKIP | null {
  switch (kind) {
    case LegacyKind.Batch:
      return payload.length === 4 ? SKIP : null
    case LegacyKind.Output: {
      const record: LogRecord = {
        kind: 'data',
        rseq: next.rseq++,
        startOffset: next.offset,
        stream: Stream.Pty,
        data: payload.toString('utf-8')
      }
      next.offset += payload.length
      return record
    }
    case LegacyKind.Resize:
      return payload.length === 4
        ? {
            kind: 'resize',
            rseq: next.rseq++,
            startOffset: next.offset,
            cols: payload.readUInt16LE(0),
            rows: payload.readUInt16LE(2),
            pxWidth: 0,
            pxHeight: 0
          }
        : null
    default:
      return null
  }
}
