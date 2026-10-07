/**
 * vornd's copy of the server's session registry, against the registry itself;
 * then the terminals vornd creates and changes with the Native server switch
 * on, against the server's with it off.
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
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { isDeepStrictEqual } from 'node:util'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '../packages/shared/src/protocol'
import type { HeadlessSession, TerminalSession } from '../packages/shared/src/types'
import type { SessionMirror as Mirror } from '../packages/server/src/vornd-sessions'
import { spawnsRealServers } from './helpers/one-at-a-time'
import {
  normalizeRun,
  outputWhole,
  withoutHookLinks,
  type RunDirs
} from './helpers/sessions-parity'

const TEST_CREDENTIAL = 'native-server-sessions-credential'
const GROUPS = 'terminal=shadow,shell=shadow,headless=shadow,worktree=shadow,git=shadow'

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
  let work: { projA: string; projC: string; wt: string; agent: string }
  let server: {
    ptyManager: typeof import('../packages/server/src/pty-manager').ptyManager
    headlessManager: typeof import('../packages/server/src/headless-manager').headlessManager
    vorndSessions: typeof import('../packages/server/src/vornd-sessions').vorndSessions
    SessionMirror: typeof Mirror
    hookServer: typeof import('../packages/server/src/hook-server').hookServer
  }
  /**
   * How many compared calls went through vornd, to check the counts against:
   * the reads, and the calls that create or change a terminal, whose plan or
   * refusal vornd works out beside the server's.
   */
  const made: Record<string, number> = { terminal: 0, shell: 0, headless: 0, worktree: 0 }
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
      agent: path.join(base, 'bin', 'fake-agent')
    }
    for (const dir of [work.projA, work.projC, work.wt, path.dirname(work.agent)]) {
      fs.mkdirSync(dir, { recursive: true })
    }
    // An agent that says nothing and waits on a terminal; headless, on pipes,
    // it reads its prompt, prints and ends.
    fs.writeFileSync(
      work.agent,
      '#!/bin/sh\nif [ -t 0 ]; then exec sleep 600; fi\ncat >/dev/null\necho done\nexit 3\n',
      { mode: 0o755 }
    )

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
    // The holder too: the server asks for a spawn once it is up, and a
    // create's comparison waits for that spawn.
    await until('the copy to be fed and the session holder up', async () => {
      const res = await fetch(`http://127.0.0.1:${vorndPort}/vornd/health`)
      const health = (await res.json()) as {
        registry?: { fed?: boolean }
        sessiond?: { current?: { pid?: number } }
      }
      return health.registry?.fed === true && !!health.sessiond?.current?.pid
    })
    expect((await counts()).terminal?.mode).toBe('shadow')
    through = await Client.open(vorndPort)
    // Saved, not set in place: vornd reads the agents' commands from the database.
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      agentCommands: {
        claude: { command: work.agent, args: [] },
        copilot: { command: work.agent, args: [] }
      }
    })
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
    made.shell += 2
    await compare()

    // An agent in a worktree, linked by a hook as Claude's SessionStart does.
    const claude = await through.result<TerminalSession>('terminal:create', {
      agentType: 'claude',
      projectName: 'proj-a',
      projectPath: work.projA,
      existingWorktreePath: work.wt
    })
    await until('the agent to start', () => record(claude.id)!.pid > 0)
    made.terminal++
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
    made.terminal++
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
    // And what the server refuses: vornd would have refused it in the same words.
    const twice = await through.call('terminal:reorder', [shellA.id, shellA.id])
    expect((twice.error as { message: string }).message).toBe('Duplicate session IDs')
    const missing = await through.call('terminal:rename', { id: 'no-such', displayName: 'x' })
    expect((missing.error as { message: string }).message).toBe('Session not found: no-such')
    made.terminal += 6
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
    made.terminal++
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
    // Its create was compared too, as the spawn each side would ask for.
    made.headless++
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

  it('foresees a worktree’s branch rename and move as the server answers them', async () => {
    const base = path.join(dataDir, 'work')
    const repo = path.join(base, 'shadow-repo')
    repository(repo)
    const wt = path.join(base, 'shadow-1a2b3c4d')
    execFileSync('git', ['worktree', 'add', '-q', '-b', 'shadowed', wt], {
      cwd: repo,
      stdio: 'ignore'
    })
    const git = async (): Promise<Required<Counts[string]> & { shadowUnported: number }> => {
      const g = (await counts()).git as Counts[string] & { shadowUnported?: number }
      return {
        mode: g?.mode ?? '',
        forwarded: g?.forwarded ?? 0,
        shadowMatched: g?.shadowMatched ?? 0,
        shadowMismatched: g?.shadowMismatched ?? 0,
        shadowUnported: g?.shadowUnported ?? 0
      }
    }
    const before = await git()
    expect(before.mode).toBe('shadow')
    const rename = (newBranch: string): Promise<unknown> =>
      through.result('git:renameWorktreeBranch', { worktreePath: wt, newBranch })
    expect(await rename('main')).toBe(false)
    expect(await rename('renamed')).toBe(true)
    expect(
      await through.result('git:renameWorktree', { worktreePath: wt, newName: 'moved' })
    ).toEqual({ newPath: path.join(base, 'moved-1a2b3c4d'), name: 'moved' })
    const settled = (g: Awaited<ReturnType<typeof git>>): number =>
      g.shadowMatched + g.shadowMismatched + g.shadowUnported
    await until('the comparisons', async () => settled(await git()) - settled(before) === 3)
    const after = await git()
    // A branch name is judged only where gix reads the repository; the move always is.
    expect(after.shadowMismatched - before.shadowMismatched).toBe(0)
    expect(after.shadowMatched - before.shadowMatched).toBeGreaterThanOrEqual(1)
  })
})

spawnsRealServers()

const AGENTS = ['claude', 'codex', 'copilot', 'gemini', 'opencode'] as const

/**
 * A stub agent: it says what it was started with, and waits. Headless, on
 * pipes, it reads its prompt and says it, waits if told to, and ends with 3.
 */
const ARGV_AGENT = `#!/bin/sh
printf 'ARGV:%s\\n' "$*"
if [ -t 0 ]; then exec sleep 600; fi
p=$(cat)
printf 'PROMPT:%s\\n' "$p"
case "$p" in *wait*) sleep 600;; esac
exit 3
`

/** A client of one server that keeps every notification it is told. */
class Watcher {
  private next = 1
  readonly told: Array<{ method: string; params: unknown }> = []

  private constructor(private ws: WebSocket) {
    ws.on('message', (raw) => {
      const frame = JSON.parse(String(raw)) as { method?: string; params?: unknown }
      if (frame.method) this.told.push({ method: frame.method, params: frame.params })
    })
  }

  static open(port: number): Promise<Watcher> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${TEST_CREDENTIAL}` }
    })
    return new Promise((resolve, reject) => {
      ws.once('open', () => resolve(new Watcher(ws)))
      ws.once('error', reject)
    })
  }

  call(method: string, params?: unknown): Promise<Frame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 30_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(String(raw)) as Frame
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

  /** What every client was told by `method`. */
  toldBy(method: string): unknown[] {
    return this.told.filter((t) => t.method === method).map((t) => t.params)
  }

  close(): void {
    this.ws.close()
  }
}

interface RealServer {
  child: ChildProcess
  dirs: RunDirs
  port: number
  vornd: number
  log: string[]
}

const realServers: RealServer[] = []

async function startRealServer(nativeServer: boolean): Promise<RealServer> {
  const made = (name: string): string =>
    fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-parity-${name}-`)))
  // Its own home: the agents' hook settings and the hook endpoint are there.
  const dirs = { home: made('home'), data: made('data'), work: made('work') }
  const log: string[] = []
  const child = spawn(
    process.execPath,
    [
      '--import',
      'tsx',
      path.join(__dirname, '..', 'packages', 'server', 'src', 'index.ts'),
      '--data-dir',
      dirs.data,
      '--port',
      '0'
    ],
    {
      cwd: path.join(__dirname, '..'),
      env: {
        ...process.env,
        HOME: dirs.home,
        [BOOTSTRAP_ENV_VAR]: TEST_CREDENTIAL,
        VORN_VORND_PATH: vornd!,
        VORN_NATIVE_SERVER: nativeServer ? '1' : '0',
        VORND_NATIVE_SERVER: '',
        VORND_GROUPS: '',
        NODE_ENV: 'test',
        VITEST: ''
      },
      stdio: ['ignore', 'pipe', 'pipe']
    }
  )
  child.stdout?.on('data', (d) => log.push(String(d)))
  child.stderr?.on('data', (d) => log.push(String(d)))
  const server: RealServer = { child, dirs, port: 0, vornd: 0, log }
  realServers.push(server)
  await until('the server to listen', () => {
    try {
      const record = JSON.parse(fs.readFileSync(path.join(dirs.data, WS_PORT_FILENAME), 'utf-8'))
      server.port = typeof record.port === 'number' ? (record.port as number) : 0
    } catch {
      server.port = 0
    }
    return server.port > 0
  })
  const direct = await Watcher.open(server.port)
  await until('vornd to start', async () => {
    const s = await direct.result<{ state: string; port?: number; nativeServer?: boolean }>(
      'server:vornd'
    )
    if (s.state !== 'on' || !s.port) return false
    expect(s.nativeServer).toBe(nativeServer)
    server.vornd = s.port
    return true
  })
  await until('the session holder, and with the switch the copy deciding', async () => {
    const res = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const health = (await res.json()) as {
      sessiond?: { current?: { pid?: number } }
      registry?: { fed?: boolean; decides?: boolean }
    }
    if (!health.sessiond?.current?.pid) return false
    return !nativeServer || (health.registry?.fed === true && health.registry.decides === true)
  })
  direct.close()
  return server
}

async function stopRealServer(server: RealServer): Promise<void> {
  const holder = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    .then((r) => r.json() as Promise<{ sessiond?: { current?: { pid?: number } } }>)
    .then((h) => h.sessiond?.current?.pid)
    .catch(() => undefined)
  if (server.child.exitCode === null) {
    const exited = new Promise((r) => server.child.once('exit', r))
    server.child.kill()
    await exited
  }
  // The session holder outlives the server, by design, and its sessions with it.
  if (holder) {
    try {
      process.kill(holder, 'SIGTERM')
    } catch {
      /* already gone */
    }
  }
}

/** A repository with one commit, the same commit on every run. */
function repository(dir: string): void {
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(path.join(dir, 'README'), 'parity\n')
  const env = {
    ...process.env,
    GIT_AUTHOR_NAME: 'Vorn',
    GIT_AUTHOR_EMAIL: 'vorn@example.invalid',
    GIT_COMMITTER_NAME: 'Vorn',
    GIT_COMMITTER_EMAIL: 'vorn@example.invalid',
    GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z',
    GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z'
  }
  const git = (...args: string[]): void => {
    execFileSync('git', args, { cwd: dir, env, stdio: 'ignore' })
  }
  git('init', '-q', '-b', 'main')
  git('add', 'README')
  git('commit', '-q', '-m', 'first')
}

/** A call's answer as the transcript keeps it: its result, or its error's message. */
function answered(frame: Frame): unknown {
  if (frame.error) return { error: (frame.error as { message?: string }).message }
  return 'result' in frame ? { result: frame.result } : { void: true }
}

/** The same calls, on one server, through its vornd; answers the transcript. */
async function scenario(server: RealServer): Promise<Record<string, unknown>> {
  const { work } = server.dirs
  const stub = path.join(work, 'bin', 'argv-agent')
  fs.mkdirSync(path.dirname(stub))
  fs.writeFileSync(stub, ARGV_AGENT, { mode: 0o755 })
  const repo = path.join(work, 'repo')
  repository(repo)

  const direct = await Watcher.open(server.port)
  const through = await Watcher.open(server.vornd)
  const replies: Record<string, unknown> = {}
  const call = async (step: string, method: string, params?: unknown): Promise<Frame> => {
    const frame = await through.call(method, params)
    replies[step] = answered(frame)
    return frame
  }
  const created = async (step: string, method: string, params?: unknown): Promise<string> => {
    const frame = await call(step, method, params)
    if (frame.error) throw new Error(`${step}: ${JSON.stringify(frame.error)}`)
    return (frame.result as TerminalSession).id
  }
  const listed = (): Promise<TerminalSession[]> =>
    direct.result<TerminalSession[]>('terminal:listActive')
  const live = async (ids: string[]): Promise<void> => {
    await until('the sessions to start', async () => {
      const all = await listed()
      return ids.every((id) => (all.find((s) => s.id === id)?.pid ?? 0) > 0)
    })
  }
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      defaults: { ...(config.defaults as object), shell: '/bin/sh' },
      agentCommands: Object.fromEntries(AGENTS.map((a) => [a, { command: stub, args: [] }]))
    })

    // An agent of every kind, each in a project of its own.
    const agents: Record<string, string> = {}
    for (const agent of AGENTS) {
      const project = path.join(work, agent)
      fs.mkdirSync(project)
      agents[agent] = await created(`create ${agent}`, 'terminal:create', {
        agentType: agent,
        projectName: agent,
        projectPath: project,
        displayName: agent === 'gemini' ? 'Named' : undefined,
        initialPrompt: agent === 'opencode' ? 'write the parity test' : undefined
      })
    }
    await live(Object.values(agents))
    const argv: Record<string, string> = {}
    const shown: Record<string, string[]> = {}
    await until('every agent to say what it was started with', async () => {
      for (const agent of AGENTS) {
        const out = await through.result<string[]>('terminal:readOutput', { id: agents[agent] })
        shown[agent] = out
        // After the shell's prompt, when the agent was started before the shell drew it.
        const line = out.find((l) => l.includes('ARGV:'))
        if (!line) return false
        argv[agent] = line.slice(line.indexOf('ARGV:')).trimEnd()
      }
      return true
    }).catch((err: Error) => {
      throw new Error(`${err.message}; their screens: ${JSON.stringify(shown)}`)
    })

    // Shells: in a project, and in the home directory.
    const shellA = await created('shell in a project', 'shell:create', path.join(work, 'claude'))
    await live([shellA])
    const shellB = await created('shell at home', 'shell:create')
    await live([shellB])

    // An agent in a new worktree, which is the only session there.
    const inWorktree = await created('create in a worktree', 'terminal:create', {
      agentType: 'claude',
      projectName: 'repo',
      projectPath: repo,
      useWorktree: true,
      branch: 'feature',
      worktreeName: 'wt-one'
    })
    await live([inWorktree])

    // The worktree's branch renamed and the worktree moved, and a rename refused.
    const worktreeOf = async (): Promise<string> =>
      (await listed()).find((s) => s.id === inWorktree)!.worktreePath!
    const renamed = { worktreePath: await worktreeOf(), newBranch: 'feature-two' }
    await call('rename the worktree branch', 'git:renameWorktreeBranch', renamed)
    await call('rename it to a branch that is taken', 'git:renameWorktreeBranch', {
      worktreePath: renamed.worktreePath,
      newBranch: 'main'
    })
    await call('rename the worktree', 'git:renameWorktree', {
      worktreePath: renamed.worktreePath,
      newName: 'wt two'
    })
    await call('rename a worktree that moved', 'git:renameWorktree', {
      worktreePath: renamed.worktreePath,
      newName: 'wt three'
    })
    await until(
      'the moved worktree to be listed',
      async () => (await worktreeOf()) !== renamed.worktreePath
    )
    // One conversation asked for twice at once: one session, both answered with it.
    const named = {
      agentType: 'codex',
      projectName: 'codex',
      projectPath: path.join(work, 'codex'),
      resumeSessionId: 'conversation-twice'
    }
    const [first, second] = await Promise.all([
      through.call('terminal:create', named),
      through.call('terminal:create', named)
    ])
    const once = (first.result as TerminalSession).id
    replies['one conversation twice at once'] = {
      same: once === (second.result as TerminalSession).id
    }
    await live([once])
    const third = await through.result<TerminalSession>('terminal:create', named)
    replies['one conversation again'] = { same: third.id === once }

    // What a person does to the cards, and what is refused.
    await call('rename', 'terminal:rename', { id: shellA, displayName: 'Build' })
    await call('group', 'terminal:setGroup', { id: shellB, groupId: 'group-1' })
    const order = [shellB, ...Object.values(agents), inWorktree, once, shellA]
    await call('reorder', 'terminal:reorder', order)
    await call('ungroup', 'terminal:setGroup', { id: shellB, groupId: '' })
    await call('rename a card that is not there', 'terminal:rename', {
      id: 'no-such-card',
      displayName: 'x'
    })
    await call('reorder twice over', 'terminal:reorder', [shellA, shellA])
    await call('reorder with one missing', 'terminal:reorder', [shellA, 'no-such-card'])

    // The last session in the worktree closed: one offer to clean it up.
    // Each close waits for its exit, as the server tells it and as a client
    // through vornd hears it (from whoever tells it, so in an order of its own).
    const exited = async (id: string): Promise<void> => {
      const told = (w: Watcher): boolean =>
        w.toldBy('terminal:exit').some((p) => (p as { id?: string }).id === id)
      await until(`the exit of ${id}`, () => told(direct) && told(through))
    }
    await call('close the worktree agent', 'terminal:kill', inWorktree)
    await exited(inWorktree)
    await call('close a shell', 'terminal:kill', shellB)
    await until('the shell to go', async () => !(await listed()).some((s) => s.id === shellB))
    await exited(shellB)
    // A card that is not there: the server tells its exit anyway.
    await call('close a card that is not there', 'terminal:kill', 'no-such-card')
    await exited('no-such-card')

    // A shell opened again after the closes is numbered after the ones left.
    const shellC = await created('shell after a close', 'shell:create', path.join(work, 'claude'))
    await live([shellC])

    // A headless agent of every kind, each run to its end, and one stopped.
    const headless: Record<string, string> = {}
    for (const agent of AGENTS) {
      headless[agent] = await created(`headless ${agent}`, 'headless:create', {
        agentType: agent,
        projectName: agent,
        projectPath: path.join(work, agent),
        initialPrompt: `print the ${agent} parity\nline two`,
        workflowId: 'wf-1',
        workflowName: 'Parity'
      })
    }
    const stopped = await created('headless to stop', 'headless:create', {
      agentType: 'claude',
      projectName: 'claude',
      projectPath: path.join(work, 'claude'),
      displayName: 'Waits',
      initialPrompt: 'wait here'
    })
    const headlessEnded = async (ids: string[]): Promise<void> => {
      await until('the headless agents to end', async () => {
        const all = await direct.result<HeadlessSession[]>('headless:list')
        const exits = direct.toldBy('headless:exit') as { id: string }[]
        return ids.every(
          (id) =>
            all.find((s) => s.id === id)?.status === 'exited' && exits.some((e) => e.id === id)
        )
      })
    }
    await headlessEnded(Object.values(headless))
    // Stopped once it has said its prompt: a stop can reach an agent before it prints.
    await until('the waiting agent to say its prompt', () =>
      direct
        .toldBy('headless:data')
        .some(
          (p) =>
            (p as { id: string; data: string }).id === stopped &&
            /wait here/.test((p as { data: string }).data)
        )
    )
    await call('stop a headless agent', 'headless:kill', stopped)
    await headlessEnded([stopped])
    await call('stop one that ended', 'headless:kill', headless.claude)
    await call('stop one that is not there', 'headless:kill', 'no-such-agent')
    const agentsListed = await direct.result<HeadlessSession[]>('headless:list')
    const byAgent = { ...headless, stopped }

    // Settled: the registry stops changing.
    let last = ''
    let since = Date.now()
    await until('the registry to settle', async () => {
      const now = JSON.stringify(await listed())
      if (now !== last) {
        last = now
        since = Date.now()
      }
      return Date.now() - since > 500
    })
    const toldOf = (method: string): unknown[] => direct.toldBy(method)
    const health = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const groups = (
      (await health.json()) as { groups: Counts & Record<string, { native?: number }> }
    ).groups
    const by = (group: string): { native?: number; forwarded?: number } => ({
      native: groups[group]?.native,
      forwarded: groups[group]?.forwarded
    })
    const exits = toldOf('headless:exit') as { id: string; exitCode: number }[]
    const exitsTold = toldOf('terminal:exit').map((p) => (p as { id: string }).id)
    const heardThrough = through.toldBy('terminal:exit').map((p) => (p as { id: string }).id)
    return {
      answeredBy: {
        terminal: by('terminal'),
        shell: by('shell'),
        headless: by('headless'),
        git: by('git')
      },
      replies: withoutHookLinks(replies),
      argv,
      listed: await listed(),
      agentsListed: Object.fromEntries(
        Object.entries(byAgent).map(([name, id]) => [name, agentsListed.find((s) => s.id === id)])
      ),
      agentsOutput: outputWhole(toldOf('headless:data') as { id: string; data: string }[], byAgent),
      agentsExits: Object.fromEntries(
        Object.entries(byAgent).map(([name, id]) => [name, exits.find((e) => e.id === id)])
      ),
      told: {
        created: toldOf('session:created'),
        reordered: toldOf('session:reordered'),
        cleanup: toldOf('worktree:confirmCleanup'),
        // In the order they were closed: each waited for the one before.
        exits: exitsTold,
        // As a client through vornd hears them: once each, whoever tells it,
        // in an order of its own, so listed in the server's.
        exitsThrough: {
          once: new Set(heardThrough).size === heardThrough.length,
          ids: exitsTold.filter((id) => heardThrough.includes(id)),
          unheard: exitsTold.filter((id) => !heardThrough.includes(id))
        },
        renamed: toldOf('session:updated')
          .map((p) => p as TerminalSession)
          .filter((s) => s.displayName === 'Build' || s.groupId === 'group-1')
          .map((s) => ({ id: s.id, displayName: s.displayName, groupId: s.groupId })),
        // Each change to the worktree agent's branch, path and name, once.
        moved: [
          ...new Set(
            toldOf('session:updated')
              .map((p) => p as TerminalSession)
              .filter((s) => s.id === inWorktree)
              .map((s) => JSON.stringify([s.branch, s.worktreePath, s.worktreeName]))
          )
        ]
      }
    }
  } finally {
    direct.close()
    through.close()
  }
}

describe.skipIf(!runnable)('the terminals vornd creates and changes, against the server', () => {
  const runs: Partial<Record<'off' | 'on', Record<string, unknown>>> = {}

  beforeAll(async () => {
    for (const [mode, on] of [
      ['off', false],
      ['on', true]
    ] as const) {
      const server = await startRealServer(on)
      try {
        runs[mode] = normalizeRun(await scenario(server), server.dirs)
      } catch (err) {
        throw new Error(`${mode}: ${(err as Error).message}\n${server.log.join('').slice(-4000)}`, {
          cause: err
        })
      } finally {
        await stopRealServer(server)
      }
    }
  }, 240_000)

  afterAll(() => {
    for (const s of realServers) {
      for (const dir of Object.values(s.dirs)) {
        fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 })
      }
    }
  })

  it('creates, starts and changes terminals as the server does with the switch off', () => {
    const off = runs.off as {
      replies: Record<string, unknown>
      told: { cleanup: unknown[]; moved: string[] }
    }
    // What the switch must not change, read off the server's own run.
    expect(off.replies['one conversation twice at once']).toEqual({ same: true })
    expect(off.replies['one conversation again']).toEqual({ same: true })
    expect(off.told.cleanup).toHaveLength(1)
    expect(JSON.parse(off.told.moved.at(-1)!)).toEqual([
      'feature-two',
      expect.stringMatching(/\/wt-two-<id>$/),
      'wt-two'
    ])
    expect(runs.off?.answeredBy).toEqual({
      terminal: { native: 0, forwarded: 19 },
      shell: { native: 0, forwarded: 3 },
      headless: { native: 0, forwarded: 9 },
      git: { native: 0, forwarded: 4 }
    })
    const exits = runs.off as { agentsExits: Record<string, { exitCode: number }> }
    expect(Object.values(exits.agentsExits).map((e) => e.exitCode)).toEqual([3, 3, 3, 3, 3, 143])
  })

  it('has vornd answer them with the switch on, and the server what is its own', () => {
    // Refused by the server in its words: a card that is not there, a
    // duplicate in an order, one missing from it; and a close of a card that
    // is not there, which the server tells clients of anyway.
    // A stop of an agent the registry does not hold is the server's, which
    // answers nothing for it too.
    expect(runs.on?.answeredBy).toEqual({
      terminal: { native: 15, forwarded: 4 },
      shell: { native: 3, forwarded: 0 },
      headless: { native: 8, forwarded: 1 },
      git: { native: 4, forwarded: 0 }
    })
  })

  it('answers, starts, tells and lists the same with the switch on', () => {
    for (const part of [
      'replies',
      'argv',
      'told',
      'listed',
      'agentsListed',
      'agentsOutput',
      'agentsExits'
    ] as const) {
      expect([part, runs.on?.[part]]).toEqual([part, runs.off?.[part]])
    }
  })
})
