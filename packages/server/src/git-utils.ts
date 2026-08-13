import { execFileSync } from 'node:child_process'
import path from 'node:path'
import fs from 'node:fs'
import crypto from 'node:crypto'
import type { RemoteHost } from '@vornrun/shared/types'
import { sshExecSync, shellEscape, getSafeEnv } from './process-utils'
import { resolveExecutable } from './resolve-executable'

// Resolve `git` from the login-shell PATH so packaged Electron finds the
// same binary the user would from their terminal (e.g. a newer Homebrew git
// rather than the Xcode stub). Falls back to the bare name so callers still
// work if resolution fails.
function gitBin(): string {
  return resolveExecutable('git') ?? 'git'
}

/**
 * Run a git command locally or via SSH depending on whether a remote host is provided.
 * For remote: `cd <cwd> && git <args>`
 */
function gitExec(
  args: string[],
  cwd: string,
  opts?: { timeout?: number; maxBuffer?: number; remote?: RemoteHost }
): string {
  if (opts?.remote) {
    const cmd = `cd ${shellEscape(cwd, 'posix')} && git ${args.map((a) => shellEscape(a, 'posix')).join(' ')}`
    return sshExecSync(opts.remote, cmd, { timeout: opts?.timeout ?? 10000 })
  }
  return execFileSync(gitBin(), args, {
    cwd,
    ...EXEC_OPTS,
    env: getSafeEnv(),
    timeout: opts?.timeout ?? 10000,
    maxBuffer: opts?.maxBuffer
  }).trim()
}

const EXEC_OPTS = {
  encoding: 'utf-8' as const,
  stdio: ['pipe', 'pipe', 'pipe'] as ['pipe', 'pipe', 'pipe']
}

export function isGitRepo(projectPath: string): boolean {
  try {
    return (
      execFileSync(gitBin(), ['rev-parse', '--is-inside-work-tree'], {
        cwd: projectPath,
        ...EXEC_OPTS,
        env: getSafeEnv(),
        timeout: 3000
      }).trim() === 'true'
    )
  } catch {
    return false
  }
}

export function getGitBranch(projectPath: string, remote?: RemoteHost): string | null {
  try {
    const branch = gitExec(['rev-parse', '--abbrev-ref', 'HEAD'], projectPath, {
      timeout: 3000,
      remote
    }).trim()
    return branch && branch !== 'HEAD' ? branch : null
  } catch {
    return null
  }
}

/**
 * Pull `owner/repo` out of a git remote URL, for the forms git actually
 * stores: `git@host:owner/repo.git`, `https://host/owner/repo.git`,
 * `ssh://git@host/owner/repo`, and any of them without the `.git`.
 *
 * Restricted to github.com on purpose. A GitLab or Bitbucket remote parses
 * into the same shape, and handing that to a GitHub connection would produce a
 * connection that looks configured and returns nothing.
 */
export function parseGitHubRemote(url: string): { owner: string; repo: string } | null {
  const trimmed = url.trim()
  if (!trimmed) return null
  const match = trimmed.match(
    /^(?:https?:\/\/|ssh:\/\/)?(?:[^@/]+@)?github\.com[:/]+([^/]+)\/(.+?)(?:\.git)?\/?$/i
  )
  if (!match) return null
  const [, owner, repo] = match
  // A nested path means this is not a repo URL — `github.com/owner/repo/tree/x`
  // would otherwise yield a repo of "repo/tree/x".
  if (!owner || !repo || repo.includes('/')) return null
  return { owner, repo }
}

/**
 * The GitHub repo a project points at, read from its `origin` remote.
 *
 * Reads git rather than `gh repo view`, so repo auto-detect works whether or
 * not the GitHub CLI is installed — the connector that needs `gh` is packaged
 * and separate, and this is the one place Vorn itself wanted to know.
 */
export function detectRepoSlug(projectPath: string): { owner: string; repo: string } | null {
  try {
    return parseGitHubRemote(
      gitExec(['remote', 'get-url', 'origin'], projectPath, { timeout: 3000 })
    )
  } catch {
    // No repo, no origin, or no git. All of them mean "cannot tell", which is
    // what the caller does something useful with.
    return null
  }
}

export function listBranches(projectPath: string, remote?: RemoteHost): string[] {
  try {
    const output = gitExec(['branch', '--format=%(refname:short)'], projectPath, {
      timeout: 5000,
      remote
    }).trim()
    return output
      ? output
          .split('\n')
          .map((b: string) => b.trim())
          .filter(Boolean)
      : []
  } catch {
    return []
  }
}

export function listRemoteBranches(projectPath: string, remote?: RemoteHost): string[] {
  try {
    gitExec(['fetch', '--prune'], projectPath, { timeout: 15000, remote })
    const output = gitExec(['branch', '-r', '--format=%(refname:short)'], projectPath, {
      timeout: 5000,
      remote
    }).trim()
    return output
      ? output
          .split('\n')
          .map((b: string) => b.trim().replace(/^origin\//, ''))
          .filter((b: string) => b && b !== 'HEAD')
      : []
  } catch {
    return []
  }
}

export function checkoutBranch(
  projectPath: string,
  branch: string,
  remote?: RemoteHost
): { ok: boolean; error?: string } {
  try {
    gitExec(['checkout', branch], projectPath, { timeout: 10000, remote })
    return { ok: true }
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err)
    return { ok: false, error: msg }
  }
}

export function extractWorktreeName(worktreePath: string): string {
  const basename = path.basename(worktreePath)
  const match = basename.match(/^(.+)-[0-9a-f]{8}$/)
  return match ? match[1] : basename
}

const ADJECTIVES = [
  'gilded',
  'marble',
  'ornate',
  'sacred',
  'divine',
  'golden',
  'silver',
  'crimson',
  'ivory',
  'velvet',
  'noble',
  'royal',
  'regal',
  'ancient',
  'baroque',
  'classical',
  'tuscan',
  'florentine',
  'venetian',
  'emerald',
  'amber',
  'obsidian',
  'bronze',
  'sienna',
  'scarlet'
]

const NOUNS = [
  'fresco',
  'madrigal',
  'etching',
  'sketch',
  'triptych',
  'inkwell',
  'study',
  'canvas',
  'palette',
  'tableau',
  'vellum',
  'relic',
  'mosaic',
  'statue',
  'chapel',
  'garden',
  'fountain',
  'frieze',
  'archive',
  'folio',
  'portrait',
  'scroll',
  'chronicle',
  'stanza',
  'muse'
]

function generateName(): string {
  const adj = ADJECTIVES[Math.floor(Math.random() * ADJECTIVES.length)]
  const noun = NOUNS[Math.floor(Math.random() * NOUNS.length)]
  return `${adj}-${noun}`
}

/**
 * True when a branch name came from `generateName()` — an adjective-noun pair,
 * optionally suffixed with the 8-hex worktree id. Used to tell branches vorn
 * created for a worktree apart from branches the user named themselves, so
 * cleanup only ever proposes deleting its own leftovers.
 */
export function isGeneratedWorktreeBranch(branch: string): boolean {
  const match = branch.match(/^([a-z]+)-([a-z]+)(?:-[0-9a-f]{8})?$/)
  if (!match) return false
  return ADJECTIVES.includes(match[1]) && NOUNS.includes(match[2])
}

export function createWorktree(
  projectPath: string,
  branch: string,
  worktreeName?: string,
  remote?: RemoteHost
): { worktreePath: string; branch: string; name: string } {
  // Use posix path separators for remote (always Linux)
  const sep = remote ? '/' : path.sep
  const projectName = remote ? projectPath.split('/').pop()! : path.basename(projectPath)
  const shortId = crypto.randomUUID().slice(0, 8)
  const rawName = worktreeName || generateName()
  const name = rawName.replace(/[^a-zA-Z0-9-]/g, '-')
  const parentDir = remote
    ? projectPath.split('/').slice(0, -1).join('/')
    : path.dirname(projectPath)
  const baseDir = `${parentDir}${sep}.vorn-worktrees${sep}${projectName}`
  const worktreeDir = `${baseDir}${sep}${name}-${shortId}`

  if (remote) {
    sshExecSync(remote, `mkdir -p ${shellEscape(baseDir, 'posix')}`, { timeout: 5000 })
  } else {
    fs.mkdirSync(baseDir, { recursive: true })
  }

  const localBranches = listBranches(projectPath, remote)

  if (localBranches.includes(branch)) {
    try {
      gitExec(['worktree', 'add', worktreeDir, branch], projectPath, {
        timeout: 30000,
        remote
      })
    } catch {
      const newBranch = localBranches.includes(name) ? `${name}-${shortId}` : name
      gitExec(['worktree', 'add', '-b', newBranch, worktreeDir, branch], projectPath, {
        timeout: 30000,
        remote
      })
      return { worktreePath: worktreeDir, branch: newBranch, name }
    }
  } else {
    gitExec(['worktree', 'add', '-b', branch, worktreeDir], projectPath, {
      timeout: 30000,
      remote
    })
  }

  return { worktreePath: worktreeDir, branch, name }
}

export function isWorktreeDirty(worktreePath: string, remote?: RemoteHost): boolean {
  try {
    const output = gitExec(['status', '--porcelain'], worktreePath, {
      timeout: 5000,
      remote
    }).trim()
    return output.length > 0
  } catch {
    return true
  }
}

export function renameWorktreeBranch(
  worktreePath: string,
  newBranch: string,
  remote?: RemoteHost
): boolean {
  const trimmed = newBranch.trim()
  if (!trimmed || trimmed.startsWith('-')) return false

  try {
    const currentBranch = getGitBranch(worktreePath, remote)
    if (!currentBranch) {
      gitExec(['switch', '-c', trimmed], worktreePath, { timeout: 10000, remote })
    } else {
      gitExec(['branch', '-m', trimmed], worktreePath, { timeout: 10000, remote })
    }
    return true
  } catch {
    return false
  }
}

export function renameWorktree(
  worktreePath: string,
  newName: string,
  remote?: RemoteHost
): { newPath: string; name: string } | null {
  const trimmed = newName
    .trim()
    .replace(/[^a-zA-Z0-9-]/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-|-$/g, '')
  if (!trimmed) return null

  const sep = remote ? '/' : path.sep
  const dir = remote ? worktreePath.split('/').slice(0, -1).join('/') : path.dirname(worktreePath)
  const basename = remote ? worktreePath.split('/').pop()! : path.basename(worktreePath)
  const idMatch = basename.match(/-([0-9a-f]{8})$/)
  if (!idMatch) return null
  const shortId = idMatch[1]
  const newDir = `${dir}${sep}${trimmed}-${shortId}`

  if (newDir === worktreePath) return null

  if (remote) {
    const check = sshExecSync(
      remote,
      `test -d ${shellEscape(newDir, 'posix')} && echo EXISTS || echo MISSING`,
      { timeout: 5000 }
    ).trim()
    if (check === 'EXISTS') return null
  } else {
    if (fs.existsSync(newDir)) return null
  }

  try {
    gitExec(['worktree', 'move', worktreePath, newDir], worktreePath, {
      timeout: 10000,
      remote
    })
    return { newPath: newDir, name: trimmed }
  } catch {
    return null
  }
}

export function removeWorktree(
  projectPath: string,
  worktreePath: string,
  force = false,
  remote?: RemoteHost,
  deleteBranch = false
): boolean {
  // Read the branch before removal — afterwards git no longer associates it
  // with a path, and we would have nothing left to delete.
  const branch = deleteBranch ? getGitBranch(worktreePath, remote) : null
  try {
    const args = ['worktree', 'remove', worktreePath]
    if (force) args.push('--force')
    gitExec(args, projectPath, { timeout: 10000, remote })
  } catch {
    return false
  }
  // Best-effort, and never forced: `force` here means "discard uncommitted
  // changes", which is not permission to drop unmerged commits. A branch git
  // refuses to delete is left alone rather than failing a removal that
  // already succeeded.
  if (branch) deleteBranches(projectPath, [branch], false, remote)
  return true
}

/**
 * The branch a project's work is measured against. Prefers the remote HEAD
 * symref, then the usual local names, then whatever HEAD points at.
 */
export function getDefaultBranch(projectPath: string, remote?: RemoteHost): string | null {
  try {
    const symref = gitExec(['symbolic-ref', '--short', 'refs/remotes/origin/HEAD'], projectPath, {
      timeout: 5000,
      remote
    }).trim()
    if (symref) return symref.replace(/^origin\//, '')
  } catch {
    // No origin/HEAD — fall through to local names.
  }
  const locals = listBranches(projectPath, remote)
  for (const candidate of ['main', 'master', 'trunk', 'develop']) {
    if (locals.includes(candidate)) return candidate
  }
  return getGitBranch(projectPath, remote)
}

/** True when `branch` is already contained in `base` — nothing would be lost. */
export function isBranchMerged(
  projectPath: string,
  branch: string,
  base: string,
  remote?: RemoteHost
): boolean {
  if (branch === base) return true
  try {
    gitExec(['merge-base', '--is-ancestor', branch, base], projectPath, { timeout: 5000, remote })
    return true
  } catch {
    return false
  }
}

/** The upstream ref for a branch, or null when it was never pushed. */
export function getBranchUpstream(
  projectPath: string,
  branch: string,
  remote?: RemoteHost
): string | null {
  try {
    const upstream = gitExec(
      ['rev-parse', '--abbrev-ref', '--symbolic-full-name', `${branch}@{upstream}`],
      projectPath,
      { timeout: 5000, remote }
    ).trim()
    return upstream || null
  } catch {
    return null
  }
}

/** ISO timestamp of a ref's last commit, or null if the ref is unreadable. */
export function getLastCommitDate(cwd: string, ref = 'HEAD', remote?: RemoteHost): string | null {
  try {
    const raw = gitExec(['log', '-1', '--format=%cI', ref], cwd, { timeout: 5000, remote }).trim()
    return raw || null
  } catch {
    return null
  }
}

/**
 * Branches already contained in `base` — one call instead of a `merge-base`
 * per branch, which matters when a repo has dozens of them.
 */
export function listMergedBranches(
  projectPath: string,
  base: string,
  remote?: RemoteHost
): string[] {
  try {
    const output = gitExec(['branch', '--merged', base, '--format=%(refname:short)'], projectPath, {
      timeout: 10000,
      remote
    }).trim()
    return output
      ? output
          .split('\n')
          .map((b: string) => b.trim())
          .filter(Boolean)
      : []
  } catch {
    return []
  }
}

/**
 * Every local branch with its upstream and last commit date, tab-separated —
 * enough to classify a whole repo's branches in a single git invocation.
 */
export function gitForEachRef(projectPath: string, remote?: RemoteHost): string[] {
  try {
    const output = gitExec(
      [
        'for-each-ref',
        '--format=%(refname:short)%09%(upstream:short)%09%(committerdate:iso-strict)',
        'refs/heads'
      ],
      projectPath,
      { timeout: 10000, remote }
    ).trim()
    return output ? output.split('\n').filter(Boolean) : []
  } catch {
    return []
  }
}

/**
 * The real git directory backing a path. For a linked worktree this is
 * `<repo>/.git/worktrees/<name>`, whose `index` mtime tracks activity in that
 * worktree alone. Returns null when the path isn't inside a repository.
 */
export function getAbsoluteGitDir(anyPath: string, remote?: RemoteHost): string | null {
  try {
    const dir = gitExec(['rev-parse', '--absolute-git-dir'], anyPath, {
      timeout: 5000,
      remote
    }).trim()
    return dir || null
  } catch {
    return null
  }
}

/**
 * Delete local branches. Uses `-d` so git refuses anything unmerged; `force`
 * escalates to `-D` and must be an explicit choice by the user.
 */
export function deleteBranches(
  projectPath: string,
  branches: string[],
  force = false,
  remote?: RemoteHost
): { deleted: string[]; failed: { branch: string; error: string }[] } {
  const deleted: string[] = []
  const failed: { branch: string; error: string }[] = []
  for (const branch of branches) {
    try {
      gitExec(['branch', force ? '-D' : '-d', branch], projectPath, { timeout: 10000, remote })
      deleted.push(branch)
    } catch (err) {
      failed.push({ branch, error: err instanceof Error ? err.message : String(err) })
    }
  }
  return { deleted, failed }
}

export interface WorktreeEntry {
  path: string
  branch: string
  isMain: boolean
  name: string
}

export function getGitDiffStat(
  cwd: string,
  remote?: RemoteHost
): { filesChanged: number; insertions: number; deletions: number } | null {
  try {
    const output = gitExec(['diff', 'HEAD', '--numstat'], cwd, {
      timeout: 10000,
      remote
    }).trim()

    if (!output) return { filesChanged: 0, insertions: 0, deletions: 0 }

    let insertions = 0
    let deletions = 0
    let filesChanged = 0
    for (const line of output.split('\n')) {
      const parts = line.split('\t')
      if (parts[0] === '-') {
        // binary file
        filesChanged++
        continue
      }
      insertions += parseInt(parts[0], 10) || 0
      deletions += parseInt(parts[1], 10) || 0
      filesChanged++
    }
    return { filesChanged, insertions, deletions }
  } catch {
    return null
  }
}

export function getGitDiffFull(
  cwd: string,
  remote?: RemoteHost
): {
  stat: { filesChanged: number; insertions: number; deletions: number }
  files: { filePath: string; status: string; insertions: number; deletions: number; diff: string }[]
} | null {
  try {
    const stat = getGitDiffStat(cwd, remote)
    if (!stat) return null

    const MAX_DIFF_SIZE = 500 * 1024 // 500KB
    let rawDiff = gitExec(['diff', 'HEAD', '-U3'], cwd, {
      timeout: 15000,
      maxBuffer: MAX_DIFF_SIZE * 2,
      remote
    })

    if (rawDiff.length > MAX_DIFF_SIZE) {
      rawDiff = rawDiff.slice(0, MAX_DIFF_SIZE) + '\n\n... diff truncated (too large) ...\n'
    }

    const numstatOutput = gitExec(['diff', 'HEAD', '--numstat'], cwd, {
      timeout: 10000,
      remote
    }).trim()

    const fileStats = new Map<string, { insertions: number; deletions: number }>()
    if (numstatOutput) {
      for (const line of numstatOutput.split('\n')) {
        const parts = line.split('\t')
        if (parts.length >= 3) {
          const ins = parts[0] === '-' ? 0 : parseInt(parts[0], 10) || 0
          const del = parts[1] === '-' ? 0 : parseInt(parts[1], 10) || 0
          fileStats.set(parts.slice(2).join('\t'), { insertions: ins, deletions: del })
        }
      }
    }

    // Split raw diff by file boundaries
    const fileDiffs: {
      filePath: string
      status: string
      insertions: number
      deletions: number
      diff: string
    }[] = []
    const diffSections = rawDiff.split(/^diff --git /m).filter(Boolean)

    for (const section of diffSections) {
      const fullSection = 'diff --git ' + section
      // Extract file path from +++ line
      const plusMatch = fullSection.match(/^\+\+\+ b\/(.+)$/m)
      const minusMatch = fullSection.match(/^--- a\/(.+)$/m)
      const filePath = plusMatch?.[1] || minusMatch?.[1]?.replace(/^\/dev\/null$/, '') || 'unknown'

      // Determine status
      let status: string = 'modified'
      if (fullSection.includes('--- /dev/null')) {
        status = 'added'
      } else if (fullSection.includes('+++ /dev/null')) {
        status = 'deleted'
      } else if (fullSection.includes('rename from')) {
        status = 'renamed'
      }

      const stats = fileStats.get(filePath) || { insertions: 0, deletions: 0 }

      fileDiffs.push({
        filePath,
        status,
        insertions: stats.insertions,
        deletions: stats.deletions,
        diff: fullSection
      })
    }

    return { stat, files: fileDiffs }
  } catch {
    return null
  }
}

export function gitCommit(
  cwd: string,
  message: string,
  includeUnstaged: boolean,
  remote?: RemoteHost
): { success: boolean; error?: string } {
  try {
    if (includeUnstaged) {
      gitExec(['add', '-A'], cwd, { timeout: 10000, remote })
    }
    gitExec(['commit', '-m', message], cwd, { timeout: 15000, remote })
    return { success: true }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    return { success: false, error: msg }
  }
}

export function gitPush(cwd: string, remote?: RemoteHost): { success: boolean; error?: string } {
  try {
    gitExec(['push'], cwd, { timeout: 30000, remote })
    return { success: true }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    return { success: false, error: msg }
  }
}

export function listWorktrees(projectPath: string, remote?: RemoteHost): WorktreeEntry[] {
  try {
    const output = gitExec(['worktree', 'list', '--porcelain'], projectPath, {
      timeout: 5000,
      remote
    }).trim()

    if (!output) return []

    const worktrees: WorktreeEntry[] = []
    const blocks = output.split('\n\n')
    for (const block of blocks) {
      const lines = block.split('\n')
      const wtPath = lines.find((l: string) => l.startsWith('worktree '))?.replace('worktree ', '')
      const branchLine = lines.find((l: string) => l.startsWith('branch '))
      const branch = branchLine?.replace('branch refs/heads/', '') || 'detached'
      if (wtPath) {
        worktrees.push({
          path: wtPath,
          branch,
          isMain: worktrees.length === 0,
          name: extractWorktreeName(wtPath)
        })
      }
    }
    return worktrees
  } catch {
    return []
  }
}
