/**
 * The worktree manager with the Native server switch on, against the server
 * with it off: the same repository, the same calls through vornd, the same
 * answers once {@link normalizeWorktrees} has applied the accepted differences.
 *
 * Runs where vornd and vorn-sessiond have been built (`yarn build:core`, or
 * the binaries in `VORN_CONFORMANCE_VORND`), on a Unix: the agent is a shell
 * script.
 */
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { BOOTSTRAP_ENV_VAR, WS_PORT_FILENAME } from '../packages/shared/src/protocol'
import { spawnsRealServers } from './helpers/one-at-a-time'
import { normalizeWorktrees } from './helpers/worktrees-parity'
import { vorndStopped } from './helpers/real-server'

const TEST_CREDENTIAL = 'native-server-worktrees-credential'

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, '../packages/core/vornd'),
  path.resolve(__dirname, '../packages/core/target/release/vornd')
].find((p): p is string => !!p && fs.existsSync(p))
const runnable =
  !!vornd &&
  process.platform !== 'win32' &&
  fs.existsSync(path.join(path.dirname(vornd), 'vorn-sessiond'))

vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

type Frame = Record<string, unknown>

const PATIENCE_MS = 30_000

async function until(what: string, check: () => boolean | Promise<boolean>): Promise<void> {
  const start = Date.now()
  while (!(await check())) {
    if (Date.now() - start > PATIENCE_MS) throw new Error(`timed out waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 50))
  }
}

/** A headless agent that reads its prompt and waits. */
const WAITING_AGENT = `#!/bin/sh
cat >/dev/null
exec sleep 600
`

class Client {
  private next = 1
  private constructor(private ws: WebSocket) {}

  static async open(port: number): Promise<Client> {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, {
      headers: { authorization: `Bearer ${TEST_CREDENTIAL}` }
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

  close(): void {
    this.ws.close()
  }
}

interface RealServer {
  child: ChildProcess
  dirs: { home: string; data: string; work: string }
  port: number
  vornd: number
  log: string[]
}

const realServers: RealServer[] = []

async function startRealServer(nativeServer: boolean): Promise<RealServer> {
  const made = (name: string): string =>
    fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), `vorn-wt-${name}-`)))
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
  const direct = await Client.open(server.port)
  await until('vornd to start', async () => {
    const s = await direct.result<{ state: string; port?: number; nativeServer?: boolean }>(
      'server:vornd'
    )
    if (s.state !== 'on' || !s.port) return false
    expect(s.nativeServer).toBe(nativeServer)
    server.vornd = s.port
    return true
  })
  await until('the session holder, and with the switch the copy fed', async () => {
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
  await vorndStopped(server.vornd)
  if (holder) {
    try {
      process.kill(holder, 'SIGTERM')
    } catch {
      /* already gone */
    }
  }
}

/**
 * A repository with the same commits on every run, and worktrees beside it:
 * one merged, one unmerged with build output, one with uncommitted work, one
 * an agent works in, and a directory git has forgotten.
 */
function repository(work: string): { repo: string; wt: (name: string) => string } {
  const repo = path.join(work, 'repo')
  const wt = (name: string): string => path.join(work, '.vorn-worktrees', 'repo', name)
  fs.mkdirSync(repo, { recursive: true })
  fs.writeFileSync(path.join(repo, 'README'), 'parity\n')
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_AUTHOR_NAME: 'Vorn',
    GIT_AUTHOR_EMAIL: 'vorn@example.invalid',
    GIT_COMMITTER_NAME: 'Vorn',
    GIT_COMMITTER_EMAIL: 'vorn@example.invalid',
    GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z',
    GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z'
  }
  const git = (cwd: string, ...args: string[]): void => {
    execFileSync('git', args, { cwd, env, stdio: 'ignore' })
  }
  git(repo, 'init', '-q', '-b', 'main')
  git(repo, 'add', 'README')
  git(repo, 'commit', '-q', '-m', 'first')
  git(repo, 'branch', 'stale')
  for (const name of ['merged', 'unmerged', 'dirty', 'busy']) {
    git(repo, 'worktree', 'add', '-q', '-b', name, wt(name))
  }
  git(wt('unmerged'), 'commit', '-q', '--allow-empty', '-m', 'unmerged work')
  fs.mkdirSync(path.join(wt('unmerged'), 'node_modules', 'dep'), { recursive: true })
  fs.writeFileSync(path.join(wt('unmerged'), 'node_modules', 'dep', 'index.js'), 'x'.repeat(8192))
  fs.writeFileSync(path.join(wt('dirty'), 'notes.txt'), 'not committed\n')
  fs.mkdirSync(wt('orphan'))
  fs.writeFileSync(path.join(wt('orphan'), 'left.txt'), 'left behind\n')
  return { repo, wt }
}

function answered(frame: Frame): unknown {
  if (frame.error) return { error: (frame.error as { message?: string }).message }
  return { result: frame.result }
}

/** The same calls, on one server, through its vornd; answers the transcript. */
async function scenario(
  server: RealServer
): Promise<{ transcript: Record<string, unknown>; sessions: string[] }> {
  const { work } = server.dirs
  const stub = path.join(work, 'bin', 'waiting-agent')
  fs.mkdirSync(path.dirname(stub))
  fs.writeFileSync(stub, WAITING_AGENT, { mode: 0o755 })
  const { repo, wt } = repository(work)

  const direct = await Client.open(server.port)
  const through = await Client.open(server.vornd)
  const replies: Record<string, unknown> = {}
  const call = async (step: string, method: string, params?: unknown): Promise<void> => {
    replies[step] = answered(await through.call(method, params))
  }
  try {
    const config = await direct.result<Record<string, unknown>>('config:load')
    await direct.result('config:save', {
      ...config,
      projects: [{ name: 'repo', path: repo, preferredAgents: ['claude'] }],
      agentCommands: { claude: { command: stub, args: [] } }
    })
    const agent = await through.result<{ id: string }>('headless:create', {
      agentType: 'claude',
      projectName: 'repo',
      projectPath: repo,
      existingWorktreePath: wt('busy'),
      initialPrompt: 'wait here'
    })
    await until('the agent to be at work in its worktree', async () => {
      const active = await direct.result<{ count: number }>('worktree:activeSessions', wt('busy'))
      return active.count === 1
    })

    await call('inventory', 'worktree:inventory')
    await call('inventory of one project, measured again', 'worktree:inventory', {
      projectPaths: [repo],
      refresh: true
    })
    await call('remove where an agent works', 'worktree:removeMany', {
      items: [{ projectPath: repo, worktreePath: wt('busy'), force: true }]
    })
    await call('prune where an agent works', 'worktree:pruneOrphans', { paths: [wt('busy')] })
    await call('remove uncommitted and merged', 'worktree:removeMany', {
      items: [
        { projectPath: repo, worktreePath: wt('dirty') },
        { projectPath: repo, worktreePath: wt('merged'), deleteBranch: true }
      ]
    })
    await call('reclaim build output', 'worktree:reclaimArtifacts', { paths: [wt('unmerged')] })
    await call('prune a registered worktree and a forgotten one', 'worktree:pruneOrphans', {
      paths: [wt('unmerged'), wt('orphan')]
    })
    await call('remove one, forced', 'git:removeWorktree', {
      projectPath: repo,
      worktreePath: wt('dirty'),
      force: true
    })
    await call('inventory after', 'worktree:inventory')

    await direct.result('headless:kill', agent.id)
    const health = await fetch(`http://127.0.0.1:${server.vornd}/vornd/health`)
    const groups = (
      (await health.json()) as {
        groups: Record<string, { native?: number; forwarded?: number }>
      }
    ).groups
    const transcript = {
      answeredBy: {
        worktree: { native: groups.worktree?.native, forwarded: groups.worktree?.forwarded },
        git: { native: groups.git?.native, forwarded: groups.git?.forwarded }
      },
      replies,
      left: ['merged', 'unmerged', 'dirty', 'busy', 'orphan'].filter((n) => fs.existsSync(wt(n))),
      buildOutput: fs.existsSync(path.join(wt('unmerged'), 'node_modules'))
    }
    return { transcript, sessions: [agent.id] }
  } finally {
    direct.close()
    through.close()
  }
}

spawnsRealServers()

describe.skipIf(!runnable)('the worktree manager in vornd, against the server', () => {
  const runs: Partial<Record<'off' | 'on', Record<string, unknown>>> = {}

  beforeAll(async () => {
    for (const [mode, on] of [
      ['off', false],
      ['on', true]
    ] as const) {
      const server = await startRealServer(on)
      try {
        const { transcript, sessions } = await scenario(server)
        runs[mode] = normalizeWorktrees(transcript, server.dirs.work, sessions)
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

  it('keeps what an agent uses, reports uncommitted work and removes the rest', () => {
    const off = runs.off as { replies: Record<string, unknown>; left: string[] }
    expect(off.replies['remove where an agent works']).toEqual({
      error: '<work>/.vorn-worktrees/repo/busy has 1 active session — close them first'
    })
    const removed = off.replies['remove uncommitted and merged'] as {
      result: { succeeded: string[]; failed: { path: string }[]; deletedBranches: string[] }
    }
    expect(removed.result.succeeded).toEqual(['<work>/.vorn-worktrees/repo/merged'])
    expect(removed.result.failed.map((f) => f.path)).toEqual(['<work>/.vorn-worktrees/repo/dirty'])
    expect(removed.result.deletedBranches).toEqual(['merged'])
    expect(off.left).toEqual(['unmerged', 'busy'])
    expect(runs.off?.answeredBy).toEqual({
      worktree: { native: 0, forwarded: 8 },
      git: { native: 0, forwarded: 1 }
    })
  })

  it('has vornd answer them with the switch on', () => {
    expect(runs.on?.answeredBy).toEqual({
      worktree: { native: 8, forwarded: 0 },
      git: { native: 1, forwarded: 0 }
    })
  })

  it('answers and removes the same with the switch on', () => {
    for (const part of ['replies', 'left', 'buildOutput'] as const) {
      expect([part, runs.on?.[part]]).toEqual([part, runs.off?.[part]])
    }
  })
})
