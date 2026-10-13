#!/usr/bin/env node
// Seeds a TEST vornd with workflows and runs for the Workflows view.
// usage: node seed-test-vornd.mjs <data-dir> [--no-runs] [--review] [--shell <cwd>] [--shells N]   (never the real ~/.vorn)
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
const trigger = (config) => ({
  id: 'trigger',
  type: 'trigger',
  label: 'Trigger',
  config,
  position: at(0, 0)
})
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
    script(
      'tests',
      'Integration tests',
      'echo "running 42 tests"; echo "1 failed" >&2; exit 1',
      240
    )
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
    [
      trigger({ triggerType: 'recurring', cron: '0 9 * * 1' }),
      script('clean', 'Prune', 'echo pruned', 120)
    ],
    false
  )
]

// A gate with a review page, built by the step before it, that can send the work back once.
if (process.argv.includes('--review')) {
  workflows.push(
    workflow('wf-review', 'Review it', 'Hand', '#c9972a', [
      trigger({ triggerType: 'manual' }),
      script('find', 'Find issues', "echo '<h1>Findings</h1><p>found</p>'", 120),
      {
        id: 'check',
        type: 'approval',
        label: 'Check findings',
        slug: 'check',
        config: {
          message: 'Look over the findings.',
          view: '{{steps.find.output}}',
          edit: '{{steps.find.output}}',
          feedback: { from: 'find', maxRounds: 2 }
        },
        position: at(0, 240)
      }
    ])
  )
}

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
if (process.argv.includes('--review')) {
  const run = await call('workflow:run', { workflowId: 'wf-review' })
  console.log('started wf-review', run?.runId)
  await sleep(1500)
}
const shellAt = process.argv.indexOf('--shell')
const shellsAt = process.argv.indexOf('--shells')
const shellCount = shellsAt > 0 ? Number(process.argv[shellsAt + 1]) : 1
const shells = []
if (shellAt > 0) {
  for (let i = 0; i < shellCount; i++) {
    shells.push(await call('shell:create', process.argv[shellAt + 1]))
  }
}
// --artifacts: a doc with two versions and comments, and a design with artboards and tweaks, from the first shell.
if (process.argv.includes('--artifacts') && shells[0]) {
  const sessionId = shells[0].id
  const plan =
    '# Launch plan\n\nWe ship the native Mac app to a small group first, then widen it once crash reports stay quiet for a week.\n\nThe browser pane, artifacts and the device pane come over in this wave.\n\n- Browser pane\n- Artifacts and comments\n- Device pane\n'
  const doc = await call('artifact:publish', {
    sessionId,
    title: 'Launch plan',
    kind: 'doc',
    content: plan,
    open: false
  })
  await call('artifact:publish', {
    sessionId,
    artifactId: doc.artifact.id,
    title: 'Launch plan',
    kind: 'doc',
    content: plan.replace('a small group', 'a small group of testers'),
    open: false
  })
  await call('artifact:saveComment', {
    artifactId: doc.artifact.id,
    version: 2,
    anchor: {
      kind: 'quote',
      quote: 'crash reports stay quiet',
      prefix: 'once ',
      suffix: ' for a week'
    },
    body: 'How quiet is quiet? Give a number.'
  })
  await call('artifact:saveComment', {
    artifactId: doc.artifact.id,
    version: 2,
    anchor: null,
    body: 'Add a rollback step.'
  })
  const design = `<!doctype html><html><head><script type="application/json" id="artifact">
{"kind":"design","title":"Pricing hero","tweaks":{"dark":{"type":"boolean","default":true,"label":"Dark"},"accent":{"type":"color","default":"#c9a26b","label":"Accent"},"size":{"type":"number","default":44,"min":28,"max":64,"unit":"px","label":"Title size"}},
"artboards":[{"id":"desktop","label":"Desktop","width":1280,"height":800},{"id":"phone","label":"Phone","width":390,"height":844}]}
</script><style>body{margin:0;font:16px -apple-system,sans-serif;background:#0b0b0c;color:#eee;display:flex;align-items:center;justify-content:center;height:100vh}h1{font-size:44px;margin:0 0 12px}a{color:#c9a26b}</style></head>
<body><div><h1>Simple pricing</h1><p>One plan, every agent. <a href="#">Start free</a></p></div></body></html>`
  await call('artifact:publish', {
    sessionId,
    title: 'Pricing hero',
    kind: 'design',
    content: design,
    open: false
  })
}
const runs = await call('workflowRun:listAll', { workspaceId: 'personal', limit: 50 })
console.log(
  runs
    .map(
      (r) =>
        `${r.workflowId} ${r.status} ${r.nodeStates.map((n) => `${n.nodeId}:${n.status}`).join(',')}`
    )
    .join('\n')
)
ws.close()
