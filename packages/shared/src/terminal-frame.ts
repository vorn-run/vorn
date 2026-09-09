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
