#!/usr/bin/env node
// Seeds a TEST vornd with workflows and runs for the Workflows view.
// usage: node seed-test-vornd.mjs <data-dir>   (never the real ~/.vorn)
import { readFileSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const dir = process.argv[2]
if (!dir) {
  console.error('usage: seed-test-vornd.mjs <data-dir>')
  process.exit(2)
}
if (resolve(dir) === join(homedir(), '.vorn')) {
  console.error('refusing to seed the real data directory')
  process.exit(2)
}
const { port } = JSON.parse(readFileSync(join(dir, 'ws-port'), 'utf8'))
const token = readFileSync(join(dir, 'local-token'), 'utf8').trim()
const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
let next = 1
const pending = new Map()
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data)
  if (msg.id != null && pending.has(msg.id)) {
    const { ok, fail } = pending.get(msg.id)
    pending.delete(msg.id)
    msg.error ? fail(new Error(`${msg.error.message}`)) : ok(msg.result)
  }
}
const call = (method, params) =>
  new Promise((ok, fail) => {
    const id = next++
    pending.set(id, { ok, fail })
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
await new Promise((r) => (ws.onopen = r))
await call('auth:authenticate', { token })

const cwd = tmpdir()
const at = (x, y) => ({ x, y })
const trigger = (config) => ({ id: 'trigger', type: 'trigger', label: 'Trigger', config, position: at(0, 0) })
const script = (id, label, body, y) => ({
  id,
  type: 'script',
  label,
  slug: id,
  config: { scriptType: 'bash', scriptContent: body, cwd, projectPath: cwd },
  position: at(0, y)
})
const chain = (ids) => ids.slice(1).map((t, i) => ({ id: `e${i}`, source: ids[i], target: t }))
const workflow = (id, name, icon, iconColor, nodes, enabled = true) => ({
  id,
  name,
  icon,
  iconColor,
  nodes,
  edges: chain(nodes.map((n) => n.id)),
  enabled,
  workspaceId: 'personal'
})

const workflows = [
  workflow('wf-release', 'Release checklist', 'Rocket', '#c9972a', [
    trigger({ triggerType: 'manual' }),
    script('build', 'Build', 'echo building; sleep 0.4; echo done', 120),
    {
      id: 'gate',
      type: 'approval',
      label: 'Sign off',
      slug: 'gate',
      config: { message: 'Build is green. Ship version 0.9 to the beta channel?' },
      position: at(0, 240)
    },
    script('publish', 'Publish', 'echo published', 360)
  ]),
  workflow('wf-lint', 'Lint the repo', 'Code', '#6f8faf', [
    trigger({ triggerType: 'manual' }),
    script('lint', 'Run lint', 'echo "3 files checked"; sleep 0.3', 120),
    script('report', 'Report', 'echo all clean', 240)
  ]),
  workflow('wf-flaky', 'Flaky integration', 'FlaskConical', '#d4623f', [
    trigger({ triggerType: 'manual' }),
    script('setup', 'Set up fixtures', 'echo ready', 120),
    script('tests', 'Integration tests', 'echo "running 42 tests"; echo "1 failed" >&2; exit 1', 240)
  ]),
  workflow('wf-nightly', 'Nightly digest', 'Zap', '#7d9471', [
    trigger({ triggerType: 'recurring', cron: '0 3 * * *' }),
    script('digest', 'Write digest', 'echo digest', 120)
  ]),
  workflow(
    'wf-weekly',
    'Weekly cleanup',
    'Database',
    '#7d8590',
    [trigger({ triggerType: 'recurring', cron: '0 9 * * 1' }), script('clean', 'Prune', 'echo pruned', 120)],
    false
  )
]

const existing = new Set((await call('workflow:list')).map((w) => w.id))
for (const wf of workflows) {
  if (existing.has(wf.id)) await call('workflow:update', { id: wf.id, updates: wf })
  else await call('workflow:create', { workflow: wf })
}
if (!process.argv.includes('--no-runs')) {
  for (const id of ['wf-lint', 'wf-flaky', 'wf-release', 'wf-lint', 'wf-release']) {
    const run = await call('workflow:run', { workflowId: id })
    console.log('started', id, run?.runId)
    await sleep(1200)
  }
  await sleep(1500)
}
const runs = await call('workflowRun:listAll', { workspaceId: 'personal', limit: 50 })
console.log(
  runs.map((r) => `${r.workflowId} ${r.status} ${r.nodeStates.map((n) => `${n.nodeId}:${n.status}`).join(',')}`).join('\n')
)
ws.close()
