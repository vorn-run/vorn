/**
 * How the client commands write.
 *
 * One rule underneath all of it: stdout carries data and nothing else, so a
 * command can be piped without anything having to be stripped out of it.
 * Notices and errors go to stderr, which is where the logger already writes.
 */

// Built rather than typed, so no escape byte sits in the source.
const CSI = `${String.fromCharCode(27)}[`
const RESET = `${CSI}0m`

/**
 * Whether output must stay plain: piped, redirected, or asked to be.
 *
 * `NO_COLOR` is honoured by presence, not value — that is what the convention
 * says, and an empty string is how most shells set it.
 */
export function isPlain(isTty: boolean): boolean {
  return !isTty || process.env.NO_COLOR !== undefined || process.env.TERM === 'dumb'
}

/** The one place data becomes JSON, so every command emits the same shape. */
export function asJson(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`
}

const UUID_HEAD = /^[0-9a-f]{8}-[0-9a-f]{4}-/i

/**
 * Eight characters of a uuid still identify it, and a row stays readable.
 *
 * Only of a uuid: seeded and imported workflows carry names as ids, and cutting
 * `system:default-task-workflow` down to `system:d` identifies nothing — two of
 * them would print the same string.
 */
export function shortId(id: string): string {
  return UUID_HEAD.test(id) ? id.slice(0, 8) : id
}

/**
 * Columns padded to their widest cell, never truncated.
 *
 * Padding happens before `paint` runs, so colour codes cannot widen a cell and
 * push the columns out of line.
 */
export function table(
  headers: string[],
  rows: string[][],
  paint?: (cell: string, column: number) => string
): string {
  const widths = headers.map((header, i) =>
    Math.max(header.length, ...rows.map((row) => (row[i] ?? '').length))
  )
  const line = (cells: string[], colour: boolean): string =>
    cells
      .map((cell, i) => {
        const padded = i === cells.length - 1 ? cell : cell.padEnd(widths[i])
        return colour && paint ? paint(padded, i) : padded
      })
      .join('  ')
      .trimEnd()

  return [line(headers, false), ...rows.map((row) => line(row, true))].join('\n') + '\n'
}

/** Colour carries status and nothing else, and only when a terminal is watching. */
export function paintStatus(text: string, plain: boolean): string {
  if (plain) return text
  const key = text.trim()
  if (key === 'waiting') return `${CSI}33m${text}${RESET}`
  if (key === 'error' || key === 'cancelled') return `${CSI}31m${text}${RESET}`
  if (key === 'running' || key === 'success') return `${CSI}32m${text}${RESET}`
  if (key === 'idle' || key === 'exited') return `${CSI}2m${text}${RESET}`
  return text
}

/** How long ago, in the coarsest unit that still says something. */
export function timeAgo(when: number | string | undefined): string {
  if (when === undefined) return '-'
  const then = typeof when === 'number' ? when : Date.parse(when)
  if (!Number.isFinite(then)) return '-'

  const seconds = Math.max(0, Math.round((Date.now() - then) / 1000))
  if (seconds < 60) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.round(minutes / 60)
  if (hours < 48) return `${hours}h ago`
  return `${Math.round(hours / 24)}d ago`
}
