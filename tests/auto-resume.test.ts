// @vitest-environment jsdom
/**
 * Automatic resume, end to end, with "Reopen sessions" on.
 *
 * Agents are started on a real server, then the server goes away twice. Once
 * while the session holder keeps running, as when Vorn restarts: the agents
 * must still be live, not started again. Once with the holder and every
 * program gone too, as after a reboot: the next server and vornd start fresh
 * from what vornd wrote down, and the renderer's own start-up pass
 * (`syncBoard`, with `resume` on as App does) must start each agent again on
 * its conversation, once, and its pane must attach through vornd with a
 * screen and a live stream. Nothing is clicked.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { TerminalSession } from '../packages/shared/src/types'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  TEST_CREDENTIAL,
  Watcher,
  removeRealServerDirs,
  repository,
  runnable,
  startRealServer,
  stopRealServer,
  until,
  type RealServer
} from './helpers/real-server'

vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

spawnsRealServers()

/** An agent that says what it was started with, then echoes what it is sent. */
const ARGV_AGENT = `#!/bin/sh
printf 'ARGV:%s\\n' "$*"
exec cat
`

/** The renderer's bridge to the server, as main forwards it: every call through vornd. */
let bridge: Watcher | null = null
const api = {
  listActiveSessions: () => bridge!.result('terminal:listActive'),
  getRestoredSessions: () => bridge!.result('sessions:restored'),
  resumeSession: (params: { id: string }) => bridge!.result('sessions:resume', params),
  sessionRestored: (params: unknown) => bridge!.result('workflow:sessionRestored', params),
  notifyWidgetStatus: () => undefined
}
Object.defineProperty(window, 'api', { value: api, writable: true })

const { useAppStore } = await import('../src/renderer/stores')
const { syncBoard } = await import('../src/renderer/lib/board-sync')

/** A pane's socket: the attach answer, then the bytes the session prints. */
class Pane {
  private bytes = ''
  private next = 1
  private constructor(private ws: WebSocket) {
    ws.on('message', (raw, isBinary) => {
      if (isBinary) this.bytes += Buffer.from(raw as Buffer).toString('latin1')
    })
  }

  static open(port: number): Promise<Pane> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    return new Promise((resolve, reject) => {
      ws.once('open', () => resolve(new Pane(ws)))
      ws.once('error', reject)
    })
  }

  call(method: string, params: unknown): Promise<Record<string, unknown>> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData, isBinary: boolean): void => {
        if (isBinary) return
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

  write(id: string, data: string): void {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'terminal:write', params: { id, data } }))
  }

  streamed(text: string): boolean {
    return this.bytes.includes(text)
  }

  close(): void {
    this.ws.close()
  }
}

interface Attached {
  replies?: string
  live?: boolean
  data?: string
}

/** What one pane saw: its attach, and whether input came back on the stream. */
interface PaneSeen {
  replies?: string
  live?: boolean
  screen: string
  streams: boolean
}

async function attachPane(port: number, id: string, probe: string): Promise<PaneSeen> {
  const pane = await Pane.open(port)
  try {
    const frame = await pane.call('terminal:attach', { id })
    const answer = (frame.result ?? {}) as Attached
    pane.write(id, `${probe}\r`)
    let streams = true
    await until(`${probe} streamed back`, () => pane.streamed(probe)).catch(() => {
      streams = false
    })
    return { replies: answer.replies, live: answer.live, screen: answer.data ?? '', streams }
  } finally {
    pane.close()
  }
}

/** Every process under `root`, deepest first. */
function descendants(root: number): number[] {
  const table = execFileSync('ps', ['-A', '-o', 'pid=,ppid='], { encoding: 'utf8' })
  const children = new Map<number, number[]>()
  for (const line of table.split('\n')) {
    const [pid, ppid] = line.trim().split(/\s+/).map(Number)
    if (!pid) continue
    children.set(ppid, [...(children.get(ppid) ?? []), pid])
  }
  const out: number[] = []
  const walk = (pid: number): void => {
    for (const child of children.get(pid) ?? []) {
      walk(child)
      out.push(child)
    }
  }
  walk(root)
  return out
}

function kill(pid: number): void {
  try {
    process.kill(pid, 'SIGKILL')
  } catch {
    /* already gone */
  }
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

async function holderPid(server: RealServer): Promise<number> {
  const res = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
  const health = (await res.json()) as { sessiond?: { current?: { pid?: number } } }
  return health.sessiond?.current?.pid ?? 0
}

/** A machine going down: the server, vornd, the holder and every program in it, at once. */
async function reboot(server: RealServer): Promise<void> {
  const holder = await holderPid(server)
  const serverPid = server.child.pid!
  const gone = [...descendants(serverPid), serverPid, ...descendants(holder), holder]
  const exited = new Promise((r) => server.child.once('exit', r))
  for (const pid of gone) kill(pid)
  await exited
  await until('every process to be gone', () => gone.every((pid) => !alive(pid)))
}

/** Every line an agent printed with what it was started with. */
async function argvLines(client: Watcher, id: string): Promise<string[]> {
  const out = await client.result<string[]>('terminal:readOutput', { id })
  return out.filter((l) => l.includes('ARGV:')).map((l) => l.slice(l.indexOf('ARGV:')).trimEnd())
}

interface Started {
  dirs: RealServer['dirs']
  agents: TerminalSession[]
}

async function startAgents(server: RealServer): Promise<TerminalSession[]> {
  const { work } = server.dirs
  const stub = path.join(work, 'bin', 'argv-agent')
  fs.mkdirSync(path.dirname(stub), { recursive: true })
  fs.writeFileSync(stub, ARGV_AGENT, { mode: 0o755 })
  const repo = path.join(work, 'repo')
  repository(repo)
  const client = await Watcher.open(server.vornd)
  try {
    const config = await client.result<Record<string, unknown>>('config:load')
    await client.result('config:save', {
      ...config,
      defaults: { ...(config.defaults as object), shell: '/bin/sh', reopenSessions: true },
      agentCommands: { claude: { command: stub, args: [] } }
    })
    const ids: string[] = []
    for (const name of ['First', 'Second']) {
      const s = await client.result<TerminalSession>('terminal:create', {
        agentType: 'claude',
        projectName: 'repo',
        projectPath: repo,
        displayName: name
      })
      ids.push(s.id)
    }
    await until('both agents to print how they started', async () => {
      for (const id of ids) if ((await argvLines(client, id)).length === 0) return false
      return true
    })
    const listed = await client.result<TerminalSession[]>('terminal:listActive')
    const agents = ids.map((id) => listed.find((s) => s.id === id)!)
    for (const a of agents) expect(a.agentSessionId).toBeTruthy()
    // Written down by vornd as it goes; a machine going down gives it no last word.
    const carried = path.join(server.dirs.data, 'vornd', 'sessions.json')
    await until('vornd to write the agents down', () => {
      try {
        const text = fs.readFileSync(carried, 'utf8')
        return agents.every((a) => text.includes(a.agentSessionId!))
      } catch {
        return false
      }
    })
    return agents
  } finally {
    client.close()
  }
}

/** The app starting on `server`: the bridge through vornd, then App's start-up pass. */
async function appStarts(server: RealServer): Promise<void> {
  useAppStore.setState({ terminals: new Map() } as never)
  bridge = await Watcher.open(server.vornd)
  await syncBoard({ showCold: true, resume: true })
}

interface Run {
  agents: TerminalSession[]
  warm: { argv: string[][]; ended: boolean[]; panes: PaneSeen[] }
  cold: {
    argv: string[][]
    ended: boolean[]
    panes: PaneSeen[]
    offeredAfter: number
    boardIds: string[]
  }
}

async function scenario(): Promise<Run> {
  const first = await startRealServer()
  let started: Started
  try {
    started = { dirs: first.dirs, agents: await startAgents(first) }
  } finally {
    await stopRealServer(first, true)
  }
  const ids = started.agents.map((a) => a.id)

  // (a) Vorn restarts; the holder never stopped.
  const second = await startRealServer(started.dirs, { early: true })
  let warm: Run['warm']
  try {
    await appStarts(second)
    const terms = useAppStore.getState().terminals
    const argv: string[][] = []
    const panes: PaneSeen[] = []
    for (const id of ids) {
      argv.push(await argvLines(bridge!, id))
      panes.push(await attachPane(second.vornd, id, `warm-${id.slice(0, 6)}`))
    }
    warm = { argv, ended: ids.map((id) => !!terms.get(id)?.ended), panes }
    bridge!.close()
    // (b) The machine goes down with everything on it.
    await reboot(second)
  } catch (err) {
    throw new Error(`${(err as Error).message}\n${second.log.join('').slice(-4000)}`, {
      cause: err
    })
  }

  const third = await startRealServer(started.dirs, { early: true })
  try {
    await appStarts(third)
    const terms = useAppStore.getState().terminals
    const argv: string[][] = []
    const panes: PaneSeen[] = []
    for (const id of ids) {
      await until(`${id} to print how it started again`, async () => {
        return (await argvLines(bridge!, id)).length > 0
      }).catch(() => undefined)
      argv.push(await argvLines(bridge!, id))
      panes.push(await attachPane(third.vornd, id, `cold-${id.slice(0, 6)}`))
    }
    const offered = await bridge!.result<unknown[]>('sessions:restored')
    const cold = {
      argv,
      ended: ids.map((id) => !!terms.get(id)?.ended),
      panes,
      offeredAfter: offered.length,
      boardIds: [...terms.keys()]
    }
    bridge!.close()
    return { agents: started.agents, warm, cold }
  } catch (err) {
    throw new Error(`${(err as Error).message}\n${third.log.join('').slice(-4000)}`, {
      cause: err
    })
  } finally {
    await stopRealServer(third)
  }
}

describe.skipIf(!runnable)('automatic resume with Reopen sessions on', () => {
  let run: Run

  beforeAll(async () => {
    run = await scenario()
  }, 300_000)

  afterAll(() => removeRealServerDirs())

  it('takes the agents on live after Vorn restarts, without starting them again', () => {
    expect(run.warm.ended).toEqual([false, false])
    run.agents.forEach((agent, i) => {
      expect(run.warm.argv[i]).toEqual([`ARGV:--session-id ${agent.agentSessionId}`])
      expect(run.warm.panes[i]).toMatchObject({ replies: 'vornd', live: true, streams: true })
      expect(run.warm.panes[i].screen).toContain('ARGV:')
    })
  })

  it('starts each agent again on its conversation, once, after a reboot', () => {
    expect(run.cold.boardIds.sort()).toEqual(run.agents.map((a) => a.id).sort())
    expect(run.cold.ended).toEqual([false, false])
    run.agents.forEach((agent, i) => {
      expect(run.cold.argv[i]).toEqual([`ARGV:--resume ${agent.agentSessionId}`])
    })
    expect(run.cold.offeredAfter).toBe(0)
  })

  it('attaches each resumed pane through vornd with its screen and a live stream', () => {
    for (const pane of run.cold.panes) {
      expect(pane).toMatchObject({ replies: 'vornd', live: true, streams: true })
      expect(pane.screen).toContain('ARGV:--resume')
    }
  })
})
