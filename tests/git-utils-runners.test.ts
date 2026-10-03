/**
 * The git functions against a real repository, on both runners.
 *
 * `git-native-parity.test.ts` compares against the built core and only runs
 * where it has been built. This one stands a plain async child process in for
 * `gitRun`, so the native branches in `git-utils` run on every machine: what
 * is checked here is that each function reads its answer the same way whichever
 * runner produced it.
 */
import { execFile, execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { RemoteHost } from '../packages/shared/src/types'
import * as git from '../packages/server/src/git-utils'
import { nativeRunner, resetGitRunner, type GitRunner } from '../packages/server/src/git-runner'
import type { NativeGitRequest } from '../packages/server/src/native-core'

function sh(cwd: string, ...args: string[]): string {
  return execFileSync('git', args, { cwd, encoding: 'utf-8', stdio: ['ignore', 'pipe', 'pipe'] })
}

/** `gitRun`'s contract, met by an async child process instead of the core. */
const stubGitRun = (req: NativeGitRequest): Promise<string> =>
  new Promise((resolve, reject) =>
    execFile(
      req.bin,
      req.args,
      {
        cwd: req.cwd,
        env: req.env,
        timeout: req.timeoutMs,
        maxBuffer: req.maxBuffer,
        encoding: 'utf-8'
      },
      (err, stdout) => (err ? reject(err) : resolve(stdout))
    )
  )

let root: string
let origin: string
let repo: string

beforeAll(() => {
  root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-git-runners-')))
  origin = path.join(root, 'origin.git')
  repo = path.join(root, 'repo')
  fs.mkdirSync(repo)
  sh(root, 'init', '-q', '--bare', '-b', 'main', origin)
  sh(repo, 'init', '-q', '-b', 'main')
  sh(repo, 'config', 'user.email', 'runners@vorn.invalid')
  sh(repo, 'config', 'user.name', 'runners')
  sh(repo, 'config', 'commit.gpgsign', 'false')
  sh(repo, 'remote', 'add', 'origin', origin)
  fs.writeFileSync(path.join(repo, 'a.txt'), 'one\ntwo\n')
  sh(repo, 'add', '-A')
  sh(repo, 'commit', '-q', '-m', 'base')
  sh(repo, 'push', '-q', '-u', 'origin', 'main')
  sh(repo, 'branch', 'side')
})

afterEach(() => {
  delete process.env.VORN_GIT
  resetGitRunner()
})

afterAll(() => {
  if (root) fs.rmSync(root, { recursive: true, force: true })
})

describe.each([
  ['js', () => (process.env.VORN_GIT = 'js')],
  [
    'native',
    () => {
      process.env.VORN_GIT = 'native'
      resetGitRunner(nativeRunner(stubGitRun))
    }
  ]
])('git-utils on the %s runner', (_mode, use) => {
  it('reads HEAD, branches and the remote', async () => {
    use()
    expect(await git.getGitBranch(repo)).toBe('main')
    expect(await git.getGitBranchAsync(repo)).toBe('main')
    const head = sh(repo, 'rev-parse', 'HEAD').trim()
    expect(await git.getGitHead(repo)).toBe(head)
    expect(await git.getGitHeadAsync(repo)).toBe(head)
    expect(await git.getGitHeadAsync(path.join(root, 'nowhere'))).toBeNull()
    expect(await git.listRemoteBranches(repo)).toEqual(['main'])
    expect(await git.getBranchUpstream(repo, 'main')).toBe('origin/main')
    expect(await git.getBranchUpstream(repo, 'side')).toBeNull()
    expect(await git.getLastCommitDate(repo)).toMatch(/^\d{4}-\d{2}-\d{2}/)
    expect(await git.remoteHostOf(repo)).toBeNull()
  })

  it('reads changes, then commits and pushes them', async () => {
    use()
    // Each runner starts from the other's commit, so each writes its own lines.
    const before = fs.readFileSync(path.join(repo, 'a.txt'), 'utf-8').split('\n')
    fs.writeFileSync(path.join(repo, 'a.txt'), [before[0], _mode, 'more', ''].join('\n'))
    expect(await git.getGitStatusPorcelain(repo)).toContain('a.txt')
    expect(await git.getGitDiffText(repo)).toContain(`+${_mode}`)
    const full = await git.getGitDiffFull(repo)
    expect(full?.files.map((f) => [f.filePath, f.status])).toEqual([['a.txt', 'modified']])
    expect(full?.files[0].insertions).toBeGreaterThan(0)
    expect(await git.gitCommit(repo, `change on ${_mode}`, true)).toEqual({ success: true })
    expect(await git.gitPush(repo)).toEqual({ success: true })
    expect(sh(origin, 'log', '-1', '--format=%s', 'main').trim()).toBe(`change on ${_mode}`)
  })

  it('checks out a branch and back', async () => {
    use()
    expect(await git.checkoutBranch(repo, 'side')).toEqual({ ok: true })
    expect(await git.getGitBranch(repo)).toBe('side')
    expect(await git.checkoutBranch(repo, 'main')).toEqual({ ok: true })
  })
})

describe('git-utils on a remote host', () => {
  const host: RemoteHost = {
    id: 'h',
    label: 'box',
    hostname: 'box.example',
    user: 'dev',
    port: 22
  } as RemoteHost

  it('sends each command over the runner, with cwd and arguments escaped', async () => {
    const sent: string[] = []
    const runner: GitRunner = {
      mode: 'native',
      local: vi.fn(async () => {
        throw new Error('no local git on a remote session')
      }),
      remote: vi.fn(async (_host, command: string) => {
        sent.push(command)
        return command.includes('branch --list') ? '  main\n' : ''
      })
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)

    await git.getGitStatusPorcelain('/srv/my app', host)
    const made = await git.createWorktree('/srv/app', 'main', 'next', host)

    expect(sent[0]).toBe("cd '/srv/my app' && git status --porcelain")
    expect(sent[1]).toMatch(/^mkdir -p '?\/srv\/\.vorn-worktrees\/app'?$/)
    expect(sent.some((c) => c.includes('git worktree add'))).toBe(true)
    expect(made.worktreePath.startsWith('/srv/.vorn-worktrees/app/next-')).toBe(true)
    expect(runner.local).not.toHaveBeenCalled()
  })
})

describe('changes to one repository', () => {
  it('take turns, so one commit never stages between another add and commit', async () => {
    const log: string[] = []
    const runner: GitRunner = {
      mode: 'native',
      // Every command yields, as native git does, so unserialized calls would interleave.
      local: async (args, cwd) => {
        log.push(`${cwd} ${args[0]}`)
        await new Promise((r) => setTimeout(r, 5))
        return ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)

    const results = await Promise.all([
      git.gitCommit('/repo', 'one', true),
      git.gitCommit('/repo', 'two', true),
      git.gitCommit('/elsewhere', 'three', false)
    ])

    expect(results.every((r) => r.success)).toBe(true)
    expect(log.filter((c) => c.startsWith('/repo '))).toEqual([
      '/repo add',
      '/repo commit',
      '/repo add',
      '/repo commit'
    ])
    // Another repository does not wait its turn behind these.
    expect(log.indexOf('/elsewhere commit')).toBeLessThan(log.indexOf('/repo commit'))
  })

  it('carries on after a change that failed', async () => {
    let calls = 0
    const runner: GitRunner = {
      mode: 'native',
      local: async () => {
        if (calls++ === 0) throw new Error('Command failed: git commit')
        return ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)
    const [first, second] = await Promise.all([
      git.gitCommit('/repo', 'one', false),
      git.gitCommit('/repo', 'two', false)
    ])
    expect(first.success).toBe(false)
    expect(second).toEqual({ success: true })
  })
})
