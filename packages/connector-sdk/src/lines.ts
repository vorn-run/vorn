import { MAX_FRAME_BYTES } from './protocol'

/** Split a byte stream into lines, decoding each only once it is whole; `overflow` fires past the frame limit. */
export function lineReader(
  onLine: (line: string) => void,
  overflow: () => void
): (chunk: Buffer) => void {
  let pending: Buffer[] = []
  let size = 0
  return (chunk) => {
    let start = 0
    for (let end = chunk.indexOf(0x0a); end !== -1; end = chunk.indexOf(0x0a, start)) {
      const part = chunk.subarray(start, end)
      if (size + part.length > MAX_FRAME_BYTES) return overflow()
      const whole =
        pending.length === 0 ? part : Buffer.concat([...pending, part], size + part.length)
      const line = whole.toString('utf8')
      pending = []
      size = 0
      start = end + 1
      onLine(line.endsWith('\r') ? line.slice(0, -1) : line)
    }
    const rest = chunk.subarray(start)
    size += rest.length
    if (size > MAX_FRAME_BYTES) return overflow()
    if (rest.length > 0) pending.push(rest)
  }
}
