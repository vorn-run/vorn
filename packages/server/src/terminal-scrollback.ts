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
 * worrying about. This is bytes, not lines, because the cost being bounded is
 * memory and the thing being stored is a byte stream.
 */
const MAX_BYTES = 256 * 1024

const buffers = new Map<string, string>()

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
  if (data.length <= MAX_BYTES) return data
  const cut = data.length - MAX_BYTES
  const boundary = data.indexOf('\n', cut)
  return boundary === -1 ? data.slice(cut) : data.slice(boundary + 1)
}

export function appendScrollback(id: string, data: string): void {
  buffers.set(id, trim((buffers.get(id) ?? '') + data))
}

export function readScrollback(id: string): string {
  return buffers.get(id) ?? ''
}

export function clearScrollback(id: string): void {
  buffers.delete(id)
}

/** Only for tests, which would otherwise leak state between cases. */
export function resetScrollback(): void {
  buffers.clear()
}
