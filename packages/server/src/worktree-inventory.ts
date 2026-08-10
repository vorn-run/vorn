import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type {
  BranchDeleteResult,
  ProjectConfig,
  RemoteHost,
  StaleBranch,
  WorktreeActionResult,
  WorktreeInventory,
  WorktreeInventoryEntry,
  WorktreeProjectInventory,
  WorktreeRetentionConfig,
  WorktreeVerdict
} from '@vornrun/shared/types'
import {
  DEFAULT_ARTIFACT_DIRS,
  DEFAULT_IDLE_DAYS_THRESHOLD,
  getProjectHostIds
} from '@vornrun/shared/types'
import { getSafeEnv, shellEscape, sshExecSync } from './process-utils'
import * as gitUtils from './git-utils'
import log from './logger'

/** The directory vorn parks every worktree under, relative to a project's parent. */
export const WORKTREE_ROOT_SEGMENT = '.vorn-worktrees'

/** Sizes are stable enough that re-measuring on every panel open is waste. */
const SIZE_CACHE_TTL_MS = 5 * 60 * 1000
const DU_TIMEOUT_MS = 45_000
const MS_PER_DAY = 24 * 60 * 60 * 1000

interface SizeSample {
  sizeBytes: number
  artifactBytes: number
  measuredAt: number
}

const sizeCache = new Map<string, SizeSample>()

/** Drop cached sizes for a path (and anything under it) after it changes on disk. */
export function invalidateSizeCache(prefix?: string): void {
  if (!prefix) {
    sizeCache.clear()
    return
  }
  for (const key of sizeCache.keys()) {
    if (key === prefix || key.startsWith(prefix + path.sep) || key.startsWith(prefix + '/')) {
      sizeCache.delete(key)
    }
  }
}

// ─── Shell helpers ──────────────────────────────────────────────

/**
 * Whether this host can run the POSIX `du`/`find` fast path. Windows without a
 * remote falls back to walking the tree in Node, which is slower but correct.
 */
function hasPosixTools(remote?: RemoteHost): boolean {
  return !!remote || process.platform !== 'win32'
}

function runShell(cmd: string, remote: RemoteHost | undefined, timeout: number): string {
  if (remote) return sshExecSync(remote, cmd, { timeout })
  return execFileSync('/bin/sh', ['-c', cmd], {
    encoding: 'utf-8',
    stdio: ['pipe', 'pipe', 'pipe'],
    env: getSafeEnv(),
    timeout,
    maxBuffer: 8 * 1024 * 1024
  }).trim()
}

function joinPath(base: string, child: string, remote?: RemoteHost): string {
  return remote ? `${base}/${child}` : path.join(base, child)
}

function baseName(p: string, remote?: RemoteHost): string {
  if (remote) return p.split('/').filter(Boolean).pop() ?? p
  return path.basename(p)
}

function parentDir(p: string, remote?: RemoteHost): string {
  if (remote) return p.split('/').slice(0, -1).join('/')
  return path.dirname(p)
}

/** Where a project's worktrees live: `<parent>/.vorn-worktrees/<project>`. */
export function worktreeBaseDir(projectPath: string, remote?: RemoteHost): string {
  const parent = parentDir(projectPath, remote)
  const name = baseName(projectPath, remote)
  return joinPath(joinPath(parent, WORKTREE_ROOT_SEGMENT, remote), name, remote)
}

// ─── Deletion guards ────────────────────────────────────────────
//
// Two different questions, two different authorities.
//
// `git worktree remove` only ever touches a path git already claims as a
// worktree of the repo, and refuses to drop uncommitted work unless forced.
// For that, git's own answer is the authority — a worktree created by hand
// outside `.vorn-worktrees/` is still perfectly safe to remove, and refusing
// it would mean showing a row the button can't act on.
//
// A raw `fs.rm` has no such backstop, so it carries the strict checks: the
// target must sit inside a worktree vorn resolved from git, or — for orphan
// directories, which by definition no longer have a git record — inside
// `.vorn-worktrees/<project>/`.

/**
 * Refuse to delete anything that isn't inside a `.vorn-worktrees/<project>/`
 * directory. Symlinks are resolved first so a link planted inside the worktree
 * root can't be used to reach the rest of the filesystem.
 *
 * Used for orphan directories only — the one case with no git record to check
 * against. Throws with a readable reason; callers surface it per path.
 */
export function assertRemovablePath(target: string, remote?: RemoteHost): void {
  if (!target || target.trim() === '') {
    throw new Error('Refusing to delete an empty path')
  }

  let resolved = remote ? target : path.resolve(target)
  if (!remote) {
    try {
      resolved = fs.realpathSync(resolved)
    } catch {
      // Missing paths keep their resolved form — nothing to delete, and the
      // segment check below still applies.
    }
  }

  const sep = remote ? '/' : path.sep
  const segments = resolved.split(sep).filter(Boolean)
  const rootIndex = segments.lastIndexOf(WORKTREE_ROOT_SEGMENT)
  if (rootIndex === -1) {
    throw new Error(`Refusing to delete a path outside ${WORKTREE_ROOT_SEGMENT}: ${target}`)
  }
  // Must be at least `.vorn-worktrees/<project>/<worktree>` — never the root
  // itself and never a whole project's worktree folder in one go.
  if (segments.length < rootIndex + 3) {
    throw new Error(`Refusing to delete ${target}: not a worktree directory`)
  }
}

/** Resolve symlinks so two spellings of the same directory compare equal. */
function canonical(p: string, remote?: RemoteHost): string {
  if (remote) return p.replace(/\/+$/, '')
  const resolved = path.resolve(p)
  try {
    return fs.realpathSync(resolved)
  } catch {
    return resolved
  }
}

/**
 * Refuse any delete target that isn't the worktree itself or something beneath
 * it. Both sides are canonicalised, so a symlink inside the worktree can't be
 * used to reach out of it.
 */
export function assertInsideWorktree(
  target: string,
  worktreePath: string,
  remote?: RemoteHost
): void {
  if (!target || target.trim() === '') throw new Error('Refusing to delete an empty path')
  const sep = remote ? '/' : path.sep
  const root = canonical(worktreePath, remote)
  const resolved = canonical(target, remote)
  if (resolved !== root && !resolved.startsWith(root + sep)) {
    throw new Error(`Refusing to delete ${target}: outside the worktree at ${worktreePath}`)
  }
}

/**
 * The worktree git says owns this path, across all known projects — or null.
 * The main worktree is deliberately never returned: it is the project itself,
 * and neither removal nor a build-output sweep belongs there.
 */
export function findOwningWorktree(
  target: string,
  projects: ProjectConfig[],
  resolveRemote: (project: ProjectConfig) => RemoteHost | undefined,
  cache = new Map<string, { path: string; isMain: boolean }[]>()
): { projectPath: string; worktreePath: string; remote?: RemoteHost } | null {
  for (const project of projects) {
    const remote = resolveRemote(project)
    let listed = cache.get(project.path)
    if (!listed) {
      listed = gitUtils
        .listWorktrees(project.path, remote)
        .map((wt) => ({ path: wt.path, isMain: wt.isMain }))
      cache.set(project.path, listed)
    }
    const wanted = canonical(target, remote)
    for (const wt of listed) {
      if (wt.isMain) continue
      if (canonical(wt.path, remote) === wanted) {
        return { projectPath: project.path, worktreePath: wt.path, remote }
      }
    }
  }
  return null
}

function removeDir(target: string, remote?: RemoteHost): void {
  if (remote) {
    runShell(`rm -rf ${shellEscape(target, 'posix')}`, remote, 60_000)
    return
  }
  fs.rmSync(target, { recursive: true, force: true })
}

// ─── Sizing ─────────────────────────────────────────────────────

/**
 * Locate build-output directories inside a worktree. `-prune` stops the search
 * descending into them, so a nested `node_modules/.../node_modules` is counted
 * once by its outermost parent.
 */
export function findArtifactDirs(
  root: string,
  artifactDirs: string[],
  remote?: RemoteHost
): string[] {
  if (artifactDirs.length === 0) return []

  if (hasPosixTools(remote)) {
    const nameTests = artifactDirs.map((name) => `-name ${shellEscape(name, 'posix')}`).join(' -o ')
    const cmd = `find ${shellEscape(root, 'posix')} -type d \\( ${nameTests} \\) -prune -print`
    try {
      const out = runShell(cmd, remote, 30_000)
      return out
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean)
    } catch (err) {
      log.warn({ err, root }, '[worktree-inventory] find failed, falling back to walk')
    }
  }

  const found: string[] = []
  const names = new Set(artifactDirs)
  const walk = (dir: string): void => {
    let entries: fs.Dirent[]
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      if (!entry.isDirectory() || entry.isSymbolicLink()) continue
      const full = path.join(dir, entry.name)
      if (names.has(entry.name)) {
        found.push(full)
        continue // prune — don't descend into build output
      }
      if (entry.name === '.git') continue
      walk(full)
    }
  }
  walk(root)
  return found
}

/** Sum `du -sk` over a set of paths, in chunks so the arg list stays sane. */
function duBytes(paths: string[], remote: RemoteHost | undefined, timeout: number): number {
  let totalKb = 0
  for (let i = 0; i < paths.length; i += 50) {
    const chunk = paths.slice(i, i + 50)
    const cmd = `du -sk ${chunk.map((p) => shellEscape(p, 'posix')).join(' ')}`
    // `du` exits non-zero if any single path is unreadable but still reports
    // the rest on stdout, so a throw here means we genuinely got nothing.
    const out = runShell(cmd, remote, timeout)
    for (const line of out.split('\n')) {
      const kb = parseInt(line.trim().split(/\s+/)[0], 10)
      if (!Number.isNaN(kb)) totalKb += kb
    }
  }
  return totalKb * 1024
}

function walkBytes(root: string, prune: Set<string>): number {
  let total = 0
  const walk = (dir: string): void => {
    let entries: fs.Dirent[]
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      const full = path.join(dir, entry.name)
      if (entry.isSymbolicLink()) continue
      if (entry.isDirectory()) {
        if (prune.has(full)) continue
        walk(full)
      } else if (entry.isFile()) {
        try {
          total += fs.statSync(full).size
        } catch {
          // Vanished mid-walk — skip it.
        }
      }
    }
  }
  walk(root)
  return total
}

/**
 * Total bytes on disk for a worktree and, separately, the part that lives in
 * build-output directories — the part a reinstall rebuilds for free.
 */
export function measureWorktree(
  root: string,
  artifactDirs: string[],
  remote?: RemoteHost,
  refresh = false
): SizeSample & { measured: boolean } {
  const cached = sizeCache.get(root)
  if (!refresh && cached && Date.now() - cached.measuredAt < SIZE_CACHE_TTL_MS) {
    return { ...cached, measured: true }
  }

  try {
    const artifacts = findArtifactDirs(root, artifactDirs, remote)
    let sizeBytes: number
    let artifactBytes: number

    if (hasPosixTools(remote)) {
      sizeBytes = duBytes([root], remote, DU_TIMEOUT_MS)
      artifactBytes = artifacts.length ? duBytes(artifacts, remote, DU_TIMEOUT_MS) : 0
    } else {
      const pruneSet = new Set(artifacts)
      const sourceBytes = walkBytes(root, pruneSet)
      artifactBytes = artifacts.reduce((sum, dir) => sum + walkBytes(dir, new Set()), 0)
      sizeBytes = sourceBytes + artifactBytes
    }

    // `du` reports allocated blocks, so a pathological rounding could put the
    // artifact total above the tree total. Keep the pair coherent.
    artifactBytes = Math.min(artifactBytes, sizeBytes)

    const sample: SizeSample = { sizeBytes, artifactBytes, measuredAt: Date.now() }
    sizeCache.set(root, sample)
    return { ...sample, measured: true }
  } catch (err) {
    log.warn({ err, root }, '[worktree-inventory] sizing failed')
    // A stale number beats no number; only fall back to zero if we never had one.
    if (cached) return { ...cached, measured: true }
    return { sizeBytes: 0, artifactBytes: 0, measuredAt: Date.now(), measured: false }
  }
}

// ─── Activity ───────────────────────────────────────────────────

/**
 * Newest git activity in a worktree, taken from the mtime of its index. The
 * index is rewritten by any git operation an agent performs, and unlike a file
 * walk it doesn't get reset to "now" by an unrelated `yarn install`.
 */
function readIndexMtime(worktreePath: string, remote?: RemoteHost): string | null {
  try {
    const gitDir = gitUtils.getAbsoluteGitDir(worktreePath, remote)
    if (!gitDir) return null
    const indexPath = joinPath(gitDir, 'index', remote)
    if (remote) {
      const epoch = parseInt(
        runShell(`stat -c %Y ${shellEscape(indexPath, 'posix')}`, remote, 10_000).trim(),
        10
      )
      return Number.isNaN(epoch) ? null : new Date(epoch * 1000).toISOString()
    }
    return fs.statSync(indexPath).mtime.toISOString()
  } catch {
    return null
  }
}

function daysSince(iso: string | null): number | null {
  if (!iso) return null
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return null
  return Math.max(0, Math.floor((Date.now() - then) / MS_PER_DAY))
}

function newest(a: string | null, b: string | null): string | null {
  if (!a) return b
  if (!b) return a
  return Date.parse(a) >= Date.parse(b) ? a : b
}

// ─── Verdict ────────────────────────────────────────────────────

export interface VerdictInput {
  isMain: boolean
  kind: 'registered' | 'orphan-dir'
  isDirty: boolean
  isMerged: boolean
  hasUpstream: boolean
  activeSessionCount: number
  isPinned: boolean
  sizeBytes: number
  artifactBytes: number
  idleDays: number | null
}

/**
 * The safest action available for one worktree. Ordered most-protective first:
 * anything that could lose work stops at `review`, and only a merged, clean,
 * idle worktree is ever pre-selected.
 */
export function computeVerdict(input: VerdictInput, idleDaysThreshold: number): WorktreeVerdict {
  const {
    isMain,
    kind,
    isDirty,
    isMerged,
    hasUpstream,
    activeSessionCount,
    isPinned,
    sizeBytes,
    artifactBytes,
    idleDays
  } = input

  if (isMain) {
    return { level: 'keep', freesBytes: 0, reasons: ['main worktree'], autoSelect: false }
  }
  if (activeSessionCount > 0) {
    return {
      level: 'keep',
      freesBytes: 0,
      reasons: [`${activeSessionCount} active session${activeSessionCount > 1 ? 's' : ''}`],
      autoSelect: false
    }
  }
  if (isPinned) {
    return { level: 'keep', freesBytes: 0, reasons: ['pinned'], autoSelect: false }
  }
  if (kind === 'orphan-dir') {
    return {
      level: 'orphan',
      freesBytes: sizeBytes,
      reasons: ['not registered with git'],
      autoSelect: false
    }
  }
  if (isDirty) {
    return {
      level: 'review',
      freesBytes: 0,
      reasons: ['uncommitted changes'],
      autoSelect: false
    }
  }
  if (!isMerged && !hasUpstream) {
    return {
      level: 'review',
      freesBytes: 0,
      reasons: ['unmerged and never pushed'],
      autoSelect: false
    }
  }
  if (!isMerged) {
    return {
      level: 'reclaim',
      freesBytes: artifactBytes,
      reasons: ['unmerged but pushed', 'build output can go'],
      autoSelect: false
    }
  }

  const reasons = ['merged']
  if (idleDays !== null) reasons.push(`idle ${idleDays} day${idleDays === 1 ? '' : 's'}`)
  return {
    level: 'remove',
    freesBytes: sizeBytes,
    reasons,
    autoSelect: idleDays === null ? false : idleDays >= idleDaysThreshold
  }
}

// ─── Scan ───────────────────────────────────────────────────────

export interface ScanOptions {
  projects: ProjectConfig[]
  /** Limit the scan to these project paths; omit to scan all of them. */
  projectPaths?: string[]
  refresh?: boolean
  retention?: WorktreeRetentionConfig
  /** Resolves a project to its remote host, or undefined when it is local. */
  resolveRemote: (project: ProjectConfig) => RemoteHost | undefined
  /** Live PTY + headless session ids running in a worktree. */
  getActiveSessions: (worktreePath: string) => string[]
}

/** Branch metadata for a whole repo in one `for-each-ref` call. */
function readBranchInfo(
  projectPath: string,
  remote?: RemoteHost
): Map<string, { upstream: string | null; committerDate: string | null }> {
  const info = new Map<string, { upstream: string | null; committerDate: string | null }>()
  try {
    const out = gitUtils.gitForEachRef(projectPath, remote)
    for (const line of out) {
      const [name, upstream, date] = line.split('\t')
      if (!name) continue
      info.set(name, { upstream: upstream || null, committerDate: date || null })
    }
  } catch {
    // Leave the map empty — callers treat a miss as "unknown", not "absent".
  }
  return info
}

function scanProject(
  project: ProjectConfig,
  opts: ScanOptions,
  artifactDirs: string[],
  idleDaysThreshold: number,
  pinned: Set<string>
): WorktreeProjectInventory {
  const remote = opts.resolveRemote(project)
  const remoteHostId = remote ? remote.id : null
  const base: WorktreeProjectInventory = {
    projectPath: project.path,
    projectName: project.name,
    defaultBranch: null,
    remoteHostId,
    entries: [],
    staleBranches: []
  }

  if (!remote && !gitUtils.isGitRepo(project.path)) {
    return { ...base, error: 'not a git repository' }
  }

  const worktrees = gitUtils.listWorktrees(project.path, remote)
  if (worktrees.length === 0) {
    return { ...base, error: 'could not read worktrees' }
  }

  const defaultBranch = gitUtils.getDefaultBranch(project.path, remote)
  const branchInfo = readBranchInfo(project.path, remote)
  const mergedBranches = defaultBranch
    ? new Set(gitUtils.listMergedBranches(project.path, defaultBranch, remote))
    : new Set<string>()

  const entries: WorktreeInventoryEntry[] = []
  const registeredPaths = new Set<string>()
  const branchesInUse = new Set<string>()

  for (const wt of worktrees) {
    registeredPaths.add(wt.path)
    if (wt.branch && wt.branch !== 'detached') branchesInUse.add(wt.branch)

    // The main worktree is the project itself — report it so the totals are
    // honest, but never measure or offer to touch it.
    if (wt.isMain) {
      entries.push({
        path: wt.path,
        name: wt.name,
        projectPath: project.path,
        projectName: project.name,
        kind: 'registered',
        branch: wt.branch,
        isMain: true,
        sizeBytes: 0,
        artifactBytes: 0,
        sizeMeasured: false,
        lastCommitAt: null,
        lastTouchedAt: null,
        idleDays: null,
        isDirty: false,
        isMerged: false,
        hasUpstream: false,
        activeSessionIds: [],
        verdict: computeVerdict(
          {
            isMain: true,
            kind: 'registered',
            isDirty: false,
            isMerged: false,
            hasUpstream: false,
            activeSessionCount: 0,
            isPinned: false,
            sizeBytes: 0,
            artifactBytes: 0,
            idleDays: null
          },
          idleDaysThreshold
        )
      })
      continue
    }

    const size = measureWorktree(wt.path, artifactDirs, remote, opts.refresh)
    const branch = wt.branch && wt.branch !== 'detached' ? wt.branch : null
    const info = branch ? branchInfo.get(branch) : undefined
    const lastCommitAt = info?.committerDate ?? gitUtils.getLastCommitDate(wt.path, 'HEAD', remote)
    const lastTouchedAt = readIndexMtime(wt.path, remote)
    const activeSessionIds = opts.getActiveSessions(wt.path)
    const isDirty = gitUtils.isWorktreeDirty(wt.path, remote)
    const isMerged = branch ? mergedBranches.has(branch) : false
    const hasUpstream = !!info?.upstream
    const idleDays = daysSince(newest(lastCommitAt, lastTouchedAt))

    entries.push({
      path: wt.path,
      name: wt.name,
      projectPath: project.path,
      projectName: project.name,
      kind: 'registered',
      branch,
      isMain: false,
      sizeBytes: size.sizeBytes,
      artifactBytes: size.artifactBytes,
      sizeMeasured: size.measured,
      lastCommitAt,
      lastTouchedAt,
      idleDays,
      isDirty,
      isMerged,
      hasUpstream,
      activeSessionIds,
      verdict: computeVerdict(
        {
          isMain: false,
          kind: 'registered',
          isDirty,
          isMerged,
          hasUpstream,
          activeSessionCount: activeSessionIds.length,
          isPinned: pinned.has(wt.path),
          sizeBytes: size.sizeBytes,
          artifactBytes: size.artifactBytes,
          idleDays
        },
        idleDaysThreshold
      )
    })
  }

  // Directories git has forgotten. `git worktree prune` won't report these —
  // once the administrative entry is gone, only the filesystem knows.
  for (const orphanPath of listOrphanDirs(project.path, registeredPaths, remote)) {
    const size = measureWorktree(orphanPath, artifactDirs, remote, opts.refresh)
    const activeSessionIds = opts.getActiveSessions(orphanPath)
    entries.push({
      path: orphanPath,
      name: baseName(orphanPath, remote),
      projectPath: project.path,
      projectName: project.name,
      kind: 'orphan-dir',
      branch: null,
      isMain: false,
      sizeBytes: size.sizeBytes,
      artifactBytes: size.artifactBytes,
      sizeMeasured: size.measured,
      lastCommitAt: null,
      lastTouchedAt: null,
      idleDays: null,
      isDirty: false,
      isMerged: false,
      hasUpstream: false,
      activeSessionIds,
      verdict: computeVerdict(
        {
          isMain: false,
          kind: 'orphan-dir',
          isDirty: false,
          isMerged: false,
          hasUpstream: false,
          activeSessionCount: activeSessionIds.length,
          isPinned: pinned.has(orphanPath),
          sizeBytes: size.sizeBytes,
          artifactBytes: size.artifactBytes,
          idleDays: null
        },
        idleDaysThreshold
      )
    })
  }

  return {
    ...base,
    defaultBranch,
    entries,
    staleBranches: collectStaleBranches(branchInfo, branchesInUse, mergedBranches, defaultBranch)
  }
}

/** Directories under `.vorn-worktrees/<project>` that git no longer tracks. */
export function listOrphanDirs(
  projectPath: string,
  registeredPaths: Set<string>,
  remote?: RemoteHost
): string[] {
  const baseDir = worktreeBaseDir(projectPath, remote)
  let names: string[]
  try {
    if (remote) {
      const out = runShell(
        `ls -1 ${shellEscape(baseDir, 'posix')} 2>/dev/null || true`,
        remote,
        10_000
      )
      names = out
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean)
    } else {
      names = fs
        .readdirSync(baseDir, { withFileTypes: true })
        .filter((e) => e.isDirectory())
        .map((e) => e.name)
    }
  } catch {
    return []
  }

  const orphans: string[] = []
  for (const name of names) {
    if (name.startsWith('.')) continue
    const full = joinPath(baseDir, name, remote)
    if (registeredPaths.has(full)) continue
    // `git worktree list` reports resolved paths; on macOS `/tmp` and other
    // symlinked parents make the string comparison above miss.
    if (!remote) {
      try {
        if (registeredPaths.has(fs.realpathSync(full))) continue
      } catch {
        // Unreadable — fall through and report it; removal is still guarded.
      }
    }
    orphans.push(full)
  }
  return orphans
}

/**
 * Branches left behind by removed worktrees. Restricted to vorn's own
 * generated adjective-noun names so cleanup never proposes deleting a branch
 * the user named themselves.
 */
export function collectStaleBranches(
  branchInfo: Map<string, { upstream: string | null; committerDate: string | null }>,
  branchesInUse: Set<string>,
  mergedBranches: Set<string>,
  defaultBranch: string | null
): StaleBranch[] {
  const stale: StaleBranch[] = []
  for (const [name, info] of branchInfo) {
    if (name === defaultBranch) continue
    if (branchesInUse.has(name)) continue
    if (!gitUtils.isGeneratedWorktreeBranch(name)) continue
    stale.push({
      name,
      isMerged: mergedBranches.has(name),
      hasUpstream: !!info.upstream,
      lastCommitAt: info.committerDate
    })
  }
  return stale.sort((a, b) => a.name.localeCompare(b.name))
}

export function scanWorktreeInventory(opts: ScanOptions): WorktreeInventory {
  const artifactDirs = opts.retention?.artifactDirs?.length
    ? opts.retention.artifactDirs
    : DEFAULT_ARTIFACT_DIRS
  const idleDaysThreshold = opts.retention?.idleDaysThreshold ?? DEFAULT_IDLE_DAYS_THRESHOLD
  const pinned = new Set(opts.retention?.pinnedPaths ?? [])

  const wanted = opts.projectPaths?.length ? new Set(opts.projectPaths) : null
  const projects = opts.projects.filter((p) => !wanted || wanted.has(p.path))

  // Deduplicate: a project configured on several hosts appears once per host in
  // the sidebar, but its worktrees live in one place per host path.
  const seen = new Set<string>()
  const results: WorktreeProjectInventory[] = []
  for (const project of projects) {
    const key = `${getProjectHostIds(project).join(',')}:${project.path}`
    if (seen.has(key)) continue
    seen.add(key)
    try {
      results.push(scanProject(project, opts, artifactDirs, idleDaysThreshold, pinned))
    } catch (err) {
      log.error({ err, project: project.name }, '[worktree-inventory] project scan failed')
      results.push({
        projectPath: project.path,
        projectName: project.name,
        defaultBranch: null,
        remoteHostId: null,
        entries: [],
        staleBranches: [],
        error: err instanceof Error ? err.message : String(err)
      })
    }
  }

  return { projects: results, scannedAt: new Date().toISOString() }
}

// ─── Actions ────────────────────────────────────────────────────

const EMPTY_RESULT = (): WorktreeActionResult => ({
  succeeded: [],
  failed: [],
  freedBytes: 0,
  deletedBranches: []
})

/**
 * Delete build output inside worktrees without touching git state. The highest
 * value action in the manager and the only one that cannot lose work.
 */
export function reclaimArtifacts(
  paths: string[],
  artifactDirs: string[],
  projects: ProjectConfig[],
  resolveRemote: (project: ProjectConfig) => RemoteHost | undefined
): WorktreeActionResult {
  const result = EMPTY_RESULT()
  const listCache = new Map<string, { path: string; isMain: boolean }[]>()

  for (const worktreePath of paths) {
    try {
      // Git decides what counts as a worktree — including ones created by hand
      // outside `.vorn-worktrees/`, which hold build output like any other.
      const owner = findOwningWorktree(worktreePath, projects, resolveRemote, listCache)
      if (!owner) {
        throw new Error('not a worktree of any known project')
      }
      const remote = owner.remote
      const dirs = findArtifactDirs(worktreePath, artifactDirs, remote)
      if (dirs.length === 0) {
        result.succeeded.push(worktreePath)
        continue
      }
      const before = hasPosixTools(remote)
        ? duBytes(dirs, remote, DU_TIMEOUT_MS)
        : dirs.reduce((sum, d) => sum + walkBytes(d, new Set()), 0)

      for (const dir of dirs) {
        // Each artifact directory is re-checked on its own: `find` follows the
        // worktree's real layout, and a symlinked build dir must not become a
        // way out of the worktree root.
        assertInsideWorktree(dir, owner.worktreePath, remote)
        removeDir(dir, remote)
      }
      invalidateSizeCache(worktreePath)
      result.freedBytes += before
      result.succeeded.push(worktreePath)
    } catch (err) {
      result.failed.push({
        path: worktreePath,
        error: err instanceof Error ? err.message : String(err)
      })
    }
  }
  return result
}

export interface RemoveItem {
  projectPath: string
  worktreePath: string
  force?: boolean
  deleteBranch?: boolean
}

/**
 * Remove registered worktrees, optionally taking their branches with them.
 *
 * The guard here is git's own record rather than a path prefix: `git worktree
 * remove` refuses anything it doesn't own and won't drop uncommitted work
 * unless forced, so a worktree created by hand outside `.vorn-worktrees/` is
 * as safe to remove as one vorn made.
 */
export function removeWorktrees(
  items: RemoveItem[],
  sizeOf: (worktreePath: string) => number,
  projects: ProjectConfig[],
  resolveRemote: (project: ProjectConfig) => RemoteHost | undefined
): WorktreeActionResult {
  const result = EMPTY_RESULT()
  const listCache = new Map<string, { path: string; isMain: boolean }[]>()

  for (const item of items) {
    try {
      const owner = findOwningWorktree(item.worktreePath, projects, resolveRemote, listCache)
      if (!owner) {
        throw new Error('not a worktree of any known project')
      }
      const remote = owner.remote
      const bytes = sizeOf(item.worktreePath)
      const branch = item.deleteBranch ? gitUtils.getGitBranch(item.worktreePath, remote) : null

      // Use the project git resolved, not the one the client claimed.
      const ok = gitUtils.removeWorktree(
        owner.projectPath,
        item.worktreePath,
        item.force ?? false,
        remote,
        item.deleteBranch ?? false
      )
      if (!ok) throw new Error('git worktree remove failed')

      invalidateSizeCache(item.worktreePath)
      result.freedBytes += bytes
      result.succeeded.push(item.worktreePath)
      // removeWorktree deletes the branch best-effort; report only what is
      // actually gone so the summary can't overstate what happened.
      if (branch && !gitUtils.listBranches(owner.projectPath, remote).includes(branch)) {
        result.deletedBranches.push(branch)
      }
    } catch (err) {
      result.failed.push({
        path: item.worktreePath,
        error: err instanceof Error ? err.message : String(err)
      })
    }
  }
  return result
}

/** Delete directories git has forgotten. `git worktree remove` can't reach these. */
export function pruneOrphanDirs(
  paths: string[],
  sizeOf: (p: string) => number,
  resolveRemoteByPath: (p: string) => RemoteHost | undefined
): WorktreeActionResult {
  const result = EMPTY_RESULT()

  for (const target of paths) {
    const remote = resolveRemoteByPath(target)
    try {
      assertRemovablePath(target, remote)
      // Last line of defence: if git still claims this path, it is a real
      // worktree and must go through `git worktree remove` instead.
      if (isRegisteredWorktree(target, remote)) {
        throw new Error('still registered with git — remove it as a worktree instead')
      }
      const bytes = sizeOf(target)
      removeDir(target, remote)
      invalidateSizeCache(target)
      result.freedBytes += bytes
      result.succeeded.push(target)
    } catch (err) {
      result.failed.push({ path: target, error: err instanceof Error ? err.message : String(err) })
    }
  }
  return result
}

function isRegisteredWorktree(target: string, remote?: RemoteHost): boolean {
  try {
    return gitUtils.getAbsoluteGitDir(target, remote) !== null
  } catch {
    return false
  }
}

export function deleteStaleBranches(
  projectPath: string,
  branches: string[],
  force: boolean,
  remote?: RemoteHost
): BranchDeleteResult {
  return gitUtils.deleteBranches(projectPath, branches, force, remote)
}
