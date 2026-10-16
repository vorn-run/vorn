// Turns the bench's results.json into the markdown tables the report quotes:
// each number is the median of the runs. `node summary.mjs results.json`
import { readFileSync } from 'node:fs'

const r = JSON.parse(readFileSync(process.argv[2] ?? 'results/results.json', 'utf8'))
const tiers = ['builtin', 'program', 'wasm']
const median = (xs) => {
  const v = xs.filter((x) => typeof x === 'number').sort((a, b) => a - b)
  if (!v.length) return null
  const m = Math.floor(v.length / 2)
  return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2
}
const at = (o, path) => path.split('.').reduce((x, k) => (x == null ? x : x[k]), o)
const fmt = (x, digits = 1) =>
  x == null ? '—' : x >= 10000 ? Math.round(x).toLocaleString('en-US') : x.toFixed(digits)
const of = (ext, tier, path) => median(r.perf.filter((p) => p.ext === ext && p.tier === tier).map((p) => at(p, path)))

const out = []
const table = (title, rows) => {
  out.push(`| ${title} | built-in | program | Wasm |`, '|---|---:|---:|---:|')
  for (const [label, ext, path, digits] of rows) {
    out.push(`| ${label} | ${tiers.map((t) => fmt(of(ext, t, path), digits)).join(' | ')} |`)
  }
  out.push('')
}

out.push(`Machine: ${r.machine.cpu}, ${r.machine.cores} cores, ${r.machine.os}; medians of ${r.runs} runs.`, '')
for (const [ext, extra] of [
  ['worktrees', [
    ['scan p50 (µs)', 'scan_us.p50'],
    ['scan p99 (µs)', 'scan_us.p99'],
    ['5k worktree.idle: events/s', 'idle_events.events_per_s'],
    ['5k worktree.idle: CPU µs/event', 'idle_events.cpu_us_per_event'],
  ]],
  ['widget', [
    ['event → row p50 (µs)', 'event_to_row_us.p50'],
    ['event → row p99 (µs)', 'event_to_row_us.p99'],
    ['20k session.status: events/s', 'status_events.events_per_s'],
    ['20k session.status: CPU µs/event', 'status_events.cpu_us_per_event'],
    ['20k session.status: guest calls', 'status_events.publishes', 0],
    ['5k agent.permission: events/s', 'permission_events.events_per_s'],
    ['5k agent.permission: CPU µs/event', 'permission_events.cpu_us_per_event'],
  ]],
]) {
  out.push(`### ${ext === 'worktrees' ? 'Worktree cleanup' : 'Widget'}`, '')
  table('', [
    ['first start (ms)', ext, 'cold_first_ms', 2],
    ['restart p50 (ms)', ext, 'cold_ms.p50', 2],
    ['cold start: event → first publish p50 (ms)', ext, 'cold_event_to_publish_ms.p50', 2],
    ['call round trip p50 (µs)', ext, 'echo_us.p50'],
    ['call round trip p99 (µs)', ext, 'echo_us.p99'],
    ...extra.map(([l, p, d]) => [l, ext, p, d]),
    ['memory idle (MB)', ext, 'idle.total_mb', 2],
    ['memory busy (MB)', ext, 'busy.total_mb', 2],
    ['of which linear memory, busy (MB)', ext, 'busy.linear_mb', 2],
  ])
}

out.push('### Same rows', '', '| extension | steps | program: same rows / same order | Wasm: same rows / same order |', '|---|---:|---:|---:|')
for (const [ext, s] of Object.entries(r.same_rows)) {
  const cell = (t) => `${s[t].same_rows}/${s[t].runs} · ${s[t].same_order}/${s[t].runs}`
  out.push(`| ${ext} | ${s.steps} | ${cell('program')} | ${cell('wasm')} |`)
}
out.push('')

out.push('### Crashes', '', '| | program | Wasm |', '|---|---|---|')
const [p, w] = r.crash
const said = (x) => (x.ok !== undefined ? 'answered' : `\`${x.err}\``)
const crashRows = [
  ['endless loop', (c) => `${said(c.spin)} after ${fmt(c.spin_ms, 0)} ms`],
  ['grab 256 MB', (c) => (c.grow_256mb.ok !== undefined ? 'allowed' : `refused (${c.grow_256mb.err})`)],
  ['restart after a crash p50 (ms)', (c) => fmt(median(c.crashes.map((x) => (x.next.ok !== undefined ? x.next_ms : null))), 2)],
  // The grab counts when it was refused: that ended the guest too.
  ['crashes (the grab included) before the budget holds it', (c) =>
    String(c.crashes.findIndex((x) => x.next.err === 'start') + 1 + (c.grow_256mb.ok === undefined ? 1 : 0))],
  ['host still answers', (c) => (c.host_alive ? 'yes' : 'no')],
]
for (const [label, f] of crashRows) out.push(`| ${label} | ${f(p)} | ${f(w)} |`)
out.push('')

const kb = (b) => (b == null ? '—' : `${Math.round(b / 1024).toLocaleString('en-US')} KB`)
const s = r.sizes
out.push('### Sizes', '', '| artifact | size |', '|---|---:|')
out.push(`| program binary (both extensions) | ${kb(s.program)} |`)
out.push(`| component .wasm (both extensions) | ${kb(s.component_wasm)} |`)
out.push(`| component .cwasm (precompiled) | ${kb(s.component_cwasm)} |`)
if (s.probe_without_wasm) {
  const more = (b) => (b == null ? '—' : `${kb(b)} (+${kb(b - s.probe_without_wasm)})`)
  out.push(`| host without the Wasm tier | ${kb(s.probe_without_wasm)} |`)
  out.push(`| host with its runtime (loads .cwasm) | ${more(s.probe_with_wasm)} |`)
  out.push(`| host with runtime and compiler (loads .wasm) | ${more(s.probe_with_compiler)} |`)
}
out.push('', '### Lines of code', '', '| part | lines |', '|---|---:|')
for (const [k, v] of Object.entries(r.loc)) out.push(`| ${k.replaceAll('_', ' ')} | ${v} |`)
console.log(out.join('\n'))
