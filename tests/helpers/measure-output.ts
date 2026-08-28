/**
 * The bytes every measurement of the write path is timed against, and the clock
 * they are timed with.
 *
 * Shared because two harnesses report numbers that are meant to be read against
 * each other — one says what the byte buffer and the screen model cost, the
 * other what history adds to them — and that comparison stops meaning anything
 * the moment one copy of the generator drifts from the other.
 */

export const CHUNKS = 20_000
export const COLS = 200
export const ROWS = 50

/** Output shaped like a working agent's: colour, cursor movement, varied text. */
export function chunks(): string[] {
  const out: string[] = []
  for (let i = 0; i < CHUNKS; i++) {
    const fg = 30 + (i % 8)
    out.push(
      `\x1b[${(i % ROWS) + 1};1H\x1b[${fg}m` +
        `⏺ packages/server/src/file-${i % 40}.ts:${i} ` +
        `${'▁▂▃▄▅▆▇█'[i % 8]} done\x1b[0m\r\n`
    )
  }
  return out
}

export function ms(run: () => void): number {
  const started = process.hrtime.bigint()
  run()
  return Number(process.hrtime.bigint() - started) / 1e6
}

export async function msAsync(run: () => Promise<void>): Promise<number> {
  const started = process.hrtime.bigint()
  await run()
  return Number(process.hrtime.bigint() - started) / 1e6
}
