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
  chunks: string[]
  /** UTF-16 units across `chunks`: what `String.length` counts. */
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
  if (!held) return { chunks: [data], units: data.length }
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
  if (held.units <= cap) {
    const all = held.chunks.length === 1 ? held.chunks[0]! : held.chunks.join('')
    held.chunks = []
    held.units = 0
    return all
  }

  const taken: string[] = []
  let units = 0
  let used = 0
  while (used < held.chunks.length && units + held.chunks[used]!.length <= cap) {
    units += held.chunks[used]!.length
    taken.push(held.chunks[used]!)
    used += 1
  }

  let rest = held.chunks.slice(used)
  const first = rest[0]
  if (first !== undefined) {
    let cut = cap - units
    if (cut > 0 && isHighSurrogate(first.charCodeAt(cut - 1))) cut -= 1
    // Something must move, or a caller looping on this never finishes: a
    // whole pair, one unit over the cap, rather than half of one.
    if (!taken.length && cut === 0) cut = Math.min(2, first.length)
    if (cut > 0) {
      taken.push(first.slice(0, cut))
      units += cut
      const tail = first.slice(cut)
      rest = tail ? [tail, ...rest.slice(1)] : rest.slice(1)
    }
  }

  held.chunks = rest
  held.units -= units
  return taken.join('')
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff
}
