/**
 * Hotspot 2: the server's VT parse, in the screen model behind every PTY.
 *
 * Every flush is parsed by libghostty-vt on the terminal's own thread
 * (`terminal-screen.ts`). The client parses the same bytes again to draw them.
 *
 * Timed to the drain: the thread parses after the write returns, so the clock
 * stops when a serialize, which waits for everything fed before it, comes back.
 */
import {
  createScreen,
  feedScreen,
  hasScreen,
  serializeScreen,
  resetScreens
} from '../../packages/server/src/terminal-screen'
import { MB, repeatAsync, round, timeAsync } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import '../lib/core'
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
          // Waits for the parse; one serialize is small beside a megabyte of it.
          await serializeScreen(id)
        })
        // A core fault drops the model, which would time as a fast no-op.
        if (!hasScreen(id)) throw new Error(`screen model for ${t.name} was dropped mid-run`)
        resetScreens()
        return elapsed
      },
      CPU
    )
    metrics[`parse.${t.name}`] = metric(
      round(parse / mb),
      'ms/MB',
      `screen model parse to drain, ${t.name}, fed per flush`
    )
  }

  // What a checkpoint pays per session, on a full screen of coloured output.
  const agent = asFlushes(transcripts()[0])
  createScreen('serialize', COLS, ROWS)
  for (const f of agent) feedScreen('serialize', f)
  if (!(await serializeScreen('serialize'))) throw new Error('serializeScreen returned nothing')
  const serialize = await repeatAsync(
    QUICK ? 5 : 30,
    () => timeAsync(async () => void (await serializeScreen('serialize'))),
    { ...CPU, warmup: 3 }
  )
  if (!hasScreen('serialize')) throw new Error('screen model was dropped while serializing')
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
