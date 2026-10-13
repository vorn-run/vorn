#!/usr/bin/env node
// Seeds a TEST vornd for Settings › Plugins: a catalog cache and installed packs (before vornd starts),
// then connections and a seeded workflow (once it runs). Nothing here reaches the network.
// usage: node seed-plugins.mjs files <home> <data-dir>
//        node seed-plugins.mjs connections <data-dir>
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'

const [mode, ...rest] = process.argv.slice(2)
for (const dir of rest) {
  if (resolve(dir).startsWith(join(homedir(), '.vorn'))) {
    console.error('refusing to seed the real data directory')
    process.exit(2)
  }
}

const ticketIcon = { viewBox: '0 0 24 24', paths: ['M3 5h18v14H3zM5 7v10h14V7z', 'M7 10h10v2H7z', 'M7 13h6v2H7z'] }
const tickets = {
  id: 'acme-tickets',
  name: 'Acme Tickets',
  version: '1.2.0',
  protocol: 1,
  description: 'Start a workflow when a ticket lands in an Acme queue.',
  icon: ticketIcon,
  auth: { rung: 'key' },
  triggers: [{ type: 'ticket.created', label: 'New ticket', description: 'A ticket was opened in the queue.' }],
  actions: [
    { type: 'ticket.comment', label: 'Comment on a ticket' },
    { type: 'ticket.close', label: 'Close a ticket' }
  ],
  env: [{ name: 'ACME_QUEUE', required: true, description: 'The queue to watch.' }]
}
const timer = {
  id: 'focus-timer',
  name: 'Focus Timer',
  version: '0.3.1',
  protocol: 1,
  kind: 'extension',
  description: 'Shows how long the agent has been working on the card.',
  contributes: {
    footers: [{ id: 'elapsed', title: 'Elapsed time', every: 30 }],
    panes: [{ id: 'log', title: 'Focus log', web: 'pane/index.html' }]
  },
  permissions: ['terminal.read', 'agent.usage'],
  activates: { agent: ['claude'] },
  triggers: [],
  actions: [],
  env: []
}

if (mode === 'files') {
  const [home, data] = rest
  mkdirSync(join(home, '.vorn'), { recursive: true })
  writeFileSync(
    join(home, '.vorn', 'connector-catalog.json'),
    JSON.stringify({
      fetchedAt: Date.now(),
      connectors: [
        {
          id: 'acme-tickets',
          name: 'Acme Tickets',
          description: tickets.description,
          packageName: '@acme/connector-tickets',
          version: '1.3.0',
          capabilities: ['triggers', 'actions'],
          category: 'Development',
          authRung: 'key',
          icon: ticketIcon,
          keywords: ['tickets', 'queue']
        },
        {
          id: 'kusto',
          name: 'Azure Data Explorer',
          description: 'Run a KQL query on a schedule and act on the rows it returns.',
          packageName: '@vornrun/connector-kusto',
          version: '0.4.0',
          capabilities: ['triggers', 'actions'],
          category: 'Data',
          authRung: 'cli',
          verified: { schema: 1, version: '0.4.0', checkedAt: '2025-01-10T00:00:00Z', checks: ['installs', 'starts'] },
          keywords: ['kql', 'adx']
        },
        {
          id: 'standup-notes',
          name: 'Standup Notes',
          description: 'Posts what each card did since yesterday.',
          packageName: '@acme/connector-standup',
          capabilities: ['actions'],
          category: 'Productivity',
          authRung: 'none'
        },
        {
          id: 'focus-timer',
          name: 'Focus Timer',
          kind: 'extension',
          description: timer.description,
          packageName: '@acme/extension-focus-timer',
          version: '0.3.1',
          category: 'Productivity',
          contributes: timer.contributes,
          permissions: timer.permissions
        }
      ],
      templates: [],
      mcpServers: [
        {
          id: 'filesystem',
          name: 'Filesystem',
          description: 'Read and write files under a folder.',
          command: 'npx',
          args: ['-y', '@modelcontextprotocol/server-filesystem', '.']
        }
      ]
    })
  )
  for (const pack of [tickets, timer]) {
    const dir = join(data, 'connectors', pack.id)
    mkdirSync(join(dir, pack.version), { recursive: true })
    writeFileSync(join(dir, pack.version, 'manifest.json'), JSON.stringify(pack))
    writeFileSync(join(dir, pack.version, 'index.js'), 'process.stdin.resume()\n')
    writeFileSync(join(dir, 'current.json'), JSON.stringify({ version: pack.version, installedAt: Date.now() }))
  }
} else if (mode === 'connections') {
  const [data] = rest
  const { port } = JSON.parse(readFileSync(join(data, 'ws-port'), 'utf8'))
  const token = readFileSync(join(data, 'local-token'), 'utf8').trim()
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
  await new Promise((r) => (ws.onopen = r))
  await call('auth:authenticate', { token })

  await call('connection:create', {
    connectorId: 'http',
    name: 'Status page',
    filters: { profileName: 'Status page', baseUrl: 'https://status.example.com/api' },
    syncIntervalMinutes: 5,
    statusMapping: {}
  })
  const sdk = await call('connection:create', {
    connectorId: 'sdk',
    name: 'Acme Tickets: New ticket',
    filters: {
      command: 'node',
      args: JSON.stringify([join(data, 'connectors', 'acme-tickets', tickets.version, 'index.js')]),
      env: JSON.stringify({ ACME_QUEUE: 'support' }),
      sdkConnectorId: 'acme-tickets',
      sdkVersion: tickets.version,
      sdkIcon: JSON.stringify(ticketIcon),
      sdkTrigger: 'ticket.created'
    },
    syncIntervalMinutes: 5,
    statusMapping: {}
  })
  await call('workflow:create', {
    workflow: {
      id: `connector:${sdk.id}:ticket.created`,
      name: 'New Acme ticket',
      icon: 'Workflow',
      iconColor: '#8b8b8b',
      nodes: [
        { id: 'trigger', type: 'trigger', label: 'Trigger', position: { x: 0, y: 0 }, config: { triggerType: 'cron', cron: '*/5 * * * *' } }
      ],
      edges: [],
      enabled: true,
      workspaceId: 'personal'
    }
  })
  ws.close()
} else {
  console.error('usage: seed-plugins.mjs files <home> <data-dir> | connections <data-dir>')
  process.exit(2)
}
