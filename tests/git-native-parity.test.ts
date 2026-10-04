/**
 * The server's git functions, run on both paths against the same repositories,
 * must give the same answers. The child process path is the reference: it is
 * what a server without the core uses, and the native one is only allowed to
 * be faster.
 *
 * Runs only where `yarn build:core` has produced a binary with `gitRun`, as the
 * Core CI job does.
 */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest'
import * as git from '../packages/server/src/git-utils'
import {
  nativeRunner,
  processRunner,
  resetGitRunner,
  type GitRunner
} from '../packages/server/src/git-runner'
import { loadNativeCore } from '../packages/server/src/native-core'

const builtCore = path.resolve(__dirname, '../packages/core/vorn_core.node')
const gitRun = fs.existsSync(builtCore) ? loadNativeCore([builtCore]).gitRun : undefined
const hasGitRun = typeof gitRun === 'function'
const native: GitRunner | null = gitRun ? nativeRunner(gitRun) : null

function sh(cwd: string, ...args: string[]): string {
  return execFileSync('git', args, { cwd, encoding: 'utf-8', stdio: ['ignore', 'pipe', 'pipe'] })
}

/** Both answers, the child process path's first. */
async function both<T>(call: () => Promise<T>): Promise<[T, T]> {
  resetGitRunner(processRunner)
  const reference = await call()
  resetGitRunner(native)
  const fromCore = await call()
  return [reference, fromCore]
}

async function same<T>(call: () => Promise<T>): Promise<T> {
  const [reference, fromCore] = await both(call)
  expect(fromCore).toEqual(reference)
  return reference
}

/**
 * The error each path rejects with, as a caller would read it. The core words
 * a failure to start git as `execFileSync` did ("spawnSync git ENOENT") and a
 * child process as `execFile` does ("spawn git ENOENT"); that prefix is the
 * one difference allowed.
 */
async function sameFailure(call: () => Promise<unknown>): Promise<string> {
  const [reference, fromCore] = await both(() =>
    call().then(
      () => 'resolved',
      (err: Error) => err.message.replace(/^spawnSync /, 'spawn ')
    )
  )
  expect(fromCore).toBe(reference)
  return reference
}

let root: string
let repo: string
let worktree: string
let plain: string

beforeAll(() => {
  if (!hasGitRun) return
  root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-git-parity-')))
  repo = path.join(root, 'repo')
  plain = path.join(root, 'plain')
  fs.mkdirSync(path.join(repo, 'src', 'deep'), { recursive: true })
  fs.mkdirSync(plain)
  sh(repo, 'init', '-q', '-b', 'main')
  sh(repo, 'config', 'user.email', 'parity@vorn.invalid')
  sh(repo, 'config', 'user.name', 'parity')
  sh(repo, 'config', 'commit.gpgsign', 'false')
  sh(repo, 'remote', 'add', 'origin', 'git@github.com:vorn-run/vorn.git')
  for (let i = 0; i < 20; i++) {
    fs.writeFileSync(path.join(repo, 'src', `f${i}.ts`), `export const v${i} = ${i}\n`.repeat(50))
  }
  fs.writeFileSync(path.join(repo, 'src', 'deep', 'bin.dat'), Buffer.from([0, 1, 2, 3, 0, 255]))
  sh(repo, 'add', '-A')
  sh(repo, 'commit', '-q', '-m', 'base')
  sh(repo, 'branch', 'feature/a')
  sh(repo, 'branch', 'merged-one')
  worktree = path.join(root, 'wt')
  sh(repo, 'worktree', 'add', '-q', '-b', 'gilded-fresco', worktree)
  // A working tree with every kind of change the diff panel shows.
  fs.writeFileSync(path.join(repo, 'src', 'f1.ts'), 'changed\n')
  fs.rmSync(path.join(repo, 'src', 'f2.ts'))
  fs.writeFileSync(path.join(repo, 'src', 'new.ts'), 'new\n')
  sh(repo, 'add', 'src/new.ts')
  fs.writeFileSync(path.join(repo, 'untracked.txt'), 'u\n')
  fs.writeFileSync(path.join(repo, 'src', 'deep', 'bin.dat'), Buffer.from([0, 9, 9, 0]))
})

afterEach(() => {
  resetGitRunner()
})

afterAll(() => {
  if (root) fs.rmSync(root, { recursive: true, force: true })
})

describe.runIf(hasGitRun)('git on the native path answers as a child process does', () => {
  it('reads where a path is', async () => {
    for (const cwd of [() => repo, () => path.join(repo, 'src', 'deep'), () => worktree]) {
      expect(await same(() => git.isGitRepo(cwd()))).toBe(true)
      await same(() => git.getRepoRoot(cwd()))
      await same(() => git.getAbsoluteGitDir(cwd()))
    }
    expect(await same(() => git.isGitRepo(plain))).toBe(false)
  })

  it('reads branches and HEAD', async () => {
    expect(await same(() => git.getGitBranch(repo))).toBe('main')
    expect(await same(() => git.getGitBranch(worktree))).toBe('gilded-fresco')
    expect(await same(() => git.getGitHead(repo))).toMatch(/^[0-9a-f]{40}$/)
    expect(await same(() => git.listBranches(repo))).toContain('feature/a')
    await same(() => git.gitForEachRef(repo))
    await same(() => git.listMergedBranches(repo, 'main'))
    expect(await same(() => git.getDefaultBranch(repo))).toBe('main')
    await same(() => git.getLastCommitDate(repo))
    expect(await same(() => git.getGitBranchAsync(worktree))).toBe('gilded-fresco')
    await same(() => git.getGitHeadAsync(worktree))
  })

  it('reads the remote', async () => {
    expect(await same(() => git.detectRepoSlug(repo))).toEqual({ owner: 'vorn-run', repo: 'vorn' })
    expect(await same(() => git.remoteHostOf(repo))).toBe('github.com')
  })

  it('reads worktrees, status and diffs', async () => {
    expect((await same(() => git.listWorktrees(repo))).length).toBe(2)
    expect(await same(() => git.getGitStatusPorcelain(repo))).toContain('?? untracked.txt')
    expect(await same(() => git.isWorktreeDirty(repo))).toBe(true)
    expect(await same(() => git.isWorktreeDirty(worktree))).toBe(false)
    await same(() => git.getGitDiffText(repo))
    expect((await same(() => git.getGitDiffStat(repo)))?.filesChanged).toBe(4)
    const full = await same(() => git.getGitDiffFull(repo))
    expect(full?.files.map((f) => f.status).sort()).toEqual([
      'added',
      'deleted',
      'modified',
      'modified'
    ])
    const head = sh(repo, 'rev-parse', 'HEAD').trim()
    await same(() => git.getGitDiffFull(repo, undefined, { from: `${head}~0`, to: head }))
  })

  it('fails as the child process fails', async () => {
    const message = await sameFailure(() =>
      git.getGitStatusPorcelain(path.join(root, 'does-not-exist'))
    )
    expect(message).not.toBe('resolved')
    expect(await same(() => git.checkoutBranch(repo, 'no-such-branch'))).toMatchObject({
      ok: false
    })
    expect(await same(() => git.getGitHead(plain))).toBeNull()
  })

  it('makes the same changes', async () => {
    // Each path commits into a copy of its own, so both start from the same tree.
    const copies = ['process', 'native'].map((mode) => {
      const dir = path.join(root, `commit-${mode}`)
      fs.cpSync(repo, dir, { recursive: true })
      return dir
    })
    resetGitRunner(processRunner)
    const reference = await git.gitCommit(copies[0], 'parity', true)
    resetGitRunner(native)
    const fromCore = await git.gitCommit(copies[1], 'parity', true)
    expect(fromCore).toEqual(reference)
    expect(reference).toEqual({ success: true })
    expect(sh(copies[1], 'status', '--porcelain')).toBe('')
  })
})
