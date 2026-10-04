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
import type { ProjectConfig, RemoteHost } from '../packages/shared/src/types'
import * as git from '../packages/server/src/git-utils'
import { nativeRunner, resetGitRunner, type GitRunner } from '../packages/server/src/git-runner'
import type { NativeGitRequest } from '../packages/server/src/native-core'
import {
  pruneOrphanDirs,
  reclaimArtifacts,
  removeWorktrees
} from '../packages/server/src/worktree-inventory'

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
    // Some gits also list origin/HEAD, which shortens to `origin`.
    expect(await git.listRemoteBranches(repo)).toContain('main')
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

  it('includes a checkout, so a commit stays on the branch it started on', async () => {
    const log: string[] = []
    const runner: GitRunner = {
      mode: 'native',
      local: async (args, cwd) => {
        log.push(`${cwd} ${args[0]}`)
        await new Promise((r) => setTimeout(r, 5))
        return ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)

    await Promise.all([
      git.gitCommit('/repo', 'one', true),
      git.checkoutBranch('/repo', 'side'),
      git.deleteBranches('/repo', ['old'])
    ])
    expect(log).toEqual(['/repo add', '/repo commit', '/repo checkout', '/repo branch'])
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

describe('turns are per repository, not per path', () => {
  it('makes a commit in a linked worktree and its removal from the project take turns', async () => {
    const project = path.join(root, 'shared')
    fs.mkdirSync(project)
    sh(project, 'init', '-q', '-b', 'main')
    sh(project, 'config', 'user.email', 'runners@vorn.invalid')
    sh(project, 'config', 'user.name', 'runners')
    sh(project, 'commit', '-q', '--allow-empty', '-m', 'base')
    const linked = path.join(root, '.vorn-worktrees', 'shared', 'wt')
    sh(project, 'worktree', 'add', '-q', '-b', 'wt', linked)

    const log: string[] = []
    const runner: GitRunner = {
      mode: 'native',
      local: async (args) => {
        log.push(args.slice(0, 2).join(' '))
        await new Promise((r) => setTimeout(r, 5))
        return ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)

    await Promise.all([
      git.gitCommit(linked, 'work', true),
      git.removeWorktree(project, linked, true)
    ])
    // The commit's add and commit run back to back; the removal comes after.
    expect(log.slice(0, 2)).toEqual(['add -A', 'commit -m'])
    expect(log.slice(2).some((c) => c.startsWith('worktree remove'))).toBe(true)
  })
})

describe('a worktree being made or removed', () => {
  it('names its path before git makes anything there', async () => {
    const seen: string[] = []
    const runner: GitRunner = {
      mode: 'native',
      local: async (args) => {
        seen.push(args.slice(0, 2).join(' '))
        return args[0] === 'branch' ? '  main\n' : ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)
    const made = await git.createWorktree(repo, 'main', 'held', undefined, (p) =>
      seen.push(`path ${p}`)
    )
    expect(seen[0]).toBe(`path ${made.worktreePath}`)
    expect(seen.some((c) => c.startsWith('worktree add'))).toBe(true)
  })

  it('checks once it has the turn, so a session started while it waited is spared', async () => {
    const log: string[] = []
    // A session opens in the worktree while the commit ahead of the removal runs.
    let busy = false
    const runner: GitRunner = {
      mode: 'native',
      local: async (args) => {
        log.push(args.slice(0, 2).join(' '))
        await new Promise((r) => setTimeout(r, 5))
        if (args[0] === 'commit') busy = true
        return ''
      },
      remote: async () => ''
    }
    process.env.VORN_GIT = 'native'
    resetGitRunner(runner)
    const commit = git.gitCommit('/repo', 'work', false)
    const removal = git.removeWorktree('/repo', '/wt', false, undefined, false, () => {
      if (busy) throw new Error('has a session — close it first')
    })
    await commit
    await expect(removal).rejects.toThrow(/has a session/)
    expect(log.some((c) => c.startsWith('worktree remove'))).toBe(false)
  })
})

describe('worktree actions re-check for sessions just before deleting', () => {
  const busy = (): void => {
    throw new Error('has a session starting — close it first')
  }

  it('leaves a worktree that became busy while git ran', async () => {
    const wt = path.join(root, '.vorn-worktrees', 'repo', 'busy')
    sh(repo, 'worktree', 'add', '-q', '-b', 'busy', wt)
    const projects = [{ name: 'repo', path: repo }] as ProjectConfig[]

    const removed = await removeWorktrees(
      [{ projectPath: repo, worktreePath: wt }],
      () => 0,
      projects,
      () => undefined,
      busy
    )
    expect(removed.failed[0].error).toMatch(/session starting/)
    expect(fs.existsSync(wt)).toBe(true)

    fs.mkdirSync(path.join(wt, 'node_modules'))
    const reclaimed = await reclaimArtifacts(
      [wt],
      ['node_modules'],
      projects,
      () => undefined,
      busy
    )
    expect(reclaimed.failed[0].error).toMatch(/session starting/)
    expect(fs.existsSync(path.join(wt, 'node_modules'))).toBe(true)
  })

  it('leaves an orphan directory that became busy', async () => {
    const orphan = path.join(root, '.vorn-worktrees', 'repo', 'orphan')
    fs.mkdirSync(orphan, { recursive: true })
    const pruned = await pruneOrphanDirs(
      [orphan],
      () => 0,
      () => undefined,
      busy
    )
    expect(pruned.failed[0].error).toMatch(/session starting/)
    expect(fs.existsSync(orphan)).toBe(true)
  })
})
