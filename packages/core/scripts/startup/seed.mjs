// node seed.mjs <dataDir> <root>: fills a running vornd as a busy person's
// would be: 10 projects, 500 tasks, 22 workflows and 30 live sessions, each
// with 8,000 lines of scrollback, from an agent that then waits.
import { readFileSync, mkdirSync, writeFileSync, chmodSync } from 'node:fs'
import { join } from 'node:path'

const [dir, root] = process.argv.slice(2)
const portFile = readFileSync(join(dir, 'ws-port'), 'utf8').trim()
const port = portFile.startsWith('{') ? JSON.parse(portFile).port : Number(portFile)
const token = readFileSync(join(dir, 'local-token'), 'utf8').trim()

const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
const waiting = new Map()
let next = 1
ws.onmessage = (m) => {
  const f = JSON.parse(m.data)
  const w = waiting.get(f.id)
  if (!w) return
  waiting.delete(f.id)
  if (f.error) w.reject(new Error(`${w.method}: ${f.error.message}`))
  else w.resolve(f.result)
}
const call = (method, params = null) =>
  new Promise((resolve, reject) => {
    const id = next++
    waiting.set(id, { resolve, reject, method })
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
await new Promise((ok, fail) => {
  ws.onopen = ok
  ws.onerror = fail
})
await call('auth:authenticate', { token })

// The agent every session runs: a long history, then nothing until stopped.
const bin = join(root, 'bin')
mkdirSync(bin, { recursive: true })
const agent = join(bin, 'bench-agent')
writeFileSync(
  agent,
  "#!/bin/sh\nseq 1 8000 | sed 's/$/ lorem ipsum dolor sit amet, a line of scrollback/'\nexec sleep 100000\n"
)
chmodSync(agent, 0o755)

const projects = Array.from({ length: 10 }, (_, i) => {
  const path = join(root, 'p', `proj${i + 1}`)
  mkdirSync(path, { recursive: true })
  return { name: `proj${i + 1}`, path, preferredAgents: [] }
})
const config = await call('config:load')
config.projects = projects
config.defaults = { ...(config.defaults ?? {}), hasSeenOnboarding: '9', defaultAgent: 'claude' }
config.agentCommands = { ...(config.agentCommands ?? {}), claude: { command: agent, args: [] } }
await call('config:save', config)

const statuses = ['todo', 'in_progress', 'done']
for (let i = 0; i < 500; i++) {
  await call('task:create', {
    projectName: projects[i % 10].name,
    title: `Task ${i}`,
    description: 'd'.repeat(400),
    status: statuses[i % 3]
  })
}
let workflows = 0
for (let i = 0; i < 22; i++) {
  try {
    await call('workflow:create', {
      workflow: {
        id: crypto.randomUUID(),
        enabled: true,
        name: `Workflow ${i}`,
        icon: 'Zap',
        iconColor: '#6366f1',
        nodes: [
          {
            id: 'trigger',
            type: 'trigger',
            position: { x: 0, y: 0 },
            data: { triggerType: 'manual' }
          }
        ],
        edges: []
      }
    })
    workflows++
  } catch (e) {
    console.error(String(e))
  }
}
for (let i = 0; i < 30; i++) {
  const p = projects[i % 10]
  await call('terminal:create', { agentType: 'claude', projectName: p.name, projectPath: p.path })
}
console.log(`seeded 10 projects, 500 tasks, ${workflows} workflows, 30 sessions`)
ws.close()
