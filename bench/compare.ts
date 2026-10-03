/**
 * `yarn bench:compare`: the same suites with `VORN_CORE=js` and `VORN_CORE=native`,
 * side by side.
 *
 *   yarn bench:compare              three runs each
 *   yarn bench:compare --runs=5
 *   yarn bench:compare --quick      a smoke check
 *
 * Writes `bench/results/compare.md` and the two raw records beside it. Each
 * core runs in its own child `yarn bench`, so neither can warm the other.
 */
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type { Baseline, Summary } from './run'

const ROOT = path.resolve(__dirname, '..')
const RESULTS = path.join(ROOT, 'bench', 'results')
const SUITES = 'output-analysis,screen-model,flush,event-loop,memory'

const passthrough = process.argv.slice(2).filter((a) => !a.startsWith('--only'))
const only = process.argv.find((a) => a.startsWith('--only='))?.slice(7) ?? SUITES

function run(core: 'js' | 'native'): Baseline {
  const out = path.join(RESULTS, `compare-${core}.json`)
  const r = spawnSync(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(ROOT, 'bench', 'run.ts'),
      `--only=${only}`,
      `--out=${out}`,
      ...passthrough
    ],
    { cwd: ROOT, stdio: ['ignore', 'inherit', 'inherit'], env: { ...process.env, VORN_CORE: core } }
  )
  if (r.status !== 0) throw new Error(`bench with VORN_CORE=${core} failed`)
  return JSON.parse(fs.readFileSync(out, 'utf-8')) as Baseline
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
  const js = run('js')
  const nat = run('native')

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
      `Recorded ${js.recordedAt.slice(0, 10)} at \`${js.commit}\`, median of ${js.runs} run(s) per core.`,
    '',
    ...rows,
    ''
  ].join('\n')
  fs.writeFileSync(path.join(RESULTS, 'compare.md'), doc)
  console.log('\n' + doc)
}

main()
