/**
 * `yarn bench`: run every suite in a process of its own, several times, and
 * report the median with its spread.
 *
 *   yarn bench                      three runs of everything, compared to the baseline
 *   yarn bench --only=git,flush     a subset
 *   yarn bench --runs=5             more runs
 *   yarn bench --quick              one short run, a smoke check rather than a number
 *   yarn bench --save               write the baseline for this platform and the doc table
 *   yarn bench --max-regression=10  fail when a metric is more than 10% worse than the baseline
 *
 * Runs are interleaved (suite A, B, C, then A, B, C again) rather than batched,
 * so a machine that slows down halfway spreads its drift across every suite
 * instead of landing on one. The acceptance bar for WP0 is every metric within
 * 10% across three runs, read as every run within 10% of the median of the
 * runs; `--strict` turns a miss into a failing exit code.
 */
import { spawnSync, execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { median, round } from './lib/stats'
import type { Metric, SuiteResult } from './lib/suite'

const ROOT = path.resolve(__dirname, '..')
const SUITES = [
  'output-analysis',
  'screen-model',
  'flush',
  'git',
  'event-loop',
  'renderer',
  'memory'
]
/**
 * Processes per run, for suites whose numbers move between processes more than
 * within one: the output analysis and the screen parse land up to 15% apart
 * from one process to the next on the same machine, and event-loop percentiles
 * depend on how the OS schedules that one process. One run of these is the
 * median of three processes.
 */
const PROCESSES: Record<string, number> = {
  'output-analysis': 3,
  'screen-model': 3,
  'event-loop': 3,
  memory: 3
}
const SPREAD_LIMIT = 10
/** Baselines this close to zero (e.g. no long tasks at all) give a meaningless %. */
const ZERO_BASELINE = 1e-9

const args = new Map<string, string>()
for (const a of process.argv.slice(2)) {
  const [k, v] = a.replace(/^--/, '').split('=')
  args.set(k, v ?? 'true')
}
const quick = args.has('quick')
const runs = Number(args.get('runs') ?? (quick ? 1 : 3))
if (!Number.isInteger(runs) || runs < 1) {
  throw new Error(`--runs needs a positive whole number, got ${args.get('runs')}`)
}
const only = args.get('only')?.split(',')
const unknown = only?.filter((s) => !SUITES.includes(s)) ?? []
if (unknown.length > 0) {
  throw new Error(`unknown suite(s) in --only: ${unknown.join(', ')} (have ${SUITES.join(', ')})`)
}
const maxRegression = args.has('max-regression') ? Number(args.get('max-regression')) : null
if (maxRegression !== null && !(maxRegression >= 0)) {
  throw new Error(`--max-regression needs a percentage, got ${args.get('max-regression')}`)
}
const suites = only ? SUITES.filter((s) => only.includes(s)) : SUITES
const platformKey = `${process.platform}-${process.arch}`
// Baselines say which terminal path they were measured on. The ones recorded
// before the JavaScript path was removed say `js`, and are not a gate for this one.
const core = 'native'
const baselinePath =
  args.get('baseline') ?? path.join(ROOT, 'bench', 'baselines', `${platformKey}.json`)

export interface Summary {
  value: number
  unit: string
  better: Metric['better']
  label: string
  samples: number[]
  /** How far the furthest run sits from the median, in per cent. */
  spreadPct: number
}

export interface Baseline {
  machine: Record<string, unknown>
  recordedAt: string
  commit: string
  runs: number
  suites: Record<
    string,
    {
      metrics: Record<string, Summary>
      info?: Record<string, unknown>
      /** Where this suite was measured, when a partial save kept it from an older run. */
      commit?: string
      recordedAt?: string
    }
  >
}

function machine(): Record<string, unknown> {
  let git = ''
  try {
    git = execFileSync('git', ['--version'], { encoding: 'utf-8' }).trim()
  } catch {
    /* recorded as empty */
  }
  return {
    platform: platformKey,
    cpu: os.cpus()[0]?.model ?? 'unknown',
    cores: os.cpus().length,
    memoryGB: Math.round(os.totalmem() / 1024 ** 3),
    node: process.version,
    git
  }
}

function runOnce(suite: string): SuiteResult {
  const file = path.join(ROOT, 'bench', 'suites', `${suite}.ts`)
  const run = spawnSync(process.execPath, ['--expose-gc', '--import', 'tsx', file], {
    cwd: ROOT,
    encoding: 'utf-8',
    env: { ...process.env, VORN_BENCH_QUICK: quick ? '1' : '0' },
    maxBuffer: 64 * 1024 * 1024,
    timeout: 15 * 60_000
  })
  if (run.status !== 0) {
    throw new Error(
      `suite ${suite} failed (${run.status ?? run.signal}):\n${run.stderr.slice(-4000)}`
    )
  }
  const line = run.stdout.trim().split('\n').pop() ?? ''
  return JSON.parse(line) as SuiteResult
}

function runSuite(suite: string): SuiteResult {
  const procs = quick ? 1 : (PROCESSES[suite] ?? 1)
  const results = Array.from({ length: procs }, () => runOnce(suite))
  if (procs === 1) return results[0]
  const metrics: SuiteResult['metrics'] = {}
  for (const [name, first] of Object.entries(results[0].metrics)) {
    metrics[name] = { ...first, value: round(median(results.map((r) => r.metrics[name].value)), 3) }
  }
  return { ...results[0], metrics }
}

/**
 * How far the furthest run sits from the median, in per cent. A zero median
 * with any non-zero run is reported as 100% rather than 0%: nothing about
 * `[0, 0, 13.5]` is reproducible.
 */
function spread(samples: number[], m: number): number {
  const furthest = Math.max(...samples.map((x) => Math.abs(x - m)))
  if (m === 0) return furthest === 0 ? 0 : 100
  return round((furthest / m) * 100, 1)
}

function pct(n: number): string {
  return `${n >= 0 ? '+' : ''}${n.toFixed(1)}%`
}

function main(): void {
  if (args.has('doc')) {
    // Rewrite the doc tables from the committed baselines, for one recorded elsewhere.
    const dir = path.join(ROOT, 'bench', 'baselines')
    for (const file of fs.readdirSync(dir).filter((f) => f.endsWith('.json'))) {
      const record = JSON.parse(fs.readFileSync(path.join(dir, file), 'utf-8')) as Baseline
      const doc = writeDocTable(record, file.replace(/\.json$/, ''))
      if (doc)
        execFileSync(
          process.execPath,
          [require.resolve('prettier/bin/prettier.cjs'), '--write', doc],
          { stdio: 'ignore' }
        )
    }
    return
  }
  const collected: Record<string, SuiteResult[]> = {}
  for (let r = 0; r < runs; r++) {
    for (const suite of suites) {
      process.stderr.write(`run ${r + 1}/${runs} · ${suite} … `)
      const started = Date.now()
      const result = runSuite(suite)
      ;(collected[suite] ??= []).push(result)
      process.stderr.write(`${((Date.now() - started) / 1000).toFixed(1)}s\n`)
    }
  }

  const summary: Baseline['suites'] = {}
  for (const [suite, results] of Object.entries(collected)) {
    const metrics: Record<string, Summary> = {}
    for (const name of Object.keys(results[0].metrics)) {
      const samples = results.map((r) => r.metrics[name].value)
      const m = median(samples)
      const first = results[0].metrics[name]
      metrics[name] = {
        value: round(m, 3),
        unit: first.unit,
        better: first.better,
        label: first.label,
        samples,
        spreadPct: spread(samples, m)
      }
    }
    summary[suite] = { metrics, info: results[results.length - 1].info }
  }

  const baseline: Baseline | null = fs.existsSync(baselinePath)
    ? (JSON.parse(fs.readFileSync(baselinePath, 'utf-8')) as Baseline)
    : null
  if (maxRegression !== null && !baseline) {
    // A gate with nothing to compare against would pass without checking anything.
    throw new Error(`--max-regression needs a baseline, and ${baselinePath} does not exist`)
  }

  const here = machine()
  if (baseline) {
    const baseCore = (baseline.machine.core as string | undefined) ?? 'js'
    if (baseCore !== core) {
      const msg = `the baseline at ${path.relative(ROOT, baselinePath)} was recorded on the ${baseCore} terminal path, this run is ${core}`
      // A gate across cores would pass or fail on the core, not the change.
      if (maxRegression !== null) throw new Error(`${msg}; --max-regression needs the same core`)
      console.warn(`warning: ${msg}; compare with care`)
    }
    const was = baseline.machine
    const differs = (['cpu', 'cores', 'memoryGB'] as const).filter((k) => was[k] !== here[k])
    if (differs.length > 0) {
      console.warn(
        `warning: the baseline was recorded on a different machine (${differs
          .map((k) => `${k}: ${String(was[k])} vs ${String(here[k])}`)
          .join('; ')}); compare with care`
      )
    }
  }

  let misses = 0
  let regressions = 0
  const rows: string[][] = [['metric', 'median', 'spread', 'baseline', 'Δ']]
  for (const [suite, { metrics }] of Object.entries(summary)) {
    for (const [name, s] of Object.entries(metrics)) {
      const base = baseline?.suites[suite]?.metrics[name]
      const delta =
        base && Math.abs(base.value) > ZERO_BASELINE
          ? ((s.value - base.value) / base.value) * 100
          : null
      const wide = runs > 1 && s.spreadPct > SPREAD_LIMIT
      if (wide) misses++
      // Worse means up for a cost and down for a rate.
      const worse = delta === null ? 0 : s.better === 'lower' ? delta : -delta
      const regressed = maxRegression !== null && worse > maxRegression
      if (regressed) regressions++
      rows.push([
        `${suite}/${name}`,
        `${s.value} ${s.unit}`,
        runs > 1 ? `${s.spreadPct}%${wide ? ' !' : ''}` : '-',
        base ? `${base.value} ${base.unit}` : '-',
        delta === null ? '-' : `${pct(delta)}${regressed ? ' ✗' : ''}`
      ])
    }
  }
  const widths = rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].length)))
  for (const r of rows) console.log(r.map((c, i) => c.padEnd(widths[i])).join('  '))

  const record: Baseline = {
    machine: { ...here, core },
    recordedAt: new Date().toISOString(),
    commit: currentCommit(),
    runs,
    suites: summary
  }
  const resultsDir = path.join(ROOT, 'bench', 'results')
  fs.mkdirSync(resultsDir, { recursive: true })
  fs.writeFileSync(path.join(resultsDir, 'latest.json'), JSON.stringify(record, null, 2) + '\n')
  const out = args.get('out')
  if (out) fs.writeFileSync(path.resolve(out), JSON.stringify(record, null, 2) + '\n')

  if (runs > 1) {
    console.log(
      misses === 0
        ? `\nall metrics within ${SPREAD_LIMIT}% across ${runs} runs`
        : `\n${misses} metric(s) spread more than ${SPREAD_LIMIT}% across ${runs} runs (marked !)`
    )
  }

  if (args.has('save')) {
    if (quick) throw new Error('--save needs full runs, not --quick')
    if (only && baseline) {
      // A partial run updates its own suites and keeps the rest, each kept
      // suite still saying which commit and date it was measured at.
      const kept: Baseline['suites'] = {}
      for (const [name, s] of Object.entries(baseline.suites)) {
        kept[name] = {
          ...s,
          commit: s.commit ?? baseline.commit,
          recordedAt: s.recordedAt ?? baseline.recordedAt
        }
      }
      record.suites = { ...kept, ...record.suites }
    }
    fs.mkdirSync(path.dirname(baselinePath), { recursive: true })
    fs.writeFileSync(baselinePath, JSON.stringify(record, null, 2) + '\n')
    console.log(`baseline written to ${path.relative(ROOT, baselinePath)}`)
    const doc = writeDocTable(record)
    // Formatted as the repo formats them, so committing a baseline is not also a style diff.
    execFileSync(
      process.execPath,
      [
        require.resolve('prettier/bin/prettier.cjs'),
        '--write',
        baselinePath,
        ...(doc ? [doc] : [])
      ],
      { cwd: ROOT, stdio: 'ignore' }
    )
  }

  if (maxRegression !== null) {
    console.log(
      regressions === 0
        ? `no metric regressed more than ${maxRegression}% against the baseline`
        : `${regressions} metric(s) regressed more than ${maxRegression}% against the baseline (marked ✗)`
    )
  }

  if (args.has('strict') && misses > 0) process.exit(1)
  if (regressions > 0) process.exit(1)
}

/** HEAD, marked dirty when the tree has changes, so a baseline says what it measured. */
function currentCommit(): string {
  const git = (...a: string[]): string =>
    execFileSync('git', a, { cwd: ROOT, encoding: 'utf-8' }).trim()
  const head = git('rev-parse', '--short', 'HEAD')
  return git('status', '--porcelain', '--untracked-files=no') ? `${head}-dirty` : head
}

/** Keep the numbers in the roadmap doc in step with the committed baseline. */
function writeDocTable(record: Baseline, key = platformKey): string | null {
  const doc = path.join(ROOT, 'docs', 'native-core', 'benchmarks.md')
  if (!fs.existsSync(doc)) return null
  const start = `<!-- bench:${key}:start -->`
  const end = `<!-- bench:${key}:end -->`
  const text = fs.readFileSync(doc, 'utf-8')
  const from = text.indexOf(start)
  const to = text.indexOf(end)
  if (from === -1 || to === -1) return null
  const m = record.machine
  const olderSuites = Object.entries(record.suites)
    .filter(([, s]) => s.commit && s.commit !== record.commit)
    .map(
      ([name, s]) =>
        `\`${name}\` was recorded ${s.recordedAt?.slice(0, 10)} at \`${s.commit}\`, in an earlier run.`
    )
  const lines = [
    start,
    '',
    `${m.cpu}, ${m.cores} cores, ${m.memoryGB} GB, Node ${m.node}, ${m.git}. ` +
      `Recorded ${record.recordedAt.slice(0, 10)} at \`${record.commit}\`, median of ${record.runs} runs.`,
    '',
    ...olderSuites.flatMap((note) => [note, '']),
    '| Metric | Median | Spread | What |',
    '| --- | ---: | ---: | --- |'
  ]
  for (const [suite, { metrics }] of Object.entries(record.suites)) {
    for (const [name, s] of Object.entries(metrics)) {
      lines.push(`| \`${suite}/${name}\` | ${s.value} ${s.unit} | ${s.spreadPct}% | ${s.label} |`)
    }
  }
  const gpu = record.suites.renderer?.info?.gpu
  if (gpu) lines.push('', `Renderer GPU: ${String(gpu)}.`)
  lines.push('', end)
  fs.writeFileSync(doc, text.slice(0, from) + lines.join('\n') + text.slice(to + end.length))
  console.log(`table updated in ${path.relative(ROOT, doc)}`)
  return doc
}

main()
