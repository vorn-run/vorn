/**
 * Hotspot 1: what `appendOutput` spends per megabyte of raw PTY output.
 *
 * `appendOutput` runs on every raw chunk node-pty hands over, before any
 * coalescing. The per-read work is only queueing the chunk; the core analyzes
 * each burst once, when the stream pauses or 8 ms pass. The timed loop runs
 * that analysis at the same boundaries every other suite flushes at
 * (`asFlushes`: 100 reads or 64 KB), so the number includes it.
 *
 * `batched` (the same analysis with every chunk in one napi call) and
 * `napiFloor` (a call per chunk that does nothing) are timed on their own, so
 * a regression can be placed without a profiler.
 */
import { addAnalysisSession, pm, removeSession } from '../lib/server-harness'
import { MB, repeat, round, time } from '../lib/stats'
import { emit, metric, QUICK, type Metric } from '../lib/suite'
import { FLUSH_BYTES, FLUSH_READS, transcripts } from '../lib/transcripts'
import { nativeCore } from '../lib/core'

/** Exports the core has for measuring, beside the ones the server uses. */
const native = nativeCore as typeof nativeCore & {
  analyzeBatch(chunks: string[], analyze: boolean): number
  noop(data: string): number
}

const REPS = QUICK ? 2 : 7
const CPU = { estimator: 'min', warmup: 5, minSampleMs: 200 } as const
const metrics: Record<string, Metric> = {}
const info: Record<string, unknown> = {}

let run = 0
for (const t of transcripts()) {
  const mb = t.bytes / MB
  info[t.name] = { bytes: t.bytes, chunks: t.chunks.length, description: t.description }
  const sizes = t.chunks.map((c) => Buffer.byteLength(c))

  const full = repeat(
    REPS,
    () => {
      const id = `analysis-${t.name}-${run++}`
      addAnalysisSession(id)
      const elapsed = time(() => {
        let reads = 0
        let bytes = 0
        for (let i = 0; i < t.chunks.length; i++) {
          pm.appendOutput(id, t.chunks[i])
          reads++
          bytes += sizes[i]
          if (reads >= FLUSH_READS || bytes >= FLUSH_BYTES) {
            pm.flushAnalysis(id)
            reads = 0
            bytes = 0
          }
        }
        pm.flushAnalysis(id)
      })
      removeSession(id)
      return elapsed
    },
    CPU
  )
  metrics[`appendOutput.${t.name}`] = metric(
    round(full / mb),
    'ms/MB',
    `appendOutput per raw chunk, plus analysis per flush, ${t.name} transcript`
  )

  let sink = 0
  const batched = repeat(
    REPS,
    () => time(() => void (sink += native.analyzeBatch(t.chunks, true))),
    CPU
  )
  metrics[`batched.${t.name}`] = metric(
    round(batched / mb),
    'ms/MB',
    `native analysis, every chunk in one napi call, ${t.name}`
  )
  const floor = repeat(
    REPS,
    () =>
      time(() => {
        for (const c of t.chunks) sink += native.noop(c)
      }),
    CPU
  )
  metrics[`napiFloor.${t.name}`] = metric(
    round(floor / mb),
    'ms/MB',
    `one napi call per chunk that only receives the string, ${t.name}`
  )
  if (sink < 0) console.log(sink)
}

emit({ suite: 'output-analysis', metrics, info })
process.exit(0)
