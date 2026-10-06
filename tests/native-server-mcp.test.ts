/**
 * vornd's MCP server at `/mcp`, against the TypeScript MCP server.
 *
 * One server is started, with its store in memory, and vornds in front of
 * it: one serving MCP, one that has no local credential to check against,
 * and one leaving `/mcp` to the server. The TypeScript server runs in this
 * process and reaches the same server over its socket, as it does when an
 * agent starts it; vornd's tools reach it through vornd's own `/ws`.
 *
 * Every message goes to both and the two responses must be the same, and so
 * must the configuration a call leaves behind, but for the differences
 * `helpers/mcp-parity` names. Each call starts from the same configuration
 * on each side. Arguments are scripted for the calls that change something,
 * and seeded at random from each tool's input schema for every tool; a tool
 * whose handler would start something real (a process, a download, an
 * agent) is only sent arguments its schema refuses.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import { PassThrough } from 'node:stream'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'
import type { JSONRPCMessage } from '@modelcontextprotocol/sdk/types.js'
import type { AppConfig } from '../packages/shared/src/types'
import { comparable, randomArgs, refusedArgs, seeded, uuidsIn } from './helpers/mcp-parity'

const TEST_CREDENTIAL = 'native-server-mcp-test-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''
const SEED = Number(process.env.VORN_MCP_PARITY_SEED ?? 0x6d63)
/** Random argument sets per tool that may run. */
const RANDOM_CASES = 6

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

const store = vi.hoisted(() => ({ dir: '', config: null as unknown }))

vi.mock('node-pty', () => ({
  default: { spawn: vi.fn() },
  spawn: vi.fn()
}))

vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

// The configuration lives in `store.config`, which each case resets.
vi.mock(
  '../packages/server/src/database',
  () =>
    ({
      closeDatabase: vi.fn(),
      initDatabase: vi.fn(),
      getDataDir: vi.fn(() => store.dir),
      dbGetOwnerUser: vi.fn(() => ({
        id: 'owner-1',
        name: 'test',
        role: 'owner' as const,
        createdAt: new Date().toISOString()
      })),
      dbInsertDeviceToken: vi.fn(),
      dbListDeviceTokens: vi.fn(() => []),
      dbGetDeviceTokenSecret: vi.fn(),
      dbRevokeDeviceToken: vi.fn(() => true),
      dbTouchDeviceToken: vi.fn(),
      loadConfig: vi.fn(() => structuredClone(store.config)),
      saveConfig: vi.fn((config: unknown) => {
        store.config = structuredClone(config)
      }),
      dbListTasks: vi.fn(() => []),
      dbGetTask: vi.fn(),
      dbInsertTask: vi.fn(),
      dbUpdateTask: vi.fn(),
      dbDeleteTask: vi.fn(),
      dbGetMaxTaskOrder: vi.fn(() => 0),
      dbGetProject: vi.fn(),
      dbListProjects: vi.fn(() => []),
      dbListWorkflows: vi.fn(() => []),
      dbInsertWorkflow: vi.fn(),
      dbUpdateWorkflow: vi.fn(),
      dbDeleteWorkflow: vi.fn(),
      saveWorkflowRun: vi.fn(),
      listWorkflowRuns: vi.fn(() => []),
      listWorkflowRunIds: vi.fn(() => []),
      deleteArtifactsUpdatedBefore: vi.fn(() => []),
      listArtifactIds: vi.fn(() => []),
      listWorkflowRunsByTask: vi.fn(() => []),
      updateWorkflowRunStatus: vi.fn(),
      dbReleaseConnectorInboxLeases: vi.fn(),
      dbCountActiveConnectorInboxLeases: vi.fn(() => 0),
      dbClaimConnectorInbox: vi.fn(() => []),
      dbGetWorkflowRunByConnectorInboxId: vi.fn(() => null)
    }) satisfies Partial<Record<keyof typeof import('../packages/server/src/database'), unknown>>
)

const TASK_TODO = '0b6f3f0e-4c1a-4e8e-9a51-0d2f1c1e7a01'
const TASK_DONE = '0b6f3f0e-4c1a-4e8e-9a51-0d2f1c1e7a02'
const TASK_ARCHIVED = '0b6f3f0e-4c1a-4e8e-9a51-0d2f1c1e7a03'
const WORKFLOW = '7d2c5b9a-1e3f-4a6b-8c0d-2e4f6a8b0c01'

/** Two projects in two workspaces, three tasks and one manual workflow. */
function fixture(cwd: string): AppConfig {
  const at = '2026-01-02T03:04:05.000Z'
  return {
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
    projects: [
      { name: 'app', path: cwd, preferredAgents: ['claude'], workspaceId: 'personal' },
      { name: 'other', path: '/srv/other', preferredAgents: ['codex'], workspaceId: 'work' }
    ],
    workspaces: [
      { id: 'personal', name: 'Personal', order: 0 },
      { id: 'work', name: 'Work', order: 1 }
    ],
    tasks: [
      {
        id: TASK_TODO,
        projectName: 'app',
        title: 'Write the docs',
        description: 'All of them',
        status: 'todo',
        order: 1,
        createdAt: at,
        updatedAt: at
      },
      {
        id: TASK_DONE,
        projectName: 'app',
        title: 'Ship it',
        description: '',
        status: 'done',
        order: 2,
        createdAt: at,
        updatedAt: at,
        completedAt: at
      },
      {
        id: TASK_ARCHIVED,
        projectName: 'other',
        title: 'Old idea',
        description: '',
        status: 'todo',
        order: 1,
        createdAt: at,
        updatedAt: at,
        archivedAt: at
      }
    ],
    workflows: [
      {
        id: WORKFLOW,
        name: 'Nightly',
        icon: 'zap',
        iconColor: '#6366f1',
        enabled: true,
        nodes: [
          {
            id: 'n-trigger',
            type: 'trigger',
            label: 'Manual Trigger',
            config: { triggerType: 'manual' },
            position: { x: 0, y: 0 }
          },
          {
            id: 'n-agent',
            type: 'launchAgent',
            label: 'Launch claude',
            config: { agentType: 'claude', projectName: 'app', projectPath: cwd, prompt: 'Go' },
            position: { x: 0, y: 140 }
          }
        ],
        edges: [{ id: 'e-1', source: 'n-trigger', target: 'n-agent' }]
      }
    ],
    remoteHosts: []
  } as unknown as AppConfig
}

/**
 * Tools that may run with any arguments: they read or write the
 * configuration, ask the server something, or answer that there is no
 * session. Every other tool would start something, and is only refused.
 */
const RUNS = new Set([
  'get_config',
  'list_sessions',
  'list_session_events',
  'list_connectors',
  'list_connections',
  'read_page',
  'get_page_text',
  'read_console_messages',
  'read_network_requests',
  'browser_screenshot',
  'browser_find',
  'browser_interact',
  'browser_tabs',
  'browser_navigate',
  'browser_history',
  'list_artifacts',
  'read_artifact_comments',
  'read_artifact',
  'list_projects',
  'create_project',
  'update_project',
  'delete_project',
  'list_tasks',
  'create_task',
  'get_task',
  'update_task',
  'delete_task',
  'archive_task',
  'unarchive_task',
  'get_my_context',
  'list_workflows',
  'create_workflow',
  'update_workflow',
  'delete_workflow',
  'list_workflow_runs',
  'stop_workflow_run',
  'resolve_gate',
  'get_workflow_schedule',
  'describe_workflow_nodes',
  'list_workspaces',
  'create_workspace',
  'update_workspace',
  'delete_workspace'
])

/** Calls that change the configuration, with arguments that mean something. */
function scripted(cwd: string): Array<[string, Record<string, unknown>]> {
  return [
    ['create_task', { project_name: 'app', title: 'New one', status: 'in_progress' }],
    ['create_task', { title: 'From the directory', description: 'cwd' }],
    ['create_task', { project_name: 'nope', title: 'x' }],
    ['update_task', { id: TASK_TODO, status: 'done', title: 'Docs written' }],
    ['update_task', { id: TASK_DONE, status: 'todo' }],
    ['delete_task', { id: TASK_DONE }],
    ['archive_task', { id: TASK_TODO }],
    ['unarchive_task', { id: TASK_ARCHIVED }],
    ['list_tasks', { project_name: 'app', status: 'todo' }],
    ['list_tasks', { include_archived: true }],
    ['get_my_context', {}],
    ['create_project', { name: 'third', path: path.join(cwd, 'packages') }],
    ['create_project', { name: 'app', path: cwd }],
    ['update_project', { name: 'other', preferred_agents: ['claude', 'codex'] }],
    ['delete_project', { name: 'other' }],
    ['create_workspace', { name: 'Side' }],
    ['update_workspace', { id: 'work', name: 'Job', order: 5 }],
    ['delete_workspace', { id: 'work' }],
    ['delete_workspace', { id: 'personal' }],
    [
      'create_workflow',
      {
        name: 'Flat',
        trigger: { triggerType: 'manual' },
        actions: [{ agentType: 'claude', projectName: 'app', projectPath: cwd, prompt: 'Hi' }]
      }
    ],
    [
      'create_workflow',
      {
        name: 'Graph',
        nodes: [
          {
            id: 't',
            type: 'trigger',
            label: 'T',
            config: { triggerType: 'manual' },
            position: { x: 0, y: 0 }
          },
          {
            id: 's',
            type: 'script',
            label: 'S',
            config: { scriptType: 'bash', scriptContent: 'echo hi' },
            position: { x: 0, y: 100 }
          }
        ],
        edges: [{ id: 'e', source: 't', target: 's' }]
      }
    ],
    ['update_workflow', { workflow_id: WORKFLOW, name: 'Nightly 2', enabled: false }],
    ['delete_workflow', { id: WORKFLOW }],
    ['export_workflow', { workflow_id: WORKFLOW }],
    ['import_workflow', { workflow: '{not json', project_name: 'app' }],
    ['describe_workflow_nodes', { types: ['loop', 'script'] }],
    ['get_workflow_schedule', {}]
  ]
}

/** Sends JSON-RPC messages and returns each request's response. */
interface Side {
  send(message: JSONRPCMessage): Promise<unknown>
}

/** The TypeScript server, in this process, over an in-memory transport. */
async function typescriptSide(version: string): Promise<Side> {
  const { createMcpServer } = await import('../packages/mcp/src/server')
  const server = createMcpServer(version)
  const [ours, theirs] = InMemoryTransport.createLinkedPair()
  await server.connect(ours)
  const waiting = new Map<unknown, (m: unknown) => void>()
  theirs.onmessage = (message) => {
    const id = (message as { id?: unknown }).id
    waiting.get(id)?.(JSON.parse(JSON.stringify(message)))
    waiting.delete(id)
  }
  await theirs.start()
  return {
    send(message) {
      const id = (message as { id?: unknown }).id
      if (id === undefined) return theirs.send(message).then(() => null)
      return new Promise((resolve) => {
        waiting.set(id, resolve)
        void theirs.send(message)
      })
    }
  }
}

interface Posted {
  status: number
  headers: Headers
  body: unknown
}

async function post(port: number, headers: Record<string, string>, body: unknown): Promise<Posted> {
  const res = await fetch(`http://127.0.0.1:${port}/mcp`, {
    method: 'POST',
    headers: {
      Accept: 'application/json, text/event-stream',
      'Content-Type': 'application/json',
      ...headers
    },
    body: JSON.stringify(body)
  })
  const text = await res.text()
  let answer: unknown
  try {
    answer = JSON.parse(text)
  } catch {
    answer = text
  }
  return { status: res.status, headers: res.headers, body: answer }
}

/** vornd's `/mcp`, after an `initialize` that opened a session. */
async function vorndSide(port: number, credentials: Record<string, string>): Promise<Side> {
  let session = ''
  return {
    async send(message) {
      const headers = session ? { ...credentials, 'Mcp-Session-Id': session } : credentials
      const answer = await post(port, headers, message)
      session = answer.headers.get('mcp-session-id') ?? session
      return answer.status === 202 ? null : answer.body
    }
  }
}

interface Vornd {
  port: number
  child: ChildProcess
}

async function startVornd(
  upstream: number,
  args: string[],
  env: Record<string, string> = {}
): Promise<Vornd> {
  const child = spawn(vornd!, ['--upstream', `127.0.0.1:${upstream}`, ...args], {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: { ...process.env, VORND_LOG: process.env.VORND_LOG ?? 'warn', ...env }
  })
  const port = await new Promise<number>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vornd did not start')), 10_000)
    createInterface({ input: child.stdout! }).once('line', (line) => {
      clearTimeout(timer)
      resolve((JSON.parse(line) as { port: number }).port)
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`vornd exited with ${code} before listening`))
    })
  })
  return { port, child }
}

function stopVornd(v: Vornd | undefined): Promise<void> {
  if (!v || v.child.exitCode !== null || v.child.signalCode !== null) return Promise.resolve()
  return new Promise((resolve) => {
    v.child.once('exit', () => resolve())
    v.child.kill()
  })
}

type Groups = Record<string, { mode?: string; native?: number; forwarded?: number }>

async function groups(v: Vornd): Promise<Groups> {
  const res = await fetch(`http://127.0.0.1:${v.port}/vornd/health`)
  return ((await res.json()) as { groups: Groups }).groups
}

let closeServer: () => Promise<void>
let native: Vornd | undefined
let untold: Vornd | undefined
let forward: Vornd | undefined
let ts: Side
let rust: Side
let version: string
let start: AppConfig
let fixtureIds: Set<string>
let tools: Array<{ name: string; inputSchema: Parameters<typeof randomArgs>[0] }>
let nextId = 100

const credentials = (): Record<string, string> => ({
  Authorization: `Bearer ${TEST_CREDENTIAL}`,
  'Vorn-Cwd': encodeURIComponent(process.cwd())
})

function reset(): void {
  store.config = structuredClone(start)
}

/** Clears the server's cached configuration, so the next read is the store's. */
async function forgetCache(): Promise<void> {
  const { configManager } = await import('../packages/server/src/config-manager')
  ;(configManager as unknown as { cachedConfig: unknown }).cachedConfig = null
}

/** One tool call on each side, each from the starting configuration. */
async function both(name: string, args: Record<string, unknown> | undefined) {
  const id = nextId++
  const call = {
    jsonrpc: '2.0',
    id,
    method: 'tools/call',
    params: args === undefined ? { name } : { name, arguments: args }
  } as JSONRPCMessage
  reset()
  await forgetCache()
  const theirs = await ts.send(call)
  const theirConfig = store.config
  reset()
  await forgetCache()
  const ours = await rust.send(call)
  const ourConfig = store.config
  return {
    theirs: comparable(theirs, theirConfig, fixtureIds),
    ours: comparable(ours, ourConfig, fixtureIds),
    raw: theirs as { result?: { content?: Array<{ text?: string }>; isError?: boolean } }
  }
}

describe.skipIf(!vornd)("vornd's MCP server answers as the TypeScript one does", () => {
  beforeAll(async () => {
    delete process.env.VORN_SESSION_ID
    store.dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-mcp-')))
    process.env.VORN_DATA_DIR = store.dir
    start = fixture(process.cwd())
    fixtureIds = uuidsIn(start)
    reset()
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    const { startServer } = await import('../packages/server/src/index')
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    let serverPort: number
    try {
      const { app, port } = await startServer({ port: 0 })
      serverPort = port
      closeServer = () => app.close()
    } finally {
      process.stdout.write = origWrite
    }
    const token = { VORND_DESKTOP_TOKEN: TEST_CREDENTIAL }
    native = await startVornd(serverPort, ['--groups', 'mcp=native'], token)
    untold = await startVornd(serverPort, ['--groups', 'mcp=native'])
    forward = await startVornd(serverPort, ['--groups', 'mcp=forward'], token)

    version = (
      JSON.parse(
        fs.readFileSync(path.resolve(__dirname, '../packages/mcp/package.json'), 'utf-8')
      ) as { version: string }
    ).version
    ts = await typescriptSide(version)
    rust = await vorndSide(native.port, credentials())
    const initialize = {
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-06-18',
        capabilities: {},
        clientInfo: { name: 'parity', version: '0' }
      }
    } as JSONRPCMessage
    const [theirs, ours] = [await ts.send(initialize), await rust.send(initialize)]
    expect(ours).toEqual(theirs)
    const initialized = { jsonrpc: '2.0', method: 'notifications/initialized' } as JSONRPCMessage
    await ts.send(initialized)
    await rust.send(initialized)
  }, 60_000)

  afterAll(async () => {
    await Promise.all([stopVornd(native), stopVornd(untold), stopVornd(forward)])
    await closeServer?.()
    if (store.dir) fs.rmSync(store.dir, { recursive: true, force: true })
  })

  it('lists the same tools', async () => {
    const list = { jsonrpc: '2.0', id: 2, method: 'tools/list' } as JSONRPCMessage
    const theirs = await ts.send(list)
    const ours = await rust.send(list)
    expect(JSON.stringify(ours)).toBe(JSON.stringify(theirs))
    tools = (theirs as { result: { tools: typeof tools } }).result.tools
    expect(tools.length).toBe(73)
  })

  it('answers protocol messages the same way', async () => {
    for (const message of [
      { jsonrpc: '2.0', id: 3, method: 'ping' },
      { jsonrpc: '2.0', id: 4, method: 'resources/list' },
      { jsonrpc: '2.0', id: 5, method: 'tools/call', params: {} },
      { jsonrpc: '2.0', id: 6, method: 'tools/call', params: { name: 'no_such_tool' } }
    ] as JSONRPCMessage[]) {
      expect(JSON.stringify(await rust.send(message))).toBe(JSON.stringify(await ts.send(message)))
    }
  })

  it('makes the same changes for the scripted calls', async () => {
    for (const [name, args] of scripted(process.cwd())) {
      const { theirs, ours } = await both(name, args)
      expect(ours, `${name} ${JSON.stringify(args)}`).toBe(theirs)
    }
  }, 120_000)

  it('answers seeded random arguments to every tool the same way', async () => {
    const strings = [
      'app',
      'other',
      'nope',
      'personal',
      'work',
      'Nightly',
      TASK_TODO,
      TASK_DONE,
      TASK_ARCHIVED,
      WORKFLOW,
      'n-agent',
      '',
      ' ',
      '../escape',
      process.cwd(),
      '/srv/other',
      'relative/path',
      'todo',
      'done',
      'in_progress',
      'café ☕',
      '😀'.repeat(3),
      'x'.repeat(300)
    ]
    const pool = { strings, numbers: [-1, 0, 1, 2, 1.5, 7, 100, 1e9] }
    let ran = 0
    let refused = 0
    let answered = 0
    for (const [index, tool] of tools.entries()) {
      const random = seeded(SEED + index * 7919)
      const cases: Array<Record<string, unknown> | undefined> = []
      if (RUNS.has(tool.name)) {
        cases.push(undefined)
        for (let i = 0; i < RANDOM_CASES; i++)
          cases.push(randomArgs(tool.inputSchema, random, pool))
      } else {
        const args = refusedArgs(tool.inputSchema)
        if (args) cases.push(args)
      }
      for (const args of cases) {
        const { theirs, ours, raw } = await both(tool.name, args)
        const label = `${tool.name} seed=${SEED} ${JSON.stringify(args)}`
        if (!RUNS.has(tool.name)) {
          // Nothing real may start: the TypeScript server must have refused it too.
          expect(raw.result?.content?.[0]?.text, `${label}`).toContain('Input validation error')
          refused++
        } else {
          ran++
          if (!raw.result?.content?.[0]?.text?.includes('Input validation error')) answered++
        }
        expect(ours, `${label}`).toBe(theirs)
      }
    }
    expect(ran).toBeGreaterThan(RUNS.size * RANDOM_CASES)
    expect(refused).toBeGreaterThan(20)
    // Enough arguments got past the schemas for the handlers to be compared too.
    expect(answered).toBeGreaterThan(ran / 3)
  }, 300_000)

  it('counts its requests under the mcp group', async () => {
    const counted = await groups(native!)
    expect(counted.mcp?.mode).toBe('native')
    expect(counted.mcp?.native ?? 0).toBeGreaterThan(0)
  })

  it('lets in only an agent holding the local credential', async () => {
    const ping = { jsonrpc: '2.0', id: 1, method: 'ping' }
    const withOrigin = await post(
      native!.port,
      { ...credentials(), Origin: 'http://evil.test' },
      ping
    )
    expect(withOrigin.status).toBe(403)
    expect((await post(native!.port, {}, ping)).status).toBe(401)
    const wrong = await post(native!.port, { Authorization: 'Bearer not-it' }, ping)
    expect(wrong.status).toBe(401)
    expect((await post(untold!.port, credentials(), ping)).status).toBe(503)
    // Forwarded, `/mcp` is the server's, which has no such route.
    expect((await post(forward!.port, credentials(), ping)).status).toBe(404)
  })

  it('relays an agent over stdio to vornd when vornd serves MCP', async () => {
    const { relay, relayHeaders, vorndMcpUrl } = await import('../packages/mcp/src/relay')
    const status = (port: number) => async () =>
      ({ state: 'on', port, nativeServer: true }) as const
    expect(await vorndMcpUrl({ vorndStatus: status(forward!.port), fetch })).toBeNull()
    const url = await vorndMcpUrl({ vorndStatus: status(native!.port), fetch })
    expect(url?.href).toBe(`http://127.0.0.1:${native!.port}/mcp`)

    const stdin = new PassThrough()
    const stdout = new PassThrough()
    const transport = new StdioServerTransport(stdin, stdout)
    stdin.once('end', () => void transport.close())
    const relayed = relay(transport, url!, relayHeaders(TEST_CREDENTIAL, process.cwd(), {}))
    const lines = createInterface({ input: stdout })[Symbol.asyncIterator]()
    const ask = async (message: object): Promise<unknown> => {
      stdin.write(JSON.stringify(message) + '\n')
      return JSON.parse((await lines.next()).value as string)
    }
    const init = (await ask({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-06-18',
        capabilities: {},
        clientInfo: { name: 'r', version: '0' }
      }
    })) as { result: { serverInfo: { name: string; version: string } } }
    expect(init.result.serverInfo).toEqual({ name: 'vorn', version })
    stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n')

    reset()
    await forgetCache()
    const call = { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'list_projects' } }
    const viaRelay = await ask(call)
    reset()
    await forgetCache()
    // The relay hands on each message as the SDK's client transport parsed
    // it, which orders a response's keys as its schema does; the values are
    // the server's.
    expect(viaRelay).toEqual(await ts.send(call as JSONRPCMessage))

    stdin.end()
    await relayed
  })
})
