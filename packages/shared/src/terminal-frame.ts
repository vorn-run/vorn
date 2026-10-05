import type { TerminalData } from './protocol'

// Terminal output on the wire as bytes: [1 version][4 seq, big-endian][1 id length][id][output].
const VERSION = 1
const HEADER = 6

export type TerminalFrame = Omit<TerminalData, 'data'> & { data: Uint8Array }

/** Ids are UUIDs, and a longer one would not fit the byte that carries its length. */
export const MAX_FRAME_ID_BYTES = 255

const encoder = new TextEncoder()
const decoder = new TextDecoder()
// The same id arrives flush after flush; encoding it once per id is enough.
let lastId = ''
let lastIdBytes = new Uint8Array()

export function encodeTerminalFrame(frame: TerminalFrame): Uint8Array {
  if (frame.id !== lastId) {
    lastIdBytes = encoder.encode(frame.id)
    lastId = frame.id
  }
  const id = lastIdBytes
  if (id.length === 0 || id.length > MAX_FRAME_ID_BYTES) {
    throw new Error(`terminal frame id must be 1..${MAX_FRAME_ID_BYTES} bytes`)
  }
  const out = new Uint8Array(HEADER + id.length + frame.data.length)
  const view = new DataView(out.buffer)
  out[0] = VERSION
  view.setUint32(1, frame.seq >>> 0)
  out[5] = id.length
  out.set(id, HEADER)
  out.set(frame.data, HEADER + id.length)
  return out
}

/** The frame, or null for bytes that are not one. */
export function decodeTerminalFrame(bytes: Uint8Array): TerminalFrame | null {
  if (bytes.length < HEADER || bytes[0] !== VERSION) return null
  const idLength = bytes[5]
  if (idLength === 0 || bytes.length < HEADER + idLength) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const output = bytes.subarray(HEADER + idLength)
  // A socket in Node hands over a slice of a pooled buffer; a view of that crosses to the renderer with the pool attached.
  const pooled = bytes.byteOffset !== 0 || bytes.byteLength !== bytes.buffer.byteLength
  return {
    id: decoder.decode(bytes.subarray(HEADER, HEADER + idLength)),
    seq: view.getUint32(1),
    data: pooled ? new Uint8Array(output) : output
  }
}

/**
 * Version 2, sent by vornd for the sessions it holds: the records a frame
 * carries, so a client always knows its full cursor (the Terminal State
 * Protocol's bytes frame). Every integer big-endian; each u64 is two u32
 * halves, exact up to 2^53.
 *
 * `[2][1 id length][id][4 epoch][8 first rseq][8 last rseq][8 start offset][output]`
 *
 * The output is data records `firstRseq..=lastRseq`, its first byte at
 * `startOffset`. A resize between two frames arrives as `terminal:resized`, in
 * order with them.
 */
export interface TerminalFrameV2 {
  id: string
  epoch: number
  firstRseq: number
  lastRseq: number
  startOffset: number
  data: Uint8Array
}

const V2 = 2
/** The version byte and the id length. */
const V2_LEAD = 2
/** Epoch and three u64s, after the id. */
const V2_FIXED = 4 + 3 * 8
const TWO_32 = 0x1_0000_0000

/** The version a binary terminal frame says it is, or null for no bytes. */
export function terminalFrameVersion(bytes: Uint8Array): number | null {
  return bytes.length > 0 ? bytes[0] : null
}

function readU64(view: DataView, at: number): number {
  return view.getUint32(at) * TWO_32 + view.getUint32(at + 4)
}

function writeU64(view: DataView, at: number, value: number): void {
  view.setUint32(at, Math.floor(value / TWO_32))
  view.setUint32(at + 4, value >>> 0)
}

/** A version 2 frame, or null for bytes that are not one. */
export function decodeTerminalFrameV2(bytes: Uint8Array): TerminalFrameV2 | null {
  if (bytes.length < V2_LEAD || bytes[0] !== V2) return null
  const idLength = bytes[1]
  if (idLength === 0 || bytes.length < V2_LEAD + idLength + V2_FIXED) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const at = V2_LEAD + idLength
  const firstRseq = readU64(view, at + 4)
  const lastRseq = readU64(view, at + 12)
  if (lastRseq < firstRseq) return null
  const output = bytes.subarray(at + V2_FIXED)
  const pooled = bytes.byteOffset !== 0 || bytes.byteLength !== bytes.buffer.byteLength
  return {
    id: decoder.decode(bytes.subarray(V2_LEAD, at)),
    epoch: view.getUint32(at),
    firstRseq,
    lastRseq,
    startOffset: readU64(view, at + 20),
    data: pooled ? new Uint8Array(output) : output
  }
}

/** Writes a version 2 frame; vornd writes them, tests read back what they wrote. */
export function encodeTerminalFrameV2(frame: TerminalFrameV2): Uint8Array {
  const id = encoder.encode(frame.id)
  if (id.length === 0 || id.length > MAX_FRAME_ID_BYTES) {
    throw new Error(`terminal frame id must be 1..${MAX_FRAME_ID_BYTES} bytes`)
  }
  const out = new Uint8Array(V2_LEAD + id.length + V2_FIXED + frame.data.length)
  const view = new DataView(out.buffer)
  out[0] = V2
  out[1] = id.length
  out.set(id, V2_LEAD)
  const at = V2_LEAD + id.length
  view.setUint32(at, frame.epoch >>> 0)
  writeU64(view, at + 4, frame.firstRseq)
  writeU64(view, at + 12, frame.lastRseq)
  writeU64(view, at + 20, frame.startOffset)
  out.set(frame.data, at + V2_FIXED)
  return out
}

/** The cursor of a client that has applied this frame: the first record and byte it does not include. */
export function frameResume(frame: TerminalFrameV2): {
  epoch: number
  nextRseq: number
  nextOffset: number
} {
  return {
    epoch: frame.epoch,
    nextRseq: frame.lastRseq + 1,
    nextOffset: frame.startOffset + frame.data.length
  }
}
