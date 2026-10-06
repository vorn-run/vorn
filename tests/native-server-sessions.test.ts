/**
 * vornd's copy of the server's session registry, against the registry itself.
 *
 * One server is started on a real database, and the vornd it keeps in front of
 * it shadows the calls that read the registry
 * (`terminal=shadow,headless=shadow,worktree=shadow`): the server answers each,
 * vornd answers it too from the copy the server feeds it, and the two answers
 * are compared. The test then does to sessions what the app does (shells and
 * agents created, hooks linking them, renames, groups, a reorder, an exit, a
 * resume, a kill, a headless agent) and reads the registry through vornd after
 * each step. Every comparison must have matched: a record the server changes
 * without telling vornd shows up here as a mismatch.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix: the agents are shell
 * scripts.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { isDeepStrictEqual } from 'node:util'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { HeadlessSession, TerminalSession } from '../packages/shared/src/types'
import type { SessionMirror as Mirror } from '../packages/server/src/vornd-sessions'

const TEST_CREDENTIAL = 'native-server-sessions-credential'
const GROUPS = 'terminal=shadow,headless=shadow,worktree=shadow'

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, '../packages/core/vornd'),
  path.resolve(__dirname, '../packages/core/target/release/vornd')
].find((p): p is string => !!p && fs.existsSync(p))
const runnable =
  !!vornd &&
  process.platform !== 'win32' &&
  fs.existsSync(path.join(path.dirname(vornd), 'vorn-sessiond'))

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

type Frame = Record<string, unknown>

/** A WebSocket client that sends one call at a time and returns its frame. */
class Client {
  private next = 1
  private constructor(private ws: WebSocket) {}

  static async open(port: number): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    await new Promise<void>((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })
    const client = new Client(ws)
    // The server's first answer is what shows vornd the socket was admitted.
    await client.call('config:load')
    return client
  }

  call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(raw.toString()) as Frame
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  async result<T = unknown>(method: string, params?: unknown): Promise<T> {
    const frame = await this.call(method, params)
    if (frame.error) throw new Error(`${method}: ${JSON.stringify(frame.error)}`)
    return frame.result as T
  }

  close(): void {
    this.ws.close()
  }
}

type Counts = Record<
  string,
  { mode?: string; forwarded?: number; shadowMatched?: number; shadowMismatched?: number }
>

const PATIENCE_MS = 30_000

async function until(what: string, check: () => boolean | Promise<boolean>): Promise<void> {
  const start = Date.now()
  while (!(await check())) {
    if (Date.now() - start > PATIENCE_MS) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}

/** Records as they go on the wire: what is undefined is not there. */
function wire<T>(records: T): T {
  return JSON.parse(JSON.stringify(records)) as T
}

/** A record as the server answers it: without the registry's revision and stamps. */
function plain<T extends object>(records: readonly T[]): T[] {
  return records.map((r) => {
    const {
      rev: _rev,
      statusAt: _statusAt,
      exitAt: _exitAt,
      ...rest
    } = r as T & { rev?: number; statusAt?: unknown; exitAt?: unknown }
    return rest as T
  })
}

describe.skipIf(!runnable)('vornd keeps a copy of the session registry that agrees', () => {
  let dataDir: string
  let closeServer: (() => Promise<void>) | undefined
  let direct: Client
  let through: Client
  let vorndPort: number
  let work: { projA: string; projC: string; wt: string; agent: string; ends: string }
  let server: {
    ptyManager: typeof import('../packages/server/src/pty-manager').ptyManager
    headlessManager: typeof import('../packages/server/src/headless-manager').headlessManager
    vorndSessions: typeof import('../packages/server/src/vornd-sessions').vorndSessions
    SessionMirror: typeof Mirror
    hookServer: typeof import('../packages/server/src/hook-server').hookServer
  }
  /** How many of each call went through vornd, to check the counts against. */
  const made: Record<string, number> = { terminal: 0, headless: 0, worktree: 0 }
  const saved: Record<string, string | undefined> = {}

  async function counts(): Promise<Counts> {
    const res = await fetch(`http://127.0.0.1:${vorndPort}/vornd/health`)
    return ((await res.json()) as { groups: Counts }).groups
  }

  /** What the server holds, as one string, to tell when it has stopped changing. */
  function serverState(): string {
    return JSON.stringify([
      server.ptyManager.getActiveSessions(),
      server.headlessManager.getActiveSessions()
    ])
  }

  /**
   * Waits until the server's registry is still and vornd's copy says the same,
   * then reads the registry through vornd: each read is answered by the server
   * and compared with vornd's own answer from the copy.
   */
  async function compare(worktrees: string[] = []): Promise<void> {
    let last = ''
    let stillSince = 0
    await until('the registry to settle and the copy to agree', async () => {
      const now = serverState()
      if (now !== last) {
        last = now
        stillSince = Date.now()
        return false
      }
      if (Date.now() - stillSince < 300) return false
      // Asked on the channel the records go out on: every record sent before
      // it is in the answer.
      const copy = await server.vorndSessions.registry()
      if (!copy) return false
      // Listed as the server lists them, from the copy's records and order.
      const listed = new server.SessionMirror(() => {})
      listed.load(copy)
      // Compared as values: vornd writes a record's keys in an order of its own.
      return (
        isDeepStrictEqual(plain(listed.terminals()), wire(server.ptyManager.getActiveSessions())) &&
        isDeepStrictEqual(plain(copy.headless), wire(server.headlessManager.getActiveSessions()))
      )
    })
    const before = serverState()
    const answers: Array<[string, unknown]> = [
      ['terminal:listActive', undefined],
      ['headless:list', undefined],
      ...worktrees.map((w): [string, unknown] => ['worktree:activeSessions', w])
    ]
    for (const [method, params] of answers) {
      await through.result(method, params)
      made[method.split(':')[0]!]!++
    }
    // Nothing moved while the reads were made, or a mismatch would mean nothing.
    expect(serverState()).toBe(before)
    await until('every comparison to be counted', async () => {
      const c = await counts()
      return Object.entries(made).every(
        ([group, n]) => (c[group]?.shadowMatched ?? 0) + (c[group]?.shadowMismatched ?? 0) === n
      )
    })
    const c = await counts()
    const compared = Object.fromEntries(
      Object.keys(made).map((group) => [
        group,
        { matched: c[group]?.shadowMatched ?? 0, mismatched: c[group]?.shadowMismatched ?? 0 }
      ])
    )
    expect(compared).toEqual(
      Object.fromEntries(
        Object.entries(made).map(([group, n]) => [group, { matched: n, mismatched: 0 }])
      )
    )
    // The server follows vornd's copy too.
    await until('the mirror to catch up', () =>
      isDeepStrictEqual(
        plain(server.vorndSessions.mirror.terminals()),
        wire(server.ptyManager.getActiveSessions())
      )
    )
  }

  function record(id: string): TerminalSession | undefined {
    return server.ptyManager.getActiveSessions().find((s) => s.id === id)
  }

  function hook(session_id: string, cwd: string, hook_event_name = 'SessionStart'): void {
    server.hookServer.emit('hook-event', { session_id, cwd, hook_event_name })
  }

  beforeAll(async () => {
    for (const key of ['SECRET_VORN_BOOTSTRAP_TOKEN', 'VORN_VORND_PATH', 'VORND_GROUPS']) {
      saved[key] = process.env[key]
    }
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    process.env.VORN_VORND_PATH = vornd
    process.env.VORND_GROUPS = GROUPS
    // Short: the session holder's socket lives under it.
    dataDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-sess-')))
    const base = path.join(dataDir, 'work')
    work = {
      projA: path.join(base, 'proj-a'),
      projC: path.join(base, 'proj-c'),
      wt: path.join(base, 'wt-a'),
      agent: path.join(base, 'bin', 'fake-agent'),
      ends: path.join(base, 'bin', 'fake-headless')
    }
    for (const dir of [work.projA, work.projC, work.wt, path.dirname(work.agent)]) {
      fs.mkdirSync(dir, { recursive: true })
    }
    // An agent that says nothing and waits; a headless one that reads its
    // prompt, prints and ends.
    fs.writeFileSync(work.agent, '#!/bin/sh\nexec sleep 600\n', { mode: 0o755 })
    fs.writeFileSync(work.ends, '#!/bin/sh\ncat >/dev/null\necho done\nexit 3\n', {
      mode: 0o755
    })

    const { startServer } = await import('../packages/server/src/index')
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    try {
      const { app, port } = await startServer({ port: 0, dataDir })
      closeServer = () => app.close()
      direct = await Client.open(port)
    } finally {
      process.stdout.write = origWrite
    }
    server = {
      ptyManager: (await import('../packages/server/src/pty-manager')).ptyManager,
      headlessManager: (await import('../packages/server/src/headless-manager')).headlessManager,
      vorndSessions: (await import('../packages/server/src/vornd-sessions')).vorndSessions,
      SessionMirror: (await import('../packages/server/src/vornd-sessions')).SessionMirror,
      hookServer: (await import('../packages/server/src/hook-server')).hookServer
    }
    let state: { state?: string; port?: number } = {}
    await until('vornd', async () => {
      state = await direct.result<{ state?: string; port?: number }>('server:vornd')
      return state.state === 'on' && !!state.port
    })
    vorndPort = state.port!
    await until('vornd to ask for the records', () => server.vorndSessions.isNative())
    await until('the copy to be fed', async () => {
      const res = await fetch(`http://127.0.0.1:${vorndPort}/vornd/health`)
      const health = (await res.json()) as { registry?: { fed?: boolean } }
      return health.registry?.fed === true
    })
    expect((await counts()).terminal?.mode).toBe('shadow')
    through = await Client.open(vorndPort)
    const agents = {
      claude: { command: work.agent, args: [] },
      copilot: { command: work.agent, args: [] }
    }
    server.ptyManager.setAgentCommands(agents)
    server.headlessManager.setAgentCommands({ claude: { command: work.ends, args: [] } })
  }, 60_000)

  afterAll(async () => {
    direct?.close()
    through?.close()
    for (const s of server?.ptyManager.getActiveSessions() ?? []) {
      server.ptyManager.killPty(s.id)
    }
    // The session holder outlives the server, by design: ended here, once
    // vornd has seen every session end, so nothing writes to the data
    // directory while it is removed.
    if (vorndPort) {
      await until('every session to end', async () => {
        const res = await fetch(`http://127.0.0.1:${vorndPort}/vornd/sessions`)
        const report = (await res.json()) as { sessions?: unknown[] }
        return (report.sessions ?? []).length === 0
      }).catch(() => {})
    }
    const holder = vorndPort
      ? await fetch(`http://127.0.0.1:${vorndPort}/vornd/health`)
          .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
          .then((h) => h.sessiond?.current?.pid)
          .catch(() => undefined)
      : undefined
    await closeServer?.()
    if (holder) {
      try {
        process.kill(holder, 'SIGTERM')
      } catch {
        /* already gone */
      }
    }
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
    if (dataDir) {
      fs.rmSync(dataDir, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
    }
  }, 30_000)

  it('agrees after every change the app makes to its sessions', async () => {
    await compare([work.wt])

    // Shells, through vornd as the app creates them.
    const shellA = await through.result<TerminalSession>('shell:create', work.projA)
    const shellB = await through.result<TerminalSession>('shell:create', work.projA)
    await until('both shells to start', () => [shellA, shellB].every((s) => record(s.id)!.pid > 0))
    await compare()

    // An agent in a worktree, linked by a hook as Claude's SessionStart does.
    const claude = await through.result<TerminalSession>('terminal:create', {
      agentType: 'claude',
      projectName: 'proj-a',
      projectPath: work.projA,
      existingWorktreePath: work.wt
    })
    await until('the agent to start', () => record(claude.id)!.pid > 0)
    hook('claude-conversation', work.wt)
    await until(
      'the hook to link it',
      () =>
        record(claude.id)?.hookSessionId === 'claude-conversation' &&
        record(claude.id)?.statusSource === 'hooks'
    )
    await compare([work.wt])
    expect(await through.result('worktree:activeSessions', work.wt)).toEqual({
      count: 1,
      sessionIds: [claude.id]
    })
    made.worktree++

    // Copilot is linked when it is created, by the hooks file written for it.
    const copilot = await through.result<TerminalSession>('terminal:create', {
      agentType: 'copilot',
      projectName: 'proj-c',
      projectPath: work.projC
    })
    await until('copilot to start', () => record(copilot.id)!.pid > 0)
    const linked = record(copilot.id)?.hookSessionId
    if (linked) {
      hook(linked, work.projC)
      await until('copilot on hooks', () => record(copilot.id)?.statusSource === 'hooks')
    }
    await compare([work.wt, work.projC])

    // What a person does to the cards.
    await through.result('terminal:rename', { id: shellA.id, displayName: 'Build' })
    await through.result('terminal:setGroup', { id: shellB.id, groupId: 'group-1' })
    await through.result('terminal:reorder', [copilot.id, shellB.id, claude.id, shellA.id])
    await through.result('terminal:setGroup', { id: shellB.id, groupId: null })
    await compare([work.wt])

    // A shell that ends keeps its card, idle, with how it ended.
    server.ptyManager.writeToPty(shellB.id, 'exit 3\r')
    await until('the shell to end', () => record(shellB.id)?.shellExitCode === 3)
    await compare()

    // Resumed under the same id, with the fields of the record it replaces.
    const resumed = await through.result<{ ok: boolean; session?: TerminalSession }>(
      'sessions:resume',
      { id: shellB.id }
    )
    expect(resumed.ok).toBe(true)
    await until('the resumed shell', () => {
      const r = record(shellB.id)
      return !!r && r.pid > 0 && r.shellExitCode === undefined
    })
    await compare()

    // A card closed.
    await through.result('terminal:kill', shellA.id)
    await until('the card to go', () => record(shellA.id) === undefined)
    await compare([work.wt])

    // A headless agent, from start to its exit.
    const agent = await through.result<HeadlessSession>('headless:create', {
      agentType: 'claude',
      projectName: 'proj-a',
      projectPath: work.projA,
      existingWorktreePath: work.wt,
      initialPrompt: 'write the tests'
    })
    await until(
      'the headless agent to end',
      () =>
        server.headlessManager.getActiveSessions().find((s) => s.id === agent.id)?.status ===
        'exited'
    )
    await compare([work.wt])

    // And the comparison can fail: a record changed in place without telling
    // vornd is counted as a mismatch.
    const drifted = server.ptyManager.getActiveSessions()[0]!
    drifted.displayName = 'changed behind vornd'
    await through.result('terminal:listActive')
    await until(
      'the mismatch to be counted',
      async () => (await counts()).terminal?.shadowMismatched === 1
    )
  }, 120_000)
})
