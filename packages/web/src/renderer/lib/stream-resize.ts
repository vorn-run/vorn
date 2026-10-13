import type { Terminal } from '@xterm/xterm'

/**
 * Resizes a terminal at its place in the byte stream.
 *
 * xterm.js parses what `write` is given later, in its own timer, while
 * `resize` takes effect at once. A resize from the session's record log
 * applied straight away would land ahead of output still waiting to be
 * parsed, and that output -- written by the program for the old size --
 * would be drawn at the new one. Queued behind an empty write, the resize
 * runs once everything before it is parsed and before anything written
 * after it, which is where the record put it.
 */
export function resizeInOrder(
  term: Pick<Terminal, 'write' | 'resize'>,
  cols: number,
  rows: number,
  done?: () => void
): void {
  term.write('', () => {
    term.resize(cols, rows)
    done?.()
  })
}
