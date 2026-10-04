import { pipelineFor } from './core-pipeline'

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
 * Chunks as they arrived, joined only when somebody reads.
 *
 * This used to be one string per terminal, re-formed on every append:
 * `set(id, trim(get(id) + data))`. Once a buffer reached its cap that was two
 * ~256 KB allocations per write -- one to concatenate, one to slice -- and it
 * was fed straight from `onData`, which a busy agent drives by the hundred per
 * second. It was the most expensive thing on the hottest path in the server, for
 * a value almost nothing reads.
 *
 * It is fed from the coalesced flush now rather than from `onData`, which cuts
 * the number of writes again and, more importantly, keeps it in step with the
 * screen model. The cost that remains is per byte rather than per write, so the
 * chunk list still earns its place.
 *
 * Appending is now a push, and the cost moves to `readScrollback`, which is
 * where it belongs: reads are rare and deliberate, writes are constant and
 * incidental. The running total is kept so the bound can be enforced without
 * measuring the whole list.
 *
 * Where the trim cuts is unchanged. It looks for its boundary across chunks
 * rather than within one, because a boundary can only be found in the text
 * either side of it -- trimming chunk by chunk would cut at whatever edge a PTY
 * write happened to land on, which is precisely the mid-sequence cut the
 * boundary rule exists to avoid.
 */
interface Buffered {
  chunks: string[]
  units: number
}

const buffers = new Map<string, Buffered>()

// A terminal on a core thread keeps its scrollback there, fed by the same
// hand-off as its screen; the functions below answer from it when it does.

export function appendScrollback(id: string, data: string): void {
  const pipeline = pipelineFor(id)
  if (pipeline) {
    try {
      pipeline.appendScrollback(data)
      return
    } catch {
      // Kept here, and handed to the next pipeline for this id.
    }
  }
  let held = buffers.get(id)
  if (!held) {
    held = { chunks: [], units: 0 }
    buffers.set(id, held)
  }

  held.chunks.push(data)
  held.units += data.length

  // Compacted only when there is enough overshoot to be worth the join --
  // otherwise a terminal sitting exactly at the cap would re-join on every
  // chunk, which is the behaviour this replaced. The slack is bounded, so the
  // real ceiling is `MAX_UNITS + COMPACT_SLACK` rather than `MAX_UNITS`.
  //
  // The first chunk goes through this too. An earlier version seeded the buffer
  // and returned, so a single oversized write -- a `cat` of something large,
  // arriving before anything else -- sat unbounded until the next append or
  // read, which for a terminal that then goes quiet is indefinitely.
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
 *
 * The cut is the first newline at or after the point that leaves `MAX_UNITS`,
 * or that point itself when no newline follows it -- the same cut, to the
 * unit, as trimming the joined text would make. It is found by
 * dropping the chunks wholly before the point and searching from there, so a
 * compaction costs the chunk it lands in rather than a copy of the whole
 * buffer. Joining here was a quarter-megabyte copy every 64 KB a busy terminal
 * printed, on the event loop, in the middle of a burst.
 */
function compact(held: Buffered): void {
  const cut = held.units - MAX_UNITS
  if (cut <= 0) return
  const { chunks } = held

  let first = 0
  let before = 0
  while (before + chunks[first]!.length <= cut) before += chunks[first++]!.length

  // Where the kept text starts: chunk and offset into it.
  let at = first
  let offset = cut - before
  for (let i = first, from = offset; i < chunks.length; i++, from = 0) {
    const newline = chunks[i]!.indexOf('\n', from)
    if (newline === -1) continue
    at = i
    offset = newline + 1
    break
  }

  const kept = chunks.slice(at)
  kept[0] = kept[0]!.slice(offset)
  if (!kept[0]) kept.shift()
  let dropped = offset
  for (let i = first; i < at; i++) dropped += chunks[i]!.length
  held.units -= before + dropped
  held.chunks = kept
}

export function readScrollback(id: string): string {
  const pipeline = pipelineFor(id)
  if (pipeline) {
    try {
      return pipeline.scrollback()
    } catch {
      // A thread that has stopped has nothing to give; the screen model's
      // failure is reported where it is freed.
      return ''
    }
  }
  const held = buffers.get(id)
  if (!held) return ''
  // Joined on the way out and kept joined: a caller that reads twice should
  // not pay twice, and the result is the same bytes either way. Already joined
  // is the common case for the second read and for the checkpoint that follows
  // one, so it costs nothing at all.
  compact(held)
  if (held.chunks.length > 1) held.chunks = [held.chunks.join('')]
  return held.chunks[0] ?? ''
}

/**
 * Replace a terminal's buffer with bytes from somewhere else.
 *
 * For recovery, which reconstructs it from a checkpoint and a log rather than
 * from a PTY. It goes through the same bound as an append, because a checkpoint
 * written by an older build -- or one whose cap was larger -- must not be able
 * to seed a buffer past what this module promises to hold.
 */
export function seedScrollback(id: string, data: string): void {
  const pipeline = pipelineFor(id)
  if (pipeline) {
    try {
      pipeline.seedScrollback(data)
      return
    } catch {
      // Kept here instead, where the next pipeline for this id picks it up.
    }
  }
  const held: Buffered = { chunks: [data], units: data.length }
  buffers.set(id, held)
  compact(held)
}

export function clearScrollback(id: string): void {
  buffers.delete(id)
  try {
    pipelineFor(id)?.seedScrollback('')
  } catch {
    // Stopped: nothing left in it to clear.
  }
}

/**
 * How much this terminal is holding, before any join.
 *
 * Exposed because the bound is otherwise unobservable: `readScrollback` compacts
 * on the way out, so a read is always within the cap no matter how much is being
 * held behind it. What a test needs to see is the memory, not the answer.
 */
/**
 * Take what is kept here for a terminal that is moving onto a core thread, so
 * its pipeline can be seeded with it and nothing is kept twice.
 */
export function takeScrollback(id: string): string {
  const held = buffers.get(id)
  if (!held) return ''
  buffers.delete(id)
  compact(held)
  return held.chunks.join('')
}

/**
 * Keep a pipeline's scrollback here once its thread is going away. A session
 * resumed under the same id keeps what it showed, as on the JavaScript path.
 */
export function handBackScrollback(id: string, data: string): void {
  if (!data) return
  const held: Buffered = { chunks: [data], units: data.length }
  buffers.set(id, held)
  compact(held)
}

export function scrollbackUnitsHeld(id: string): number {
  if (pipelineFor(id)) return readScrollback(id).length
  return buffers.get(id)?.units ?? 0
}

/** Test-only, mirroring the map this module used to expose implicitly. */
export function resetScrollback(): void {
  buffers.clear()
}
