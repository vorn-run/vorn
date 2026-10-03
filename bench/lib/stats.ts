/**
 * Timing and summary helpers shared by every suite.
 *
 * A suite reports the median of several repetitions rather than one sample, and
 * the runner reports the median of several processes. Both layers exist for the
 * same reason: the acceptance bar is "within 10% across three runs", and a single
 * wall-clock sample on a shared machine does not clear it.
 */

export function nowMs(): number {
  return Number(process.hrtime.bigint()) / 1e6
}

export function time(run: () => void): number {
  const started = process.hrtime.bigint()
  run()
  return Number(process.hrtime.bigint() - started) / 1e6
}

export async function timeAsync(run: () => Promise<void>): Promise<number> {
  const started = process.hrtime.bigint()
  await run()
  return Number(process.hrtime.bigint() - started) / 1e6
}

export function median(values: number[]): number {
  if (values.length === 0) return NaN
  const sorted = [...values].sort((a, b) => a - b)
  const mid = sorted.length >> 1
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}

/** Nearest-rank percentile, `p` in 0..100. */
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return NaN
  const sorted = [...values].sort((a, b) => a - b)
  const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))
  return sorted[rank]
}

export function mean(values: number[]): number {
  return values.reduce((a, b) => a + b, 0) / values.length
}

export interface RepeatOptions {
  warmup?: number
  minSampleMs?: number
  /**
   * `min` for pure CPU work, where everything that varies between samples --
   * an interrupt, a GC thread, a timer firing late -- only ever adds time, so
   * the fastest sample is the closest to the cost of the code and the most
   * repeatable. `median` where the work itself varies, like spawning git.
   */
  estimator?: 'median' | 'min'
}

const gc = (globalThis as { gc?: () => void }).gc

/**
 * The median of `reps` samples of `fn`, where `fn` does one pass and returns its
 * milliseconds.
 *
 * Two things make this reproducible where a bare loop is not. A sample is never
 * shorter than `minSampleMs`: a pass that takes 3 ms is repeated until the
 * sample is long enough that timer granularity and one stray interrupt stop
 * mattering, and the per-pass figure is the average inside it. And the heap is
 * collected before each sample (the runner passes `--expose-gc`), so one sample
 * does not pay for the garbage of the one before.
 */
export function repeat(reps: number, fn: () => number, opts: RepeatOptions = {}): number {
  const { warmup = 2, minSampleMs = 50, estimator = 'median' } = opts
  let probe = 0
  for (let i = 0; i < warmup; i++) probe = fn()
  const passes = Math.max(1, Math.ceil(minSampleMs / Math.max(probe, 0.01)))
  const samples: number[] = []
  for (let i = 0; i < reps; i++) {
    gc?.()
    let total = 0
    for (let p = 0; p < passes; p++) total += fn()
    samples.push(total / passes)
  }
  return estimator === 'min' ? Math.min(...samples) : median(samples)
}

export async function repeatAsync(
  reps: number,
  fn: () => Promise<number>,
  opts: RepeatOptions = {}
): Promise<number> {
  const { warmup = 2, minSampleMs = 50, estimator = 'median' } = opts
  let probe = 0
  for (let i = 0; i < warmup; i++) probe = await fn()
  const passes = Math.max(1, Math.ceil(minSampleMs / Math.max(probe, 0.01)))
  const samples: number[] = []
  for (let i = 0; i < reps; i++) {
    gc?.()
    let total = 0
    for (let p = 0; p < passes; p++) total += await fn()
    samples.push(total / passes)
  }
  return estimator === 'min' ? Math.min(...samples) : median(samples)
}

export const MB = 1024 * 1024

export function round(value: number, digits = 2): number {
  const f = 10 ** digits
  return Math.round(value * f) / f
}
