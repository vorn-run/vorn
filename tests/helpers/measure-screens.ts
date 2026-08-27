import {
  feedScreen,
  serializeScreen,
  clearScreen,
  screenCount
} from '../../packages/server/src/terminal-screen'

/**
 * What fifty screen models cost, measured in a process of their own.
 *
 * Run as a child rather than inside vitest, because the vitest worker's own heap
 * — module graph, React, the rest of the suite — is noisier than the thing being
 * measured. Needs `--expose-gc`; without it there is no way to distinguish
 * "released" from "not yet collected", and a number that cannot tell those apart
 * is not a measurement.
 *
 * Prints one line of JSON on stdout. The test reads it and decides.
 */

const SESSIONS = 50
const COLS = 200
const ROWS = 50

/**
 * Output shaped like a working agent's, not `'x'.repeat(n)`.
 *
 * A screen of identical ASCII is the best case for xterm's cell storage, so
 * measuring against one would report a cost nobody actually pays. This has
 * colour changes, cursor movement and varied text — the things that make cells
 * differ from each other.
 */
function realisticPaint(seed: number): string {
  const lines: string[] = [`\x1b[?1049h\x1b[2J\x1b[H`]
  for (let row = 0; row < ROWS; row++) {
    const fg = 30 + ((row + seed) % 8)
    const bold = row % 3 === 0 ? '1;' : ''
    lines.push(
      `\x1b[${row + 1};1H\x1b[${bold}${fg}m` +
        `session ${seed} line ${row} ` +
        `${'▁▂▃▄▅▆▇█'[row % 8]} ` +
        `packages/server/src/file-${row}.ts:${row * 7} ` +
        `\x1b[0m`
    )
  }
  return lines.join('')
}

async function settle(): Promise<void> {
  // Twice, with a macrotask between: one pass does not finish, and anything
  // reachable from a finalizer needs the second.
  const gc = (globalThis as { gc?: () => void }).gc
  if (!gc) throw new Error('run with --expose-gc')
  gc()
  await new Promise((r) => setTimeout(r, 50))
  gc()
  await new Promise((r) => setTimeout(r, 50))
}

/** Build fifty, measure, release, measure. */
async function cycle(round: number): Promise<{ held: number; after: number; modelled: number }> {
  await settle()
  const before = process.memoryUsage().heapUsed

  for (let i = 0; i < SESSIONS; i++) feedScreen(`r${round}s${i}`, realisticPaint(i), COLS, ROWS)
  // Serialized too, so the addon has done its work and nothing is lazily
  // unbuilt at the moment of measuring.
  for (let i = 0; i < SESSIONS; i++) await serializeScreen(`r${round}s${i}`)

  await settle()
  const held = process.memoryUsage().heapUsed
  const modelled = screenCount()

  for (let i = 0; i < SESSIONS; i++) clearScreen(`r${round}s${i}`)
  await settle()

  return { held: held - before, after: process.memoryUsage().heapUsed - before, modelled }
}

async function main(): Promise<void> {
  // Twice, because one round cannot tell a leak from the heap simply not
  // returning to exactly where it started. A per-session leak accumulates; noise
  // does not.
  const first = await cycle(1)
  const second = await cycle(2)

  process.stdout.write(
    JSON.stringify({
      sessions: SESSIONS,
      cols: COLS,
      rows: ROWS,
      modelled: first.modelled,
      remaining: screenCount(),
      heldBytes: first.held,
      residualFirst: first.after,
      residualSecond: second.after
    }) + '\n'
  )
}

void main()
