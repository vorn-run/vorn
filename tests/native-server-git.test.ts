/**
 * vornd's own answers to the `git:`, `file:` and `ide:` calls, against the
 * answers the server's TypeScript gave to the same calls, recorded before it
 * was removed (`fixtures/js-reference/git-calls.json`, rerecorded with
 * `VORN_RECORD_JS_REFERENCE=1` against a server that still has them).
 *
 * vornd is started as the server, on a data directory and home of its own.
 * The frames a client receives must equal the recorded ones but for the
 * differences `helpers/git-parity` names. A call that changes a repository
 * is made on a copy of the fixture of its own.
 *
 * Runs where vornd has been built (`yarn build:core`, or the binary in
 * `VORN_CONFORMANCE_VORND`).
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import WebSocket from 'ws'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { JsReference, posixSeparators } from './helpers/js-reference'
import {
  answerOf,
  fileMtime,
  fixtureRoot,
  linkedWorktreeOrder,
  madeUpBy,
  worktreeMadeUp,
  type Answer
} from './helpers/git-parity'
import { startServed, type Served } from './helpers/served'

const TEST_CREDENTIAL = 'native-server-test-credential'
const EXE = process.platform === 'win32' ? '.exe' : ''

const vornd = [
  process.env.VORN_CONFORMANCE_VORND,
  path.resolve(__dirname, `../packages/core/vornd${EXE}`),
  path.resolve(__dirname, `../packages/core/target/release/vornd${EXE}`)
].find((p): p is string => !!p && fs.existsSync(p))

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

async function counts(v: Served): Promise<Counts> {
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

/** Projects as the app keeps them: one on this machine, one on a host the store does not have. */
async function saveProjects(port: number, localProject: string): Promise<void> {
  const client = await Client.open(port)
  try {
    const config = (await client.call('config:load')).result as Record<string, unknown>
    await client.call('config:save', {
      ...config,
      projects: [
        { name: 'remote-project', path: '/srv/remote-project', hostIds: ['host-1'] },
        { name: 'repo', path: localProject }
      ]
    })
  } finally {
    client.close()
  }
}

let native: Served | undefined
const reference = new JsReference('git-calls')
let reads: Fixture
let theirs: Fixture
let storeDir: string

/** The TypeScript that answered these is gone: only its recorded answers remain. */
const gone = async (): Promise<never> => {
  throw new Error('nothing records these now: the TypeScript that answered them is gone')
}

describe.skipIf(!vornd)('the native server answers as the server does', () => {
  beforeAll(async () => {
    reads = makeFixture()
    theirs = makeFixture()
    storeDir = fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-native-server-store-'))
    native = await startServed({ dataDir: storeDir, credential: TEST_CREDENTIAL })
    await saveProjects(native.port, reads.repo)
  }, 60_000)

  afterAll(async () => {
    await native?.stop()
    for (const dir of [reads?.root, theirs?.root, storeDir, native?.home]) {
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
    const through = await Client.open(native!.port)
    // A socket that opened with the bearer header is admitted silently; the
    // first answer the server sends it shows vornd it was.
    await through.call('config:load')
    try {
      for (const [method, params] of readCalls()) {
        const read = (a: Answer): Answer => {
          const rooted = posixSeparators(fixtureRoot(a, reads.root))
          return method === 'file:stamp' ? fileMtime(rooted) : rooted
        }
        const key = `${method} ${JSON.stringify(posixSeparators(fixtureRoot(params ?? null, reads.root)))}`
        const got = read(answerOf(await through.call(method, params)))
        // The editors installed are this machine's.
        if (method === 'ide:detect') {
          expect(Array.isArray(got.result), `${key}`).toBe(true)
          continue
        }
        const want = await reference.want(key, gone)
        expect(got, `${key}`).toEqual(want)
      }
      const open = { ideId: 'no-such-editor', projectPath: reads.repo }
      const opened = answerOf(await through.call('ide:open', open))
      expect(opened).toEqual(await reference.want('ide:open no-such-editor', gone))
      expect(opened).not.toHaveProperty('result')
    } finally {
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
    const through: Side = {
      client: await Client.open(native!.port),
      f: theirs,
      made: [],
      worktrees: []
    }
    await through.client.call('config:load')

    /** Makes one call on each side and compares the answers. */
    let steps = 0
    const step = async (
      method: string,
      params: (s: Side) => unknown,
      extra: (a: Answer) => Answer = (a) => a
    ): Promise<void> => {
      const run = async (side: Side): Promise<Answer> => {
        const p = params(side)
        const answer = answerOf(await side.client.call(method, p))
        if (method === 'git:createWorktree') {
          side.made.push(...madeUpBy(answer, (p as { worktreeName?: string }).worktreeName))
          const made = (answer.result as { worktreePath?: string } | undefined)?.worktreePath
          if (made) side.worktrees.push(made)
        }
        return extra(posixSeparators(worktreeMadeUp(fixtureRoot(answer, side.f.root), side.made)))
      }
      const key = `change ${++steps} ${method}`
      const want = await reference.want(key, gone)
      expect(await run(through), `${key}`).toEqual(want)
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
      // Drops the server's cached size of the worktree, so the server answers it too.
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
      through.client.close()
    }
    expect(sh(theirs.repo, 'log', '--format=%s')).toBe(await reference.want('change log', gone))
    expect(sh(path.join(theirs.root, 'origin.git'), 'log', '--format=%s', 'main')).toBe(
      await reference.want('change pushed', gone)
    )
  })

  it('answers a remote host it cannot read as this machine, and forwards nothing', async () => {
    const through = await Client.open(native!.port)
    await through.call('config:load')
    // A host the store does not have is no host: the server read it as this machine too.
    const listed = await through.call('file:listDir', {
      dirPath: reads.repo,
      remoteHostId: 'host-1'
    })
    expect(Array.isArray(listed.result)).toBe(true)
    const renamed = await through.call('git:renameWorktreeBranch', {
      worktreePath: path.join(reads.root, 'missing'),
      newBranch: 'x'
    })
    expect(renamed.result).toBe(false)
    through.close()
    const after = await counts(native!)
    for (const group of ['git', 'file', 'ide']) expect(after[group]?.forwarded ?? 0).toBe(0)
  })

  it('answers only once the socket is admitted', async () => {
    const through = await Client.open(native!.port, false)
    const refused = await through.call('git:getBranch', reads.repo)
    expect((refused.error as { code?: number } | undefined)?.code).toBe(-32001)
    through.close()

    const authed = await Client.open(native!.port, false)
    await authed.call('auth:authenticate', { token: TEST_CREDENTIAL })
    const before = await counts(native!)
    const answered = await authed.call('git:getBranch', reads.repo)
    expect(answered.result).toBe('main')
    const after = await counts(native!)
    expect((after.git?.native ?? 0) - (before.git?.native ?? 0)).toBe(1)
    authed.close()
  })

  it('answers the desktop’s socket from its first call', async () => {
    const through = await Client.open(native!.port)
    const before = await counts(native!)
    const answered = await through.call('git:getBranch', reads.repo)
    expect(answered.result).toBe('main')
    const after = await counts(native!)
    expect((after.git?.native ?? 0) - (before.git?.native ?? 0)).toBe(1)
    through.close()
  })
})
