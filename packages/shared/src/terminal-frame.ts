/**
 * Terminal output as bytes on the wire.
 *
 * `terminal:data` used to travel as a JSON notification, which meant every
 * escape byte became six characters of `\u001b`, coloured output grew by two
 * thirds, and both ends paid an encode and a decode for a string the terminal
 * emulator would only turn back into bytes. A client that asks for bytes gets
 * this frame instead: a fixed header, then the output exactly as the process
 * wrote it, which xterm takes as it is.
 *
 * Layout, big-endian:
 *
 *   0      version, always 1
 *   1..4   seq
 *   5      length of the id in bytes
 *   6..    the id, then the output
 */

const VERSION = 1
const HEADER = 6

export interface TerminalFrame {
  id: string
  seq: number
  data: Uint8Array
}

const encoder = new TextEncoder()

/** Ids are UUIDs, and a longer one would not fit the byte that carries its length. */
export const MAX_FRAME_ID_BYTES = 255

export function encodeTerminalFrame(frame: TerminalFrame): Uint8Array {
  const id = encoder.encode(frame.id)
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

const decoder = new TextDecoder()

/**
 * The frame, or null for bytes that are not one.
 *
 * The output is copied out rather than viewed: a socket hands over slices of a
 * pooled buffer, and a view handed across a process boundary is cloned with
 * everything it sits in.
 */
export function decodeTerminalFrame(bytes: Uint8Array): TerminalFrame | null {
  if (bytes.length < HEADER || bytes[0] !== VERSION) return null
  const idLength = bytes[5]
  if (idLength === 0 || bytes.length < HEADER + idLength) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return {
    id: decoder.decode(bytes.subarray(HEADER, HEADER + idLength)),
    seq: view.getUint32(1),
    // `new Uint8Array(view)` copies, and comes back as a Uint8Array even from a Buffer.
    data: new Uint8Array(bytes.subarray(HEADER + idLength))
  }
}
