/**
 * A terminal's output between the read that produced it and the flush that
 * sends it, held as the chunks node-pty handed over.
 *
 * A list rather than one string grown by `+=`, because a flush now takes at
 * most `MAX_FLUSH_UNITS` from the front and leaves the rest. Slicing the front
 * off a string that keeps growing at the back flattens the whole remainder on
 * every flush, which is quadratic in the backlog -- and a backlog is exactly
 * what a burst is.
 */
export interface HeldOutput {
  /** Held from `head` on; the ones before it are taken and wait to be dropped. */
  chunks: string[]
  head: number
  /** UTF-16 units held: what `String.length` counts. */
  units: number
}

/**
 * The most one flush carries, in UTF-16 units.
 *
 * 64 KB of terminal output, which is nearly all ASCII; the Terminal State
 * Protocol's flush cap. Everything a flush does on the event loop scales with
 * its size -- framing it for each client, analysing it, handing it to the
 * screen model and the history -- so this is what bounds how long one flush
 * can hold the loop, whatever a program prints in one go. Units rather than
 * bytes because counting bytes means encoding, and the point is to not pay for
 * the whole of a burst at once.
 */
export const MAX_FLUSH_UNITS = 64 * 1024

export function holdOutput(held: HeldOutput | undefined, data: string): HeldOutput {
  if (!held) return { chunks: [data], head: 0, units: data.length }
  held.chunks.push(data)
  held.units += data.length
  return held
}

/**
 * Take up to `cap` units from the front, leaving the rest held.
 *
 * Never between the two halves of a surrogate pair: a flush ending on a lone
 * high surrogate would be encoded as U+FFFD by every consumer, and the next
 * flush would start with another. A cap too small to hold the pair takes the
 * pair anyway, one unit over, so a caller always makes progress.
 */
export function takeOutput(held: HeldOutput, cap: number): string {
  const { chunks } = held
  if (held.units <= cap) {
    const all =
      chunks.length - held.head === 1 ? chunks[held.head]! : chunks.slice(held.head).join('')
    held.chunks = []
    held.head = 0
    held.units = 0
    return all
  }

  // Taken chunks are passed by moving `head`, not by slicing the array: a
  // backlog of thousands of reads taken a flush at a time would otherwise
  // copy what is left behind on every take.
  const taken: string[] = []
  let units = 0
  while (held.head < chunks.length && units + chunks[held.head]!.length <= cap) {
    units += chunks[held.head]!.length
    taken.push(chunks[held.head]!)
    held.head += 1
  }

  const first = chunks[held.head]
  if (first !== undefined) {
    let cut = cap - units
    if (cut > 0 && isHighSurrogate(first.charCodeAt(cut - 1))) cut -= 1
    // Something must move, or a caller looping on this never finishes: a
    // whole pair, one unit over the cap, rather than half of one.
    if (!taken.length && cut === 0) cut = Math.min(2, first.length)
    if (cut > 0) {
      taken.push(first.slice(0, cut))
      units += cut
      if (cut < first.length) chunks[held.head] = first.slice(cut)
      else held.head += 1
    }
  }

  held.units -= units
  // Dropped once they are most of the array, so each chunk is copied a
  // bounded number of times however the backlog is taken.
  if (held.head > 1024 && held.head * 2 > chunks.length) {
    held.chunks = chunks.slice(held.head)
    held.head = 0
  }
  return taken.join('')
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff
}
