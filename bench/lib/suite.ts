/**
 * The contract between a suite and the runner.
 *
 * A suite is a script run in a process of its own. It prints exactly one line of
 * JSON on stdout, last, and the runner ignores anything before it -- the same
 * convention `tests/helpers/run-measurement.ts` uses for the process tests.
 */

export interface Metric {
  value: number
  unit: string
  /** Every metric here is a cost, so lower is better; kept explicit for the report. */
  better: 'lower' | 'higher'
  /** One line on what was measured, shown in the report and the benchmarks doc. */
  label: string
}

export interface SuiteResult {
  suite: string
  metrics: Record<string, Metric>
  /** Context worth keeping beside the numbers: sizes, counts, the GPU string. */
  info?: Record<string, unknown>
}

/** Whether the runner asked for the short form, for a smoke check rather than a baseline. */
export const QUICK = process.env.VORN_BENCH_QUICK === '1'

export function emit(result: SuiteResult): void {
  process.stdout.write(JSON.stringify(result) + '\n')
}

export function metric(
  value: number,
  unit: string,
  label: string,
  better: Metric['better'] = 'lower'
): Metric {
  return { value, unit, label, better }
}
