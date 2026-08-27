/**
 * The bytes a terminal emitted, kept as they were emitted.
 *
 * Separate from the line buffer in `pty-manager`, which stores the same output
 * with every escape sequence removed. That buffer answers "what did this agent
 * say", and stripping is what makes it answerable. This one answers "what should
 * a terminal draw", and stripping destroys exactly the information needed:
 * colour, cursor movement, the alternate screen, the repaint a TUI performs on
 * every keystroke.
 *
 * A client attaching to a live session gets this first, feeds it to its terminal
 * emulator, and only then starts applying live output. Without it, attaching
 * shows a blank screen until the program next decides to redraw, which for an
 * idle agent may be never.
 */

/**
 * How much to keep per terminal.
 *
 * Enough to redraw a full-screen application several times over, and small
 * enough that a hundred idle sessions do not add up to something worth
 * worrying about. A length, not a line count, because the cost being bounded is
 * memory and the thing being stored is a stream.
 *
 * Counted in UTF-16 code units — what `String.length` returns — rather than
 * encoded bytes. For ASCII, which terminal output overwhelmingly is, they are
 * the same; CJK runs at three bytes per unit, so a buffer of entirely CJK output
 * costs about three times this. That is an acceptable ceiling, and measuring it
 * exactly would mean encoding every write to count it.
 */
const MAX_UNITS = 256 * 1024

/**
 * Trim from the front, at a line boundary where there is one nearby.
 *
 * Cutting mid-sequence would hand the client half an escape sequence, and a
 * terminal emulator fed a truncated sequence will either swallow the text that
 * follows it or render it as literal characters. A newline is a safe cut: no
 * escape sequence spans one.
 *
 * When there is no newline in the trimmed region — a single enormous line, which
 * a progress bar redrawing without newlines produces — the cut is taken as-is.
 * Losing the head of one line is better than growing without bound.
 */
function trim(data: string): string {
  if (data.length <= MAX_UNITS) return data
  const cut = data.length - MAX_UNITS
  const boundary = data.indexOf('\n', cut)
  return boundary === -1 ? data.slice(cut) : data.slice(boundary + 1)
}

/**
 * Chunks as they arrived, joined only when somebody reads.
 *
 * This used to be one string per terminal, re-formed on every append:
 * `set(id, trim(get(id) + data))`. Once a buffer reached its cap that was two
 * ~256 KB allocations per PTY chunk -- one to concatenate, one to slice -- and
 * a busy agent produces chunks by the hundred per second. It was the most
 * expensive thing on the hottest path in the server, for a value almost nothing
 * reads.
 *
 * Appending is now a push, and the cost moves to `readScrollback`, which is
 * where it belongs: reads are rare and deliberate, writes are constant and
 * incidental. The running total is kept so the bound can be enforced without
 * measuring the whole list.
 *
 * The trim itself is unchanged, including where it cuts. It runs against the
 * joined result rather than per chunk, because a boundary can only be found in
 * the text either side of it -- trimming chunk by chunk would cut at whatever
 * edge a PTY write happened to land on, which is precisely the mid-sequence cut
 * the boundary rule exists to avoid.
 */
interface Buffered {
  chunks: string[]
  units: number
}

const buffers = new Map<string, Buffered>()

export function appendScrollback(id: string, data: string): void {
  const held = buffers.get(id)
  if (!held) {
    buffers.set(id, { chunks: [data], units: data.length })
    return
  }

  held.chunks.push(data)
  held.units += data.length

  // Compacted only when there is enough overshoot to be worth the join --
  // otherwise a terminal sitting exactly at the cap would re-join on every
  // chunk, which is the behaviour this replaced. The slack is bounded, so the
  // real ceiling is `MAX_UNITS + COMPACT_SLACK` rather than `MAX_UNITS`.
  if (held.units > MAX_UNITS + COMPACT_SLACK) compact(held)
}

/**
 * How far a buffer may run past its cap before it is re-formed.
 *
 * A quarter of the cap: large enough that compaction is rare against a stream of
 * small writes, small enough that the overshoot is a rounding error against the
 * memory this bounds.
 */
const COMPACT_SLACK = MAX_UNITS / 4

function compact(held: Buffered): void {
  const trimmed = trim(held.chunks.join(''))
  held.chunks = [trimmed]
  held.units = trimmed.length
}

export function readScrollback(id: string): string {
  const held = buffers.get(id)
  if (!held) return ''
  // Compacted on the way out rather than joined and thrown away: a caller that
  // reads twice should not pay twice, and the result is the same bytes either
  // way.
  compact(held)
  return held.chunks[0] ?? ''
}

export function clearScrollback(id: string): void {
  buffers.delete(id)
}

/** Test-only, mirroring the map this module used to expose implicitly. */
export function resetScrollback(): void {
  buffers.clear()
}
