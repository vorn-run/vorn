/**
 * The terminals' statuses, which vornd's copy of the session registry
 * decides.
 *
 * vornd is started as the server on a home and a data directory of its own, and
 * runs the same stub agent under each of the five agent types: a
 * script that prints, on cue, a line that reads as running, a prompt that reads
 * as waiting and an error, then goes quiet until it is idle. Its cues come both
 * as notifications and as calls. Then the
 * agent's hooks are posted to vornd's hook endpoint as the agent would:
 * the session linked, waiting, a permission asked, stopped, running again, and
 * a screen error its hooks overrule. Every `session:updated` the server
 * broadcasts is collected, and each agent must go through the scripted
 * statuses in order.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`), on a
 * Unix: the agent is a shell script.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { AgentStatus, TerminalSession } from '@vornrun/shared/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { builtSessiond, builtVornd, startServed, type Served } from './helpers/served'

const CREDENTIAL = 'native-status-test-credential'
const runnable = !!builtVornd && !!builtSessiond && process.platform !== 'win32'

const AGENTS = ['claude', 'codex', 'copilot', 'gemini', 'opencode'] as const
type Agent = (typeof AGENTS)[number]

/**
 * What each agent's status goes through once it is up, in order: the scripted
 * output, then its hooks. Repeats collapse: a broadcast that changed something
 * else is not a new status.
 */
const EXPECTED: AgentStatus[] = [
  // Its prompt; a write through the server; its error; its work.
  'waiting',
  'running',
  'error',
  'running',
  // Quiet for five seconds, then printing again.
  'idle',
  'running',
  // Hooks: Notification, Stop, PermissionRequest, PostToolUse.
  'waiting',
  'idle',
  'waiting',
  'running',
  // Its screen's error, overruled; then Stop.
  'idle'
]

/**
 * The stub agent. It reads each cue as a line on its terminal, with echo off so
 * the cue prints nothing of its own.
 */
const AGENT_SCRIPT = `#!/bin/sh
stty -echo 2>/dev/null
printf 'agent ready\\n'
read cue
printf 'Proceed? (y/n) '
read cue
printf '\\nError: it broke\\n'
read cue
printf 'one\\ntwo\\nthree\\nfour\\nfive\\nworking\\n'
read cue
printf 'more work\\n'
read cue
printf 'Error: late\\n'
exec sleep 600
`

spawnsRealServers()

const PATIENCE_MS = 30_000

async function waitFor<T>(what: string, check: () => Promise<T | null> | T | null): Promise<T> {
  const until = Date.now() + PATIENCE_MS
  for (;;) {
    const found = await check()
    if (found !== null) return found
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}

/** A WebSocket client that keeps every `session:updated` it is told. */
class Client {
  private next = 1
  readonly updated: Array<{ id: string; status: AgentStatus }> = []

  private constructor(private ws: WebSocket) {
    ws.on('message', (raw) => {
      const frame = JSON.parse(String(raw)) as { method?: string; params?: TerminalSession }
      if (frame.method === 'session:updated' && frame.params?.id)
        this.updated.push({ id: frame.params.id, status: frame.params.status })
    })
  }

  static open(port: number): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${CREDENTIAL}` }
    })
    return new Promise((resolve, reject) => {
      ws.once('open', () => resolve(new Client(ws)))
      ws.once('error', reject)
    })
  }

  call(method: string, params?: unknown): Promise<Record<string, unknown>> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 20_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(String(raw)) as Record<string, unknown>
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  async result<T>(method: string, params?: unknown): Promise<T> {
    const frame = await this.call(method, params)
    if (frame.error) throw new Error(`${method}: ${JSON.stringify(frame.error)}`)
    return frame.result as T
  }

  notify(method: string, params: unknown): void {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
  }

  close(): void {
    this.ws.close()
  }
}

/**
 * The statuses `id` was broadcast with, repeats collapsed. Leading `running`s
 * go too: the shell's prompt and the launch line it echoed come before the
 * agent, and a broadcast of theirs may still be on its way when it is up.
 */
function statusesOf(
  updated: ReadonlyArray<{ id: string; status: AgentStatus }>,
  id: string
): AgentStatus[] {
  const seen: AgentStatus[] = []
  for (const u of updated) {
    if (u.id !== id || seen.at(-1) === u.status) continue
    if (seen.length === 0 && u.status === 'running') continue
    seen.push(u.status)
  }
  return seen
}

interface Server {
  served: Served
  dirs: string[]
  port: number
}

const servers: Server[] = []

async function startServer(): Promise<Server> {
  // Its own home: the hook endpoint's port and token are written there, and
  // the agents' hook settings, which a test must not touch for real.
  const home = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-status-home-')))
  const served = await startServed({ home, credential: CREDENTIAL, sessiond: true })
  const server: Server = { served, dirs: [home, served.dataDir], port: served.port }
  servers.push(server)
  await waitFor('the session holder', async () => {
    const res = await fetch(`http://127.0.0.1:${server.port}/vornd/health`)
    const health = (await res.json()) as {
      sessiond?: { current?: { pid?: number } }
      registry?: { fed?: boolean; decides?: boolean }
    }
    if (!health.sessiond?.current?.pid) return null
    return health.registry?.fed && health.registry.decides ? true : null
  })
  return server
}

/** Posts one hook event as an agent's hook script does. */
async function postHook(
  server: Server,
  event: Record<string, unknown>,
  signal?: AbortSignal
): Promise<void> {
  const vorn = path.join(server.served.home, '.vorn')
  const port = await waitFor('the hook endpoint', () =>
    fs.existsSync(path.join(vorn, 'port'))
      ? Number(fs.readFileSync(path.join(vorn, 'port'), 'utf-8'))
      : null
  )
  const token = fs.readFileSync(path.join(vorn, 'token'), 'utf-8').trim()
  const sent = fetch(`http://127.0.0.1:${port}/`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify(event),
    signal
  })
  // A permission request is answered only when it is decided.
  if (event.hook_event_name === 'PermissionRequest') {
    sent.catch(() => {})
    return
  }
  const res = await sent
  expect(res.status).toBe(200)
}

/**
 * Runs the script under every agent type on `server`, and answers each agent's
 * statuses from the moment it was up.
 */
async function run(server: Server): Promise<Record<Agent, AgentStatus[]>> {
  const work = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-status-work-')))
  server.dirs.push(work)
  const stub = path.join(work, 'bin', 'stub-agent')
  fs.mkdirSync(path.dirname(stub))
  fs.writeFileSync(stub, AGENT_SCRIPT, { mode: 0o755 })

  const direct = await Client.open(server.port)
  const through = await Client.open(server.port)
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    const defaults = config.defaults as Record<string, unknown>
    await direct.result('config:save', {
      ...config,
      defaults: { ...defaults, shell: '/bin/sh' },
      agentCommands: Object.fromEntries(AGENTS.map((a) => [a, { command: stub, args: [] }]))
    })

    const sessions = {} as Record<Agent, TerminalSession>
    for (const agent of AGENTS) {
      const project = path.join(work, agent)
      fs.mkdirSync(project)
      sessions[agent] = await through.result<TerminalSession>('terminal:create', {
        agentType: agent,
        projectName: agent,
        projectPath: project
      })
    }
    const ids = AGENTS.map((a) => sessions[a].id)
    const record = async (id: string): Promise<TerminalSession | undefined> =>
      (await direct.result<TerminalSession[]>('terminal:listActive')).find((s) => s.id === id)
    const statusOf = async (id: string): Promise<AgentStatus | undefined> =>
      (await record(id))?.status
    const all = async (status: AgentStatus): Promise<void> => {
      await waitFor(`every agent ${status}`, async () => {
        const listed = await direct.result<TerminalSession[]>('terminal:listActive')
        return ids.every((id) => listed.find((s) => s.id === id)?.status === status) ? true : null
      })
    }
    /** A cue written as a notification. */
    const cueByNotice = (): void => {
      for (const id of ids) direct.notify('terminal:write', { id, data: '\r' })
    }
    /** A cue written as a call. */
    const cueByCall = async (): Promise<void> => {
      for (const id of ids) await through.result('terminal:write', { id, data: '\r' })
    }

    await waitFor('every agent to be ready', async () => {
      for (const id of ids) {
        const out = await through.result<string[]>('terminal:readOutput', { id })
        if (!out.some((l) => l.includes('agent ready'))) return null
      }
      return true
    })
    // What the shell printed before the agent started is not the script's.
    const from = direct.updated.length

    cueByNotice()
    await all('waiting')
    cueByNotice()
    await all('error')
    cueByNotice()
    await all('running')
    await all('idle')
    await cueByCall()
    await all('running')

    // The hooks, linking each session first. Copilot's is linked when it is
    // created, under an id made for it.
    const conversation = {} as Record<Agent, string>
    for (const agent of AGENTS) {
      const id = sessions[agent].id
      conversation[agent] =
        agent === 'copilot'
          ? await waitFor(
              'copilot to be linked',
              async () => (await record(id))?.hookSessionId ?? null
            )
          : `conversation-${agent}`
    }
    // A permission request is held until it is decided; this lets go of
    // any still held at the end.
    const asking = new AbortController()
    const hook = async (name: string, extra: Record<string, unknown> = {}): Promise<void> => {
      for (const agent of AGENTS) {
        const event = {
          hook_event_name: name,
          session_id: conversation[agent],
          cwd: path.join(work, agent),
          ...extra
        }
        await postHook(server, event, asking.signal)
      }
    }
    await hook('SessionStart')
    await waitFor('every agent on hooks', async () => {
      const listed = await direct.result<TerminalSession[]>('terminal:listActive')
      return ids.every((id) => listed.find((s) => s.id === id)?.statusSource === 'hooks')
        ? true
        : null
    })
    await hook('Notification')
    await all('waiting')
    await hook('Stop')
    await all('idle')
    await hook('PermissionRequest', { tool_name: 'Bash', tool_input: { command: 'ls' } })
    await all('waiting')
    await hook('PostToolUse')
    await all('running')
    // Its screen says error; its hooks say how it is.
    await cueByCall()
    await waitFor('the late error on screen', async () => {
      for (const id of ids) {
        const out = await through.result<string[]>('terminal:readOutput', { id })
        if (!out.some((l) => l.includes('Error: late'))) return null
      }
      return true
    })
    for (const id of ids) expect(await statusOf(id)).toBe('running')
    await hook('Stop')
    await all('idle')
    asking.abort()

    const updated = direct.updated.slice(from)
    return Object.fromEntries(
      AGENTS.map((a) => [a, statusesOf(updated, sessions[a].id)])
    ) as Record<Agent, AgentStatus[]>
  } finally {
    direct.close()
    through.close()
  }
}

function stop(server: Server): Promise<void> {
  return server.served.stop()
}

afterAll(async () => {
  for (const s of servers) {
    await stop(s)
    for (const dir of s.dirs) {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
    }
  }
}, 60_000)

describe.skipIf(!runnable)('the statuses vornd decides', () => {
  let statuses: Record<Agent, AgentStatus[]>

  beforeAll(async () => {
    const server = await startServer()
    try {
      statuses = await run(server)
    } finally {
      await stop(server)
    }
  }, 240_000)

  it('goes through the scripted statuses, in order', () => {
    for (const agent of AGENTS) expect([agent, statuses[agent]]).toEqual([agent, EXPECTED])
  })
})
