/**
 * `yarn bench:compare`: the same suites with `VORN_CORE=js` and `VORN_CORE=native`,
 * side by side.
 *
 *   yarn bench:compare              three runs each
 *   yarn bench:compare --runs=5
 *   yarn bench:compare --quick      a smoke check
 *
 * Writes `bench/results/compare.md` and the two merged records beside it. Each
 * run of each core is its own child `yarn bench --runs=1`, so neither can warm
 * the other, and the cores take turns going first, so drift over the session
 * (load, heat) lands on both sides rather than on whichever ran second.
 */
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type { Baseline, Summary } from './run'

type Core = 'js' | 'native'

const ROOT = path.resolve(__dirname, '..')
const RESULTS = path.join(ROOT, 'bench', 'results')
const SUITES = 'output-analysis,screen-model,flush,event-loop,memory'

const argv = process.argv.slice(2)
const quick = argv.includes('--quick')
const runsArg = argv.find((a) => a.startsWith('--runs='))?.slice(7)
const runs = Number(runsArg ?? (quick ? 1 : 3))
if (!Number.isInteger(runs) || runs < 1) {
  throw new Error(`--runs needs a positive whole number, got ${runsArg}`)
}
// Each child is one run; saving or gating a single run would mean nothing.
for (const flag of ['--save', '--max-regression', '--out']) {
  if (argv.some((a) => a === flag || a.startsWith(`${flag}=`))) {
    throw new Error(`${flag} is for \`yarn bench\`; bench:compare only writes its own results`)
  }
}
const passthrough = argv.filter((a) => !a.startsWith('--only') && !a.startsWith('--runs'))
const only = argv.find((a) => a.startsWith('--only='))?.slice(7) ?? SUITES

function runOnce(core: Core, r: number): Baseline {
  const out = path.join(RESULTS, `compare-${core}-${r + 1}.json`)
  const child = spawnSync(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(ROOT, 'bench', 'run.ts'),
      `--only=${only}`,
      '--runs=1',
      `--out=${out}`,
      ...passthrough
    ],
    { cwd: ROOT, stdio: ['ignore', 'inherit', 'inherit'], env: { ...process.env, VORN_CORE: core } }
  )
  if (child.status !== 0) throw new Error(`bench with VORN_CORE=${core} failed`)
  const record = JSON.parse(fs.readFileSync(out, 'utf-8')) as Baseline
  fs.rmSync(out)
  return record
}

function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b)
  const mid = s.length >> 1
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2
}

/** One record from single-run records, as `yarn bench --runs=N` would summarize them. */
function merge(records: Baseline[]): Baseline {
  const first = records[0]
  const suites: Baseline['suites'] = {}
  for (const [suite, { metrics, info }] of Object.entries(first.suites)) {
    const merged: Record<string, Summary> = {}
    for (const [name, s] of Object.entries(metrics)) {
      const samples = records.flatMap((r) => r.suites[suite]?.metrics[name]?.samples ?? [])
      const m = median(samples)
      const furthest = Math.max(...samples.map((x) => Math.abs(x - m)))
      merged[name] = {
        ...s,
        value: Math.round(m * 1000) / 1000,
        samples,
        spreadPct: m === 0 ? (furthest === 0 ? 0 : 100) : Math.round((furthest / m) * 1000) / 10
      }
    }
    suites[suite] = { metrics: merged, info }
  }
  return { ...first, runs: records.length, suites }
}

function fmt(s: Summary | undefined): string {
  return s ? `${s.value} ${s.unit}` : '–'
}

/** How many times better native is: >1 means native wins, whichever way the metric points. */
function gain(js: Summary | undefined, nat: Summary | undefined): string {
  if (!js || !nat || js.value === 0 || nat.value === 0) return '–'
  const x = js.better === 'lower' ? js.value / nat.value : nat.value / js.value
  return x >= 1 ? `**${x.toFixed(x >= 10 ? 0 : 1)}x**` : `${x.toFixed(2)}x (slower)`
}

/**
 * What one megabyte costs the server end to end, on the loop: per-chunk
 * analysis, the flush, and for JS the xterm parse the flush queues (native
 * parses inside the flush, so it is already in `flush`).
 */
function perMbTotal(b: Baseline, name: string, native: boolean): number | null {
  const m = (suite: string, metric: string): number | undefined =>
    b.suites[suite]?.metrics[metric]?.value
  const analysis = m('output-analysis', `appendOutput.${name}`)
  const flush = m('flush', `flush.${name}.1client`)
  const parse = native ? 0 : m('screen-model', `parse.${name}`)
  if (analysis === undefined || flush === undefined || parse === undefined) return null
  return analysis + flush + parse
}

function main(): void {
  fs.mkdirSync(RESULTS, { recursive: true })
  const records: Record<Core, Baseline[]> = { js: [], native: [] }
  for (let r = 0; r < runs; r++) {
    const order: Core[] = r % 2 === 0 ? ['js', 'native'] : ['native', 'js']
    for (const core of order) {
      process.stderr.write(`\ncompare run ${r + 1}/${runs} · VORN_CORE=${core}\n`)
      records[core].push(runOnce(core, r))
    }
  }
  const js = merge(records.js)
  const nat = merge(records.native)
  for (const [core, record] of [
    ['js', js],
    ['native', nat]
  ] as const) {
    fs.writeFileSync(
      path.join(RESULTS, `compare-${core}.json`),
      JSON.stringify(record, null, 2) + '\n'
    )
  }

  const rows: string[] = ['| Metric | JS | Native | Native vs JS |', '| --- | ---: | ---: | ---: |']
  for (const name of ['agent', 'spinner', 'bulk']) {
    const a = perMbTotal(js, name, false)
    const b = perMbTotal(nat, name, true)
    if (a === null || b === null) continue
    const unit = {
      value: 0,
      unit: 'ms/MB',
      better: 'lower' as const,
      label: '',
      samples: [],
      spreadPct: 0
    }
    rows.push(
      `| **server total per MB, ${name}** | ${a.toFixed(2)} ms/MB | ${b.toFixed(2)} ms/MB | ${gain({ ...unit, value: a }, { ...unit, value: b })} |`
    )
  }
  const suites = new Set([...Object.keys(js.suites), ...Object.keys(nat.suites)])
  for (const suite of suites) {
    const names = new Set([
      ...Object.keys(js.suites[suite]?.metrics ?? {}),
      ...Object.keys(nat.suites[suite]?.metrics ?? {})
    ])
    for (const name of names) {
      const a = js.suites[suite]?.metrics[name]
      const b = nat.suites[suite]?.metrics[name]
      rows.push(`| \`${suite}/${name}\` | ${fmt(a)} | ${fmt(b)} | ${gain(a, b)} |`)
    }
  }
  const m = js.machine
  const doc = [
    `# JS vs native core`,
    '',
    `${m.cpu}, ${m.cores} cores, ${m.memoryGB} GB, Node ${m.node}. ` +
      `Recorded ${js.recordedAt.slice(0, 10)} at \`${js.commit}\`, median of ${js.runs} run(s) per core, alternating which goes first.`,
    '',
    ...rows,
    ''
  ].join('\n')
  fs.writeFileSync(path.join(RESULTS, 'compare.md'), doc)
  console.log('\n' + doc)
}

main()
