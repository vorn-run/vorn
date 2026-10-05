/**
 * vornd's own answers to the `git:`, `file:` and `ide:` calls, against the
 * server's answers to the same calls.
 *
 * One server is started, and three vornds in front of it: one with the
 * Native server switch on, one with the desktop's launch token as well, and
 * one shadowing the same groups. Every call is made directly to the server
 * and through vornd, and the two frames a client receives must be the same
 * but for the differences `helpers/git-parity` names. A call that changes a
 * repository is made once on each side, on two copies of the same fixture.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { execFileSync, spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { createInterface } from 'node:readline'
import Database from 'libsql'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import {
  answerOf,
  fileMtime,
  fixtureRoot,
  linkedWorktreeOrder,
  madeUpBy,
  worktreeMadeUp,
  type Answer
} from './helpers/git-parity'

const TEST_CREDENTIAL = 'native-server-test-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

vi.mock('node-pty', () => ({
  default: { spawn: vi.fn() },
  spawn: vi.fn()
}))

// Booting a server probes Tailscale with a real process; nothing here needs it.
vi.mock('../packages/server/src/tailscale', () => ({
  getTailscaleStatus: vi.fn(async () => ({ running: false, selfIP: '', selfDNSName: '' })),
  clearBinaryCache: vi.fn()
}))

// The server's store is stubbed with no projects, so every path is local to
// it. vornd reads its own copy of the rows from the file the test writes.
vi.mock(
  '../packages/server/src/database',
  () =>
    ({
      closeDatabase: vi.fn(),
      initDatabase: vi.fn(),
      getDataDir: vi.fn(() => '/tmp/vorn-native-server-test'),
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
      loadConfig: vi.fn(() => ({
        version: 1,
        defaults: { shell: '/bin/zsh', fontSize: 14, theme: 'dark' },
        projects: [],
        workflows: [],
        remoteHosts: [],
        tasks: [],
        workspaces: []
      })),
      saveConfig: vi.fn(),
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

/** A WebSocket client that sends one call at a time and returns its frame. */
class Client {
  private next = 1
  private constructor(private ws: WebSocket) {}

  static async open(port: number, bearer = true): Promise<Client> {
    const ws = new WebSocket(
      `ws://127.0.0.1:${port}/ws`,
      bearer ? { headers: { Authorization: `Bearer ${TEST_CREDENTIAL}` } } : {}
    )
    await new Promise<void>((resolve, reject) => {
      ws.once('open', resolve)
      ws.once('error', reject)
    })
    return new Client(ws)
  }

  call(method: string, params?: unknown): Promise<Record<string, unknown>> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timeout: ${method}`)), 20_000)
      const onMessage = (raw: WebSocket.RawData): void => {
        const frame = JSON.parse(raw.toString()) as Record<string, unknown>
        if (frame.id !== id) return
        this.ws.off('message', onMessage)
        clearTimeout(timer)
        resolve(frame)
      }
      this.ws.on('message', onMessage)
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  close(): void {
    this.ws.close()
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

type Counts = Record<
  string,
  {
    mode?: string
    forwarded?: number
    native?: number
    shadowMatched?: number
    shadowMismatched?: number
    shadowUnported?: number
  }
>

async function counts(v: Vornd): Promise<Counts> {
  const res = await fetch(`http://127.0.0.1:${v.port}/vornd/health`)
  return ((await res.json()) as { groups: Counts }).groups
}

function sh(cwd: string, ...args: string[]): string {
  return execFileSync('git', args, { cwd, encoding: 'utf-8', stdio: ['ignore', 'pipe', 'pipe'] })
}

function write(file: string, text: string | Buffer): void {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.writeFileSync(file, text)
}

/** Paths in one copy of the fixture. */
interface Fixture {
  root: string
  repo: string
  worktree: string
  plain: string
}

/**
 * A repository with a bare origin, three branches, one linked worktree, an
 * ignore file, hidden and mixed-case names, a binary file and uncommitted
 * changes of every kind.
 */
function makeFixture(): Fixture {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-server-')))
  const repo = path.join(root, 'repo')
  const plain = path.join(root, 'plain')
  fs.mkdirSync(repo)
  fs.mkdirSync(plain)
  write(path.join(plain, 'notes.txt'), 'not a repository\n')
  sh(repo, 'init', '-q', '-b', 'main')
  sh(repo, 'config', 'user.email', 'parity@vorn.invalid')
  sh(repo, 'config', 'user.name', 'parity')
  sh(repo, 'config', 'commit.gpgsign', 'false')
  write(path.join(repo, 'README.md'), '# fixture\n')
  write(path.join(repo, '.gitignore'), 'build/\n*.log\n')
  write(path.join(repo, 'src', 'a.ts'), 'export const a = 1\n'.repeat(20))
  write(path.join(repo, 'src', 'Beta.ts'), 'export const b = 2\n')
  write(path.join(repo, 'src', 'café.txt'), 'accent\n')
  write(path.join(repo, 'src', 'cafe.txt'), 'plain\n')
  write(path.join(repo, 'src', '_under.ts'), 'under\n')
  write(path.join(repo, 'src', '10-ten.md'), 'ten\n')
  write(path.join(repo, 'src', '2-two.md'), 'two\n')
  write(path.join(repo, 'Zeta', 'z.txt'), 'z\n')
  write(path.join(repo, 'alpha', 'a.txt'), 'a\n')
  write(path.join(repo, '.github', 'workflows', 'ci.yml'), 'on: push\n')
  write(path.join(repo, 'bin.dat'), Buffer.from([0, 1, 2, 3, 0, 255]))
  sh(repo, 'add', '-A')
  sh(repo, 'commit', '-q', '-m', 'base')
  sh(repo, 'branch', 'merged')
  sh(repo, 'checkout', '-q', '-b', 'feature/a')
  write(path.join(repo, 'src', 'feature.ts'), 'export const feature = true\n')
  sh(repo, 'add', '-A')
  sh(repo, 'commit', '-q', '-m', 'feature')
  sh(repo, 'checkout', '-q', 'main')
  sh(root, 'clone', '-q', '--bare', repo, path.join(root, 'origin.git'))
  sh(repo, 'remote', 'add', 'origin', path.join(root, 'origin.git'))
  sh(repo, 'fetch', '-q', 'origin')
  sh(repo, 'branch', '-q', '--set-upstream-to=origin/main', 'main')
  const worktree = path.join(root, '.vorn-worktrees', 'repo', 'existing')
  sh(repo, 'worktree', 'add', '-q', '-b', 'wt-branch', worktree)
  // Uncommitted: modified, deleted, staged new, untracked, ignored.
  write(path.join(repo, 'src', 'a.ts'), 'export const a = 2\n'.repeat(19))
  fs.rmSync(path.join(repo, 'README.md'))
  write(path.join(repo, 'src', 'staged.ts'), 'staged\n')
  sh(repo, 'add', 'src/staged.ts')
  write(path.join(repo, 'src', 'untracked.ts'), 'untracked\n')
  write(path.join(repo, 'build', 'out.js'), 'ignored\n')
  write(path.join(repo, 'debug.log'), 'ignored\n')
  write(path.join(repo, '.hidden'), 'hidden\n')
  write(path.join(worktree, 'dirty.txt'), 'dirty\n')
  return { root, repo, worktree, plain }
}

/**
 * The store file vornd reads: one project on a remote host, whose calls are
 * the server's, and one local project.
 */
function writeStore(file: string, localProject: string): void {
  const db = new Database(file)
  db.exec(`
    CREATE TABLE projects (path TEXT NOT NULL, host_ids TEXT);
    CREATE TABLE remote_hosts (id TEXT PRIMARY KEY);
    INSERT INTO remote_hosts (id) VALUES ('host-1');
  `)
  db.prepare('INSERT INTO projects (path, host_ids) VALUES (?, ?)').run(
    '/srv/remote-project',
    JSON.stringify(['host-1'])
  )
  db.prepare('INSERT INTO projects (path, host_ids) VALUES (?, ?)').run(localProject, null)
  db.close()
}

let serverPort: number
let closeServer: () => Promise<void>
let native: Vornd | undefined
let desktop: Vornd | undefined
let shadow: Vornd | undefined
let reads: Fixture
let mine: Fixture
let theirs: Fixture
let storeDir: string

describe.skipIf(!vornd)('the native server answers as the server does', () => {
  beforeAll(async () => {
    process.env.SECRET_VORN_BOOTSTRAP_TOKEN = TEST_CREDENTIAL
    const { startServer } = await import('../packages/server/src/index')
    const origWrite = process.stdout.write.bind(process.stdout)
    process.stdout.write = (() => true) as typeof process.stdout.write
    try {
      const { app, port } = await startServer({ port: 0 })
      serverPort = port
      closeServer = () => app.close()
    } finally {
      process.stdout.write = origWrite
    }
    reads = makeFixture()
    mine = makeFixture()
    theirs = makeFixture()
    storeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-server-store-'))
    const db = path.join(storeDir, 'vorn.db')
    writeStore(db, reads.repo)
    native = await startVornd(serverPort, ['--native-server', '--db', db])
    desktop = await startVornd(serverPort, ['--native-server', '--db', db], {
      VORND_DESKTOP_TOKEN: TEST_CREDENTIAL
    })
    shadow = await startVornd(serverPort, [
      '--groups',
      'git=shadow,file=shadow,ide=shadow',
      '--db',
      db
    ])
  }, 60_000)

  afterAll(async () => {
    delete process.env.SECRET_VORN_BOOTSTRAP_TOKEN
    await Promise.all([stopVornd(native), stopVornd(desktop), stopVornd(shadow)])
    await closeServer?.()
    for (const dir of [reads?.root, mine?.root, theirs?.root, storeDir]) {
      if (dir) fs.rmSync(dir, { recursive: true, force: true })
    }
  })

  /** Calls that change nothing, made on the one read fixture by both sides. */
  const readCalls = (): Array<[string, unknown]> => [
    ['git:isGitRepo', reads.repo],
    ['git:isGitRepo', reads.plain],
    ['git:isGitRepo', path.join(reads.root, 'missing')],
    ['git:isGitRepo', 'repo'],
    ['git:listBranches', reads.repo],
    ['git:listBranches', reads.plain],
    ['git:listRemoteBranches', reads.repo],
    ['git:listWorktrees', reads.repo],
    ['git:listWorktrees', reads.plain],
    ['git:getBranch', reads.repo],
    ['git:getBranch', reads.worktree],
    ['git:getBranch', reads.plain],
    ['git:getWorktreeBranch', reads.worktree],
    ['git:worktreeDirty', reads.repo],
    ['git:worktreeDirty', reads.worktree],
    ['git:diffStat', reads.repo],
    ['git:diffStat', reads.worktree],
    ['git:diffStat', reads.plain],
    ['git:diffFull', reads.repo],
    ['git:diffFull', reads.plain],
    ['git:diffFull', { cwd: reads.repo, from: 'merged', to: 'feature/a' }],
    ['git:diffFull', { cwd: reads.repo, from: 'nope', to: 'feature/a' }],
    ['git:diffFull', { cwd: reads.repo }],
    ['file:listDir', { dirPath: reads.repo }],
    ['file:listDir', { dirPath: path.join(reads.repo, 'src') }],
    ['file:listDir', { dirPath: reads.plain }],
    ['file:listDir', { dirPath: path.join(reads.root, 'missing') }],
    ['file:readContent', { filePath: path.join(reads.repo, 'src', 'a.ts') }],
    ['file:readContent', { filePath: path.join(reads.repo, 'src', 'a.ts'), maxBytes: 25 }],
    ['file:readContent', { filePath: path.join(reads.repo, 'bin.dat') }],
    ['file:readContent', { filePath: path.join(reads.repo, 'missing.txt') }],
    ['file:readContent', { filePath: path.join(reads.repo, 'src') }],
    ['file:stamp', { filePath: path.join(reads.repo, 'src', 'Beta.ts') }],
    ['file:stamp', { filePath: path.join(reads.repo, 'missing.txt') }],
    ['ide:detect', undefined]
  ]

  it('answers every call that changes nothing with the server’s frame', async () => {
    const direct = await Client.open(serverPort)
    const through = await Client.open(native!.port)
    // A socket that opened with the bearer header is admitted silently; the
    // first answer the server sends it shows vornd it was.
    await through.call('config:load')
    try {
      for (const [method, params] of readCalls()) {
        const want = answerOf(await direct.call(method, params))
        const got = answerOf(await through.call(method, params))
        expect(got, `${method} ${JSON.stringify(params)}`).toEqual(want)
      }
      const opened = answerOf(
        await through.call('ide:open', { ideId: 'no-such-editor', projectPath: reads.repo })
      )
      expect(opened).toEqual(
        answerOf(
          await direct.call('ide:open', { ideId: 'no-such-editor', projectPath: reads.repo })
        )
      )
      expect(opened).not.toHaveProperty('result')
    } finally {
      direct.close()
      through.close()
    }
    const groups = await counts(native!)
    for (const group of ['git', 'file', 'ide']) {
      expect(groups[group]?.mode).toBe('native')
      expect(groups[group]?.native ?? 0).toBeGreaterThan(0)
    }
  })

  it('makes the same changes the server makes, and answers them the same', async () => {
    /** One side: the client it calls through, its fixture copy, and what it made up. */
    interface Side {
      client: Client
      f: Fixture
      made: string[]
      worktrees: string[]
    }
    const server: Side = { client: await Client.open(serverPort), f: mine, made: [], worktrees: [] }
    const through: Side = {
      client: await Client.open(native!.port),
      f: theirs,
      made: [],
      worktrees: []
    }
    await through.client.call('config:load')

    /** Makes one call on each side and compares the answers. */
    const step = async (
      method: string,
      params: (s: Side) => unknown,
      extra: (a: Answer) => Answer = (a) => a
    ): Promise<void> => {
      const answers: Answer[] = []
      for (const side of [server, through]) {
        const p = params(side)
        const answer = answerOf(await side.client.call(method, p))
        if (method === 'git:createWorktree') {
          side.made.push(...madeUpBy(answer, (p as { worktreeName?: string }).worktreeName))
          const made = (answer.result as { worktreePath?: string } | undefined)?.worktreePath
          if (made) side.worktrees.push(made)
        }
        answers.push(extra(worktreeMadeUp(fixtureRoot(answer, side.f.root), side.made)))
      }
      expect(answers[1], `${method} ${JSON.stringify(params(server))}`).toEqual(answers[0])
    }

    try {
      await step('git:commit', (s) => ({ cwd: s.f.repo, message: 'one', includeUnstaged: false }))
      await step('git:commit', (s) => ({ cwd: s.f.repo, message: 'two', includeUnstaged: true }))
      await step('git:commit', (s) => ({ cwd: s.f.repo, message: 'none', includeUnstaged: true }))
      await step('git:push', (s) => s.f.repo)
      await step('git:push', (s) => s.f.plain)
      await step('git:createWorktree', (s) => ({
        projectPath: s.f.repo,
        branch: 'made',
        worktreeName: 'named'
      }))
      await step('git:createWorktree', (s) => ({ projectPath: s.f.repo, branch: 'feature/a' }))
      // A name already a branch, on a branch checked out elsewhere: the new
      // branch takes the directory's id.
      await step('git:createWorktree', (s) => ({
        projectPath: s.f.repo,
        branch: 'main',
        worktreeName: 'merged'
      }))
      await step('git:createWorktree', (s) => ({
        projectPath: s.f.repo,
        branch: '-bad name',
        worktreeName: 'bad'
      }))
      await step('git:listWorktrees', (s) => s.f.repo, linkedWorktreeOrder)
      await step('git:listBranches', (s) => s.f.repo)
      await step('git:worktreeDirty', (s) => s.worktrees[0])
      // Moves the server's sessions, so it is the server's to answer.
      await step('git:checkoutBranch', (s) => ({ cwd: s.worktrees[0], branch: 'merged' }))
      await step('git:removeWorktree', (s) => ({
        projectPath: s.f.repo,
        worktreePath: s.worktrees[0],
        force: false,
        deleteBranch: true
      }))
      await step('git:removeWorktree', (s) => ({
        projectPath: s.f.repo,
        worktreePath: s.f.worktree,
        force: false,
        deleteBranch: false
      }))
      await step('git:removeWorktree', (s) => ({
        projectPath: s.f.repo,
        worktreePath: s.f.worktree,
        force: true,
        deleteBranch: true
      }))
      await step('git:deleteBranches', (s) => ({
        projectPath: s.f.repo,
        branches: ['made', 'feature/a', 'no-such-branch']
      }))
      await step('git:deleteBranches', (s) => ({
        projectPath: s.f.repo,
        branches: ['wt-branch'],
        force: true
      }))
      await step('git:listWorktrees', (s) => s.f.repo, linkedWorktreeOrder)
      await step('git:listBranches', (s) => s.f.repo)
      await step('file:writeContent', (s) => ({
        filePath: path.join(s.f.repo, 'written.txt'),
        content: 'héllo\n'
      }))
      await step('file:stamp', (s) => ({ filePath: path.join(s.f.repo, 'written.txt') }), fileMtime)
      await step('file:readContent', (s) => ({ filePath: path.join(s.f.repo, 'written.txt') }))
      await step('file:writeContent', (s) => ({
        filePath: path.join(s.f.repo, 'no', 'such', 'dir.txt'),
        content: 'x'
      }))
      await step('file:writeContent', (s) => ({ filePath: s.f.repo, content: 'x' }))
    } finally {
      server.client.close()
      through.client.close()
    }
    expect(sh(theirs.repo, 'log', '--format=%s')).toBe(sh(mine.repo, 'log', '--format=%s'))
    expect(sh(path.join(theirs.root, 'origin.git'), 'log', '--format=%s', 'main')).toBe(
      sh(path.join(mine.root, 'origin.git'), 'log', '--format=%s', 'main')
    )
  })

  it('leaves the calls only the server can answer to the server', async () => {
    const through = await Client.open(native!.port)
    await through.call('config:load')
    const before = await counts(native!)
    // A project on a remote host, a file on one, and a call that moves the
    // server's sessions to another branch.
    await through.call('git:listBranches', '/srv/remote-project')
    await through.call('git:diffStat', '/srv/remote-project/sub')
    await through.call('file:listDir', { dirPath: reads.repo, remoteHostId: 'host-1' })
    await through.call('git:renameWorktreeBranch', {
      worktreePath: path.join(reads.root, 'missing'),
      newBranch: 'x'
    })
    through.close()
    const after = await counts(native!)
    expect((after.git?.forwarded ?? 0) - (before.git?.forwarded ?? 0)).toBe(3)
    expect(after.git?.native ?? 0).toBe(before.git?.native ?? 0)
    expect((after.file?.forwarded ?? 0) - (before.file?.forwarded ?? 0)).toBe(1)
  })

  it('answers only once the server has admitted the socket', async () => {
    const through = await Client.open(native!.port, false)
    let before = await counts(native!)
    const refused = await through.call('git:getBranch', reads.repo)
    expect(refused.error).toBeDefined()
    let after = await counts(native!)
    expect((after.git?.forwarded ?? 0) - (before.git?.forwarded ?? 0)).toBe(1)
    through.close()

    const authed = await Client.open(native!.port, false)
    await authed.call('auth:authenticate', { token: TEST_CREDENTIAL })
    before = await counts(native!)
    const answered = await authed.call('git:getBranch', reads.repo)
    expect(answered.result).toBe('main')
    after = await counts(native!)
    expect((after.git?.native ?? 0) - (before.git?.native ?? 0)).toBe(1)
    authed.close()
  })

  it('answers the desktop’s socket from its first call', async () => {
    const through = await Client.open(desktop!.port)
    const before = await counts(desktop!)
    const answered = await through.call('git:getBranch', reads.repo)
    expect(answered.result).toBe('main')
    const after = await counts(desktop!)
    expect((after.git?.native ?? 0) - (before.git?.native ?? 0)).toBe(1)
    through.close()
  })

  it('shadows every call that changes nothing and finds no difference', async () => {
    const direct = await Client.open(serverPort)
    const through = await Client.open(shadow!.port)
    await through.call('config:load')
    try {
      for (const [method, params] of readCalls()) {
        const want = answerOf(await direct.call(method, params))
        const got = answerOf(await through.call(method, params))
        expect(got, `${method} ${JSON.stringify(params)}`).toEqual(want)
      }
    } finally {
      direct.close()
      through.close()
    }
    // Shadow answers settle after the server's; wait for the counts to stop moving.
    let groups = await counts(shadow!)
    for (let tries = 0; tries < 50; tries++) {
      await new Promise((r) => setTimeout(r, 100))
      const next = await counts(shadow!)
      const settled = JSON.stringify(next) === JSON.stringify(groups)
      groups = next
      if (settled && tries > 2) break
    }
    for (const group of ['git', 'file', 'ide']) {
      expect(groups[group]?.mode).toBe('shadow')
      expect(groups[group]?.native ?? 0).toBe(0)
      expect(groups[group]?.shadowMismatched ?? 0).toBe(0)
      expect(groups[group]?.shadowMatched ?? 0).toBeGreaterThan(0)
    }
  })
})
