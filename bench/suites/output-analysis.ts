/**
 * Hotspot 1: what `appendOutput` spends per megabyte of raw PTY output.
 *
 * `appendOutput` runs on every raw chunk node-pty hands over, before any
 * coalescing: `stripAnsi` (four alternated regexes, then the carriage-return
 * pass that rebuilds a line one character at a time), the partial-line join, the
 * line ring, the bracketed-paste scan and the status regexes. WP3 replaces all of
 * it and is accepted on the `spinner` number here going down twentyfold.
 *
 * The parts are timed on their own as well as together, so a regression can be
 * placed without a profiler.
 */
import { stripAnsi } from '../../packages/server/src/ansi-strip'
import { analyzeOutput, createStatusContext } from '../../packages/server/src/status-parser'
import { addAnalysisSession, pm, removeSession } from '../lib/server-harness'
import { MB, repeat, round, time } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import { transcripts } from '../lib/transcripts'

const REPS = QUICK ? 2 : 11
const CPU = { estimator: 'min', warmup: 5, minSampleMs: 200 } as const
const metrics: Record<string, Metric> = {}
const info: Record<string, unknown> = {}

let run = 0
for (const t of transcripts()) {
  const mb = t.bytes / MB
  info[t.name] = { bytes: t.bytes, chunks: t.chunks.length, description: t.description }

  const full = repeat(
    REPS,
    () => {
      const id = `analysis-${t.name}-${run++}`
      addAnalysisSession(id)
      const elapsed = time(() => {
        for (const c of t.chunks) pm.appendOutput(id, c)
      })
      removeSession(id)
      return elapsed
    },
    CPU
  )
  metrics[`appendOutput.${t.name}`] = metric(
    round(full / mb),
    'ms/MB',
    `appendOutput per raw chunk, ${t.name} transcript`
  )

  let sink = 0
  const strip = repeat(
    REPS,
    () =>
      time(() => {
        for (const c of t.chunks) sink += stripAnsi(c).length
      }),
    CPU
  )
  metrics[`stripAnsi.${t.name}`] = metric(round(strip / mb), 'ms/MB', `stripAnsi alone, ${t.name}`)

  const stripped = t.chunks.map(stripAnsi)
  const status = repeat(
    REPS,
    () => {
      const ctx = createStatusContext()
      return time(() => {
        for (const c of stripped) sink += analyzeOutput(ctx, c).length
      })
    },
    CPU
  )
  metrics[`statusRegex.${t.name}`] = metric(
    round(status / mb),
    'ms/MB',
    `analyzeOutput alone (status regexes), ${t.name}`
  )
  if (sink < 0) console.log(sink)
}

emit({ suite: 'output-analysis', metrics, info })
process.exit(0)
