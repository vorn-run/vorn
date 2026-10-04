/**
 * Where the native output analysis is allowed to differ from the JavaScript
 * analysis it replaced (`ansi-strip.ts` and `status-parser.ts`, kept as
 * recorded output in `fixtures/js-reference/analysis.json`).
 *
 * As for the screen in `screen-parity.ts`, every accepted difference is named
 * here, so a new one has to be added on purpose.
 */

/**
 * The native analysis applies a carriage return and an erase in line to the
 * line being built, as a terminal does, so a line a program redrew in place
 * holds what was left on it: often nothing, when a spinner is erased before
 * the next line is written. The JavaScript path stripped those sequences and
 * kept every redraw run together on one line. Such a line is compared by its
 * position, not its text.
 */
export const REDRAWN_LINES = 'lines-redrawn-in-place-keep-what-is-left'

// eslint-disable-next-line no-control-regex
const REDRAW = /\r(?!$)|\x1b\[[0-2]?K|\x1b\[\d*G/

/**
 * The indexes of the output lines that a redraw touched, from the raw reads:
 * the lines `REDRAWN_LINES` compares by position only.
 */
export function redrawnLines(reads: string[]): Set<number> {
  const lines = reads.join('').split('\n')
  const touched = new Set<number>()
  lines.forEach((line, i) => {
    if (REDRAW.test(line)) touched.add(i)
  })
  return touched
}

/** Lines with the text of every redrawn one set aside, for comparing the rest. */
export function withoutRedrawn(lines: string[], redrawn: Set<number>): string[] {
  return lines.map((line, i) => (redrawn.has(i) ? `<${REDRAWN_LINES}>` : line))
}
