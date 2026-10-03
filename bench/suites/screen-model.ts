/**
 * Hotspot 2: the second VT parse, in the headless xterm behind every PTY.
 *
 * Every flush is written into an `@xterm/headless` terminal whose only consumer
 * is the history checkpoint (`terminal-screen.ts`). The client parses the same
 * bytes again to draw them. WP2 replaces this with libghostty-vt in the core.
 *
 * Timed to the drain: xterm parses on a macrotask, so the clock stops when the
 * parse has actually happened, not when the writes were queued. Each sample is at
 * least 200 ms of parsing, so the 1 ms timer gaps xterm yields with between
 * slices are a small, steady share of it.
 */
import {
  createScreen,
  feedScreen,
  serializeScreen,
  resetScreens
} from '../../packages/server/src/terminal-screen'
import { MB, repeatAsync, round, timeAsync } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import { asFlushes, transcripts } from '../lib/transcripts'

const REPS = QUICK ? 2 : 9
const CPU = { estimator: 'min', warmup: 5, minSampleMs: 200 } as const
const COLS = 200
const ROWS = 50

async function main(): Promise<void> {
  const metrics: Record<string, Metric> = {}
  let run = 0
  for (const t of transcripts()) {
    const flushes = asFlushes(t)
    const mb = t.bytes / MB
    const parse = await repeatAsync(
      REPS,
      async () => {
        const id = `screen-${run++}`
        createScreen(id, COLS, ROWS)
        const elapsed = await timeAsync(async () => {
          for (const f of flushes) feedScreen(id, f)
          await serializeScreen(id)
        })
        resetScreens()
        return elapsed
      },
      CPU
    )
    metrics[`parse.${t.name}`] = metric(
      round(parse / mb),
      'ms/MB',
      `headless xterm parse to drain, ${t.name}, fed per flush`
    )
  }

  // What a checkpoint pays per session, on a full screen of coloured output.
  const agent = asFlushes(transcripts()[0])
  createScreen('serialize', COLS, ROWS)
  for (const f of agent) feedScreen('serialize', f)
  await serializeScreen('serialize')
  const serialize = await repeatAsync(
    QUICK ? 5 : 30,
    () => timeAsync(async () => void (await serializeScreen('serialize'))),
    { ...CPU, warmup: 3 }
  )
  metrics['serialize.200x50'] = metric(
    round(serialize, 3),
    'ms',
    'serializeScreen of one 200x50 coloured screen (checkpoint cost per session)'
  )
  resetScreens()

  emit({ suite: 'screen-model', metrics, info: { cols: COLS, rows: ROWS } })
  process.exit(0)
}

void main()
