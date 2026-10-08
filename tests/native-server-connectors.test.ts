/**
 * Connections and connectors, answered by vornd through a real server and
 * the vornd it keeps.
 *
 * An MCP connection runs the fixture MCP server with secrets vornd keeps in
 * its vault; a pack is inspected, installed and rolled back, and its
 * connection runs the native connector fixture; a connector poll fills a
 * workflow's inbox and the workflow runs once per item however often it is
 * polled; secrets the desktop sealed are imported once; an HTTP profile signs
 * a request to a local server. None of these calls reaches the server.
 *
 * Runs where vornd and its session holder have been built.
 */
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { gzipSync } from 'node:zlib'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { queryDb } from './helpers/database'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until
} from './helpers/real-server'

vi.setConfig({ testTimeout: 90_000, hookTimeout: 120_000 })

const MCP_FIXTURE = path.resolve(__dirname, 'fixtures/mcp-server.mjs')
const NATIVE_FIXTURE = path.resolve(__dirname, 'fixtures/native-connector.mjs')
const IN_VAULT = 'vorn-vault'

interface Connection {
  id: string
  connectorId: string
  name: string
  filters: Record<string, unknown>
  lastSyncError?: string
}

/** A tar archive of `files`, gzipped: what a `.vorn.tgz` pack is. */
function tgz(files: Record<string, string>): Buffer {
  const blocks: Buffer[] = []
  for (const [name, text] of Object.entries(files)) {
    const body = Buffer.from(text)
    const header = Buffer.alloc(512)
    header.write(name, 0, 100)
    header.write('0000644\0', 100)
    header.write('0000000\0', 108)
    header.write('0000000\0', 116)
    header.write(body.length.toString(8).padStart(11, '0') + '\0', 124)
    header.write('00000000000\0', 136)
    header.write('        ', 148)
    header.write('0', 156)
    header.write('ustar\0', 257)
    header.write('00', 263)
    let sum = 0
    for (const byte of header) sum += byte
    header.write(sum.toString(8).padStart(6, '0') + '\0 ', 148)
    blocks.push(header, body, Buffer.alloc((512 - (body.length % 512)) % 512))
  }
  blocks.push(Buffer.alloc(1024))
  return gzipSync(Buffer.concat(blocks))
}

function pack(version: string): Buffer {
  return tgz({
    'manifest.json': JSON.stringify({
      protocol: 1,
      id: 'tickets',
      name: 'Tickets',
      version,
      triggers: [{ type: 'items', label: 'Items' }],
      actions: [{ type: 'create', label: 'Create', inputs: [{ key: 'title', label: 'Title' }] }]
    }),
    'index.js': fs.readFileSync(NATIVE_FIXTURE, 'utf-8'),
    'package.json': JSON.stringify({ type: 'module' })
  })
}

describe.runIf(runnable)('connections and connectors in vornd', () => {
  let server: RealServer
  let client: Watcher
  let profileServer: http.Server
  let profilePort = 0
  const seen: Array<{ url?: string; auth?: string }> = []

  const db = (): string => path.join(server.dirs.data, 'vorn.db')
  const create = (params: Record<string, unknown>): Promise<Connection> =>
    client.result<Connection>('connection:create', {
      syncIntervalMinutes: 0,
      statusMapping: {},
      ...params
    })

  beforeAll(async () => {
    profileServer = http.createServer((req, res) => {
      seen.push({ url: req.url, auth: req.headers.authorization })
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end('{"hello":"world"}')
    })
    await new Promise<void>((r) => profileServer.listen(0, '127.0.0.1', r))
    profilePort = (profileServer.address() as { port: number }).port
    server = await startRealServer()
    client = await Watcher.open(server.vornd)
  })

  afterAll(async () => {
    client?.close()
    profileServer?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('keeps an MCP connection’s secrets in the vault and runs with them', async () => {
    const conn = await create({
      connectorId: 'mcp',
      name: 'Fixture',
      filters: {
        command: process.execPath,
        args: JSON.stringify([MCP_FIXTURE]),
        env: JSON.stringify({ FIXTURE_PLAIN: 'plain value' }),
        secretEnv: JSON.stringify({ FIXTURE_SECRET: 'first' })
      }
    })
    // The row never holds the secret.
    expect(conn.filters.secretEnv).toBe(IN_VAULT)
    const row = queryDb(db(), (d) =>
      d.prepare('SELECT filters FROM source_connections WHERE id = ?').get(conn.id)
    ) as { filters: string }
    expect(row.filters).not.toContain('first')

    expect(await client.result('connection:refreshMcpTools', conn.id)).toMatchObject({ ok: true })
    const actions = await client.result<Array<{ type: string }>>('connection:listActions', conn.id)
    expect(actions.map((a) => a.type)).toContain('secret')
    const run = (): Promise<unknown> =>
      client.result('connection:executeAction', {
        connectionId: conn.id,
        action: 'secret',
        args: {}
      })
    expect(JSON.stringify(await run())).toContain('first')

    const keys =
      await client.result<Array<{ connectionId: string; fields: unknown[] }>>('connection:listKeys')
    expect(keys.find((k) => k.connectionId === conn.id)?.fields).toEqual([
      {
        key: 'secretEnv',
        label: 'Secret env (JSON object)',
        readable: true,
        envNames: ['FIXTURE_SECRET']
      }
    ])

    expect(
      await client.result('connection:rotateSecret', {
        connectionId: conn.id,
        field: 'secretEnv',
        plaintext: JSON.stringify({ FIXTURE_SECRET: 'second' })
      })
    ).toEqual({ ok: true })
    expect(JSON.stringify(await run())).toContain('second')
    expect(
      await client.result('connection:rotateSecret', {
        connectionId: conn.id,
        field: 'command',
        plaintext: 'x'
      })
    ).toEqual({ ok: false, error: 'command is not a secret on this connection' })

    await client.result('connection:delete', conn.id)
    expect(
      await client.result('connection:executeAction', { connectionId: conn.id, action: 'secret' })
    ).toEqual({
      success: false,
      error: `Connection ${conn.id} not found`
    })
  })

  it('imports a secret the desktop sealed, once', async () => {
    const conn = await create({
      connectorId: 'http',
      name: 'Legacy',
      filters: {
        baseUrl: `http://127.0.0.1:${profilePort}`,
        authHeader: 'Authorization: Bearer {{secret}}'
      }
    })
    // A row from before the vault: the secret is sealed text only the desktop can read.
    queryDb(db(), (d) =>
      d.prepare('UPDATE source_connections SET filters = ? WHERE id = ?').run(
        JSON.stringify({
          baseUrl: `http://127.0.0.1:${profilePort}`,
          authHeader: 'Authorization: Bearer {{secret}}',
          secret: 'sealed-by-the-desktop'
        }),
        conn.id
      )
    )
    const locked = await client.result('http:request', {
      profileConnectionId: conn.id,
      method: 'GET',
      url: '/x'
    })
    expect(locked).toMatchObject({ success: false })

    expect(
      await client.result('credentials:import', {
        connections: { [conn.id]: { secret: 'tok-123' } }
      })
    ).toEqual({ connections: 1 })
    const rows = await client.result<Connection[]>('connection:list', {})
    expect(rows.find((c) => c.id === conn.id)?.filters.secret).toBe(IN_VAULT)

    seen.length = 0
    const sent = await client.result<{
      success: boolean
      output: { status: number; body: unknown }
    }>('http:request', {
      profileConnectionId: conn.id,
      method: 'GET',
      url: '/items'
    })
    expect(sent).toMatchObject({ success: true, output: { status: 200, body: { hello: 'world' } } })
    expect(seen).toEqual([{ url: '/items', auth: 'Bearer tok-123' }])
    // A profile signs only its own origin.
    const elsewhere = await client.result<{ error: string }>('http:request', {
      profileConnectionId: conn.id,
      method: 'GET',
      url: 'http://localhost:1/'
    })
    expect(elsewhere.error).toMatch(/only signs requests to/)
  })

  it('installs a pack, runs its connection, and polls it into a workflow once per item', async () => {
    const file = path.join(server.dirs.work, 'tickets-1.vorn.tgz')
    fs.writeFileSync(file, pack('1.0.0'))
    const preview = await client.result<{ ok: boolean; preview: { id: string; token: string } }>(
      'connector:inspectPack',
      { kind: 'file', path: file }
    )
    expect(preview).toMatchObject({ ok: true, preview: { id: 'tickets' } })
    const installed = await client.result('connector:installPack', {
      kind: 'staged',
      token: preview.preview.token
    })
    expect(installed).toMatchObject({ ok: true, pack: { id: 'tickets', version: '1.0.0' } })
    fs.writeFileSync(file, pack('2.0.0'))
    await client.result('connector:installPack', { kind: 'file', path: file })
    expect(await client.result('connector:rollbackPack', 'tickets')).toMatchObject({
      ok: true,
      pack: { version: '1.0.0', previousVersion: '2.0.0' }
    })
    const packs = await client.result<Array<{ id: string }>>('connector:listPacks')
    expect(packs.map((p) => p.id)).toEqual(['tickets'])

    const conn = await create({
      connectorId: 'sdk',
      name: 'Tickets',
      filters: { sdkConnectorId: 'tickets', sdkVersion: '1.0.0', sdkTrigger: 'items' }
    })
    const actions = await client.result<Array<{ type: string }>>('connection:listActions', conn.id)
    expect(actions.map((a) => a.type)).toEqual(['create'])
    expect(
      await client.result('connection:executeAction', {
        connectionId: conn.id,
        action: 'create',
        args: { title: 'T' }
      })
    ).toMatchObject({ success: true, output: { echo: { title: 'T' } } })
    expect(
      await client.result('connection:executeAction', {
        connectionId: conn.id,
        action: 'create',
        args: { fail: true }
      })
    ).toEqual({ success: false, error: 'the upstream said no' })
    expect(await client.result('connection:preflight', conn.id)).toEqual({
      ok: true,
      message: 'ready'
    })

    const workflow = {
      id: 'wf-poll',
      name: 'Poll tickets',
      icon: 'Zap',
      iconColor: '#ffffff',
      enabled: true,
      nodes: [
        {
          id: 't',
          type: 'trigger',
          label: 'Trigger',
          position: { x: 0, y: 0 },
          config: {
            triggerType: 'connectorPoll',
            connectionId: conn.id,
            event: 'mcpPoll',
            cron: '0 0 1 1 *'
          }
        },
        {
          id: 's',
          type: 'script',
          label: 's',
          slug: 's',
          position: { x: 0, y: 100 },
          config: { scriptType: 'bash', scriptContent: 'echo polled' }
        }
      ],
      edges: [{ id: 'e', source: 't', target: 's' }]
    }
    await client.result('workflow:create', { workflow })
    expect(await client.result('connector:poll', { workflowId: 'wf-poll' })).toEqual({ pages: 1 })
    expect(await client.result('connector:poll', { workflowId: 'wf-poll' })).toEqual({ pages: 1 })
    await until('the polled item to run', async () => {
      const runs = await client.result<Array<{ status: string }>>('workflowRun:list', {
        workflowId: 'wf-poll'
      })
      return runs.length >= 1 && runs.every((r) => r.status !== 'running')
    })
    await new Promise((r) => setTimeout(r, 1500))
    const runs = await client.result<unknown[]>('workflowRun:list', { workflowId: 'wf-poll' })
    expect(runs).toHaveLength(1)

    const backfilled = await client.result<{ imported: number; updated: number }>(
      'connection:backfill',
      {
        connectionId: conn.id
      }
    )
    expect(backfilled.imported + backfilled.updated).toBe(1)

    // A pack removed takes its connections' children with it, and says how many connections it leaves.
    expect(await client.result('connector:removePack', 'tickets')).toEqual({
      ok: true,
      connections: 1
    })
  })

  it('answers the connector-wide calls itself', async () => {
    const list = await client.result<Array<{ id: string }>>('connector:list')
    expect(list.map((c) => c.id)).toEqual(['http', 'mcp', 'sdk'])
    expect(await client.result('connector:get', 'nope')).toBeNull()
    const catalog = await client.result<{ items: unknown[]; templates: unknown[] }>(
      'connector:catalog'
    )
    expect(catalog.items.length).toBeGreaterThan(0)
    expect(catalog.templates.length).toBeGreaterThan(0)
    expect(await client.result('connector:probeAuth', 'http')).toEqual({ ok: null })
    expect(await client.result('connector:probeSdk', { command: '   ' })).toEqual({
      ok: false,
      error: 'A command is required'
    })
    const refused = await client.result('connector:inspectPack', {
      kind: 'url',
      url: 'http://example.com/p.tgz'
    })
    expect(refused).toEqual({
      ok: false,
      error: 'A pack is fetched over https, or from this machine'
    })
  })

  it('refuses a window call it never granted', async () => {
    const res = await fetch(`http://127.0.0.1:${server.vornd}/connections/nope/browser/fetch`, {
      method: 'POST',
      headers: { authorization: 'Bearer guess', 'content-type': 'application/json' },
      body: JSON.stringify({ url: 'https://example.com', method: 'GET' })
    })
    expect(res.status).toBe(401)
  })
})
