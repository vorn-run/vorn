import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({ execFileSync: vi.fn() }))

vi.mock('node:fs', () => ({
  default: {
    realpathSync: vi.fn((p: string) => p),
    readdirSync: vi.fn(() => []),
    rmSync: vi.fn(),
    statSync: vi.fn(() => ({ size: 0, mtime: new Date('2026-08-01T00:00:00Z') })),
    accessSync: vi.fn(() => {
      throw new Error('not executable')
    }),
    constants: { X_OK: 1 },
    mkdirSync: vi.fn(),
    existsSync: vi.fn(() => true)
  }
}))

import fs from 'node:fs'
import { execFileSync } from 'node:child_process'
import {
  invalidateSizeCache,
  listOrphanDirs,
  measureWorktree,
  pruneOrphanDirs,
  reclaimArtifacts,
  removeWorktrees,
  scanWorktreeInventory
} from '../packages/server/src/worktree-inventory'
import type { ProjectConfig } from '../packages/shared/src/types'

const mockFs = vi.mocked(fs)
const mockExec = vi.mocked(execFileSync)

const PROJECT = '/dev/repo'
const WT_A = '/dev/.vorn-worktrees/repo/royal-stanza-a0494142'
const WT_B = '/dev/.vorn-worktrees/repo/ivory-relic-b1111111'
const ORPHAN = '/dev/.vorn-worktrees/repo/swift-spark-0f339ef6'

const projects: ProjectConfig[] = [{ name: 'repo', path: PROJECT, preferredAgents: [] }]

/** Per-worktree git answers the router serves. */
interface Fake {
  branch: string
  dirty?: boolean
}

let worktrees: Record<string, Fake>
let mergedBranches: string[]
let refs: string[]
/** Shell commands the code ran, in order — asserted on for the du/find path. */
let shellCalls: string[]
let removedDirs: string[]
let failCommand: ((cmd: string) => boolean) | null

function porcelain(): string {
  const blocks = [`worktree ${PROJECT}\nHEAD aaa\nbranch refs/heads/main`]
  for (const [p, f] of Object.entries(worktrees)) {
    blocks.push(`worktree ${p}\nHEAD bbb\nbranch refs/heads/${f.branch}`)
  }
  return blocks.join('\n\n')
}

/** Routes a mocked execFileSync call to a canned git or shell answer. */
function route(bin: string, args: string[]): string {
  if (args[0] === '-c') {
    const cmd = args[1]
    shellCalls.push(cmd)
    if (failCommand?.(cmd)) throw new Error('command failed')
    if (cmd.startsWith('find ')) {
      const root = cmd.split(' ')[1]
      return root === WT_A ? `${WT_A}/node_modules\n${WT_A}/packages/web/dist` : ''
    }
    if (cmd.startsWith('du -sk')) {
      // 1 MB per path, so totals are easy to reason about in assertions.
      return cmd
        .slice('du -sk '.length)
        .split(' ')
        .map((p) => `1024\t${p}`)
        .join('\n')
    }
    return ''
  }
  if (args[0] === '-ilc') return '' // login-shell env probe at import time

  const key = args.join(' ')
  if (key === 'rev-parse --is-inside-work-tree') return 'true'
  if (key === 'worktree list --porcelain') return porcelain()
  if (key === 'symbolic-ref --short refs/remotes/origin/HEAD') return 'origin/main'
  if (key.startsWith('branch --merged')) return mergedBranches.join('\n')
  if (key.startsWith('for-each-ref')) return refs.join('\n')
  if (key === 'rev-parse --absolute-git-dir') return '/dev/repo/.git/worktrees/x'
  if (key === 'rev-parse --abbrev-ref HEAD') return worktrees[currentCwd]?.branch ?? 'HEAD'
  if (key === 'status --porcelain') {
    const cwd = currentCwd
    return worktrees[cwd]?.dirty ? ' M src/index.ts' : ''
  }
  if (key.startsWith('log -1')) return '2026-05-07T18:22:48-06:00'
  if (key.startsWith('worktree remove')) return ''
  if (key.startsWith('branch -d') || key.startsWith('branch -D')) return ''
  if (key === 'branch --format=%(refname:short)')
    return ['main', ...refs.map((r) => r.split('\t')[0])].join('\n')
  return ''
}

let currentCwd = ''

beforeEach(() => {
  vi.clearAllMocks()
  invalidateSizeCache()
  shellCalls = []
  removedDirs = []
  failCommand = null
  currentCwd = ''
  worktrees = { [WT_A]: { branch: 'royal-stanza' }, [WT_B]: { branch: 'ivory-relic' } }
  mergedBranches = ['main', 'royal-stanza', 'gilded-sketch']
  refs = [
    'main\torigin/main\t2026-08-07T16:18:15-06:00',
    'royal-stanza\t\t2026-05-07T18:22:48-06:00',
    'ivory-relic\torigin/ivory-relic\t2026-07-29T09:09:32-06:00',
    'gilded-sketch\t\t2026-05-01T10:00:00-06:00'
  ]

  mockFs.realpathSync.mockImplementation(((p: string) => p) as never)
  mockFs.readdirSync.mockImplementation((() => []) as never)
  mockFs.rmSync.mockImplementation(((p: string) => {
    removedDirs.push(p)
  }) as never)
  mockFs.statSync.mockImplementation((() => ({
    size: 0,
    mtime: new Date('2026-08-01T00:00:00Z')
  })) as never)

  mockExec.mockImplementation(((bin: string, args: string[], opts?: { cwd?: string }) => {
    currentCwd = opts?.cwd ?? ''
    return route(bin, args)
  }) as never)
})

function scan(overrides: Partial<Parameters<typeof scanWorktreeInventory>[0]> = {}) {
  return scanWorktreeInventory({
    projects,
    resolveRemote: () => undefined,
    getActiveSessions: () => [],
    ...overrides
  })
}

describe('scanWorktreeInventory', () => {
  it('reports the main worktree without measuring or offering it', () => {
    const main = scan().projects[0].entries.find((e) => e.isMain)
    expect(main).toBeDefined()
    expect(main!.verdict.level).toBe('keep')
    expect(main!.sizeBytes).toBe(0)
    // No `du` was run against the project root.
    expect(shellCalls.some((c) => c.startsWith('du') && c.includes(PROJECT + ' '))).toBe(false)
  })

  it('measures each linked worktree and splits build output from source', () => {
    const a = scan().projects[0].entries.find((e) => e.path === WT_A)!
    // One `du` for the tree, one for the two build directories find reported.
    expect(a.sizeBytes).toBe(1024 * 1024)
    expect(a.artifactBytes).toBe(2 * 1024 * 1024 > a.sizeBytes ? a.sizeBytes : 2 * 1024 * 1024)
    expect(a.sizeMeasured).toBe(true)
  })

  it('never lets build output exceed the tree it lives in', () => {
    const a = scan().projects[0].entries.find((e) => e.path === WT_A)!
    expect(a.artifactBytes).toBeLessThanOrEqual(a.sizeBytes)
  })

  it('reads merged and upstream state from one pass over the refs', () => {
    const [a, b] = [WT_A, WT_B].map((p) => scan().projects[0].entries.find((e) => e.path === p)!)
    expect(a).toMatchObject({ isMerged: true, hasUpstream: false })
    expect(b).toMatchObject({ isMerged: false, hasUpstream: true })
    expect(b.verdict.level).toBe('reclaim')
  })

  it('carries uncommitted changes into the verdict', () => {
    worktrees[WT_A].dirty = true
    const a = scan().projects[0].entries.find((e) => e.path === WT_A)!
    expect(a.isDirty).toBe(true)
    expect(a.verdict.level).toBe('review')
  })

  it('marks a worktree with live sessions as keep', () => {
    const a = scan({
      getActiveSessions: (p) => (p === WT_A ? ['s1', 's2'] : [])
    }).projects[0].entries.find((e) => e.path === WT_A)!
    expect(a.activeSessionIds).toEqual(['s1', 's2'])
    expect(a.verdict.level).toBe('keep')
  })

  it('finds directories on disk that git no longer lists', () => {
    mockFs.readdirSync.mockImplementation(((dir: string) =>
      dir === '/dev/.vorn-worktrees/repo'
        ? [
            { name: 'royal-stanza-a0494142', isDirectory: () => true },
            { name: 'swift-spark-0f339ef6', isDirectory: () => true },
            { name: '.DS_Store', isDirectory: () => false }
          ]
        : []) as never)

    const orphan = scan().projects[0].entries.find((e) => e.kind === 'orphan-dir')
    expect(orphan?.path).toBe(ORPHAN)
    expect(orphan?.verdict.level).toBe('orphan')
  })

  it('lists branches left behind, and only vorn-generated ones', () => {
    const { staleBranches } = scan().projects[0]
    expect(staleBranches.map((b) => b.name)).toEqual(['gilded-sketch'])
    expect(staleBranches[0].isMerged).toBe(true)
  })

  it('records a project that is not a git repository instead of throwing', () => {
    mockExec.mockImplementation(((_b: string, args: string[]) =>
      args.join(' ') === 'rev-parse --is-inside-work-tree' ? 'false' : '') as never)
    const project = scan().projects[0]
    expect(project.error).toBe('not a git repository')
    expect(project.entries).toEqual([])
  })

  it('honours a retention threshold and pinned paths', () => {
    const result = scan({
      retention: { idleDaysThreshold: 0, pinnedPaths: [WT_A] }
    }).projects[0]
    expect(result.entries.find((e) => e.path === WT_A)!.verdict.level).toBe('keep')
  })

  it('limits the scan to the requested projects', () => {
    expect(scan({ projectPaths: ['/other'] }).projects).toEqual([])
  })

  it('stamps the scan time', () => {
    expect(Date.parse(scan().scannedAt)).not.toBeNaN()
  })
})

describe('measureWorktree', () => {
  it('serves a cached sample rather than re-running du', () => {
    measureWorktree(WT_A, ['node_modules'])
    const after = shellCalls.length
    measureWorktree(WT_A, ['node_modules'])
    expect(shellCalls.length).toBe(after)
  })

  it('re-measures when asked to refresh', () => {
    measureWorktree(WT_A, ['node_modules'])
    const after = shellCalls.length
    measureWorktree(WT_A, ['node_modules'], undefined, true)
    expect(shellCalls.length).toBeGreaterThan(after)
  })

  it('reports zero and flags itself unmeasured when du fails outright', () => {
    failCommand = (c) => c.startsWith('du')
    const sample = measureWorktree(WT_A, ['node_modules'])
    expect(sample).toMatchObject({ sizeBytes: 0, artifactBytes: 0, measured: false })
  })

  it('prefers a stale number over zero when a later measurement fails', () => {
    const first = measureWorktree(WT_A, ['node_modules'])
    failCommand = (c) => c.startsWith('du')
    const second = measureWorktree(WT_A, ['node_modules'], undefined, true)
    expect(second.sizeBytes).toBe(first.sizeBytes)
    expect(second.measured).toBe(true)
  })

  it('falls back to walking the tree when find fails', () => {
    failCommand = (c) => c.startsWith('find')
    mockFs.readdirSync.mockImplementation((() => []) as never)
    expect(() => measureWorktree(WT_A, ['node_modules'])).not.toThrow()
  })
})

describe('listOrphanDirs', () => {
  it('returns nothing when the worktree root does not exist', () => {
    mockFs.readdirSync.mockImplementation((() => {
      throw new Error('ENOENT')
    }) as never)
    expect(listOrphanDirs(PROJECT, new Set())).toEqual([])
  })

  it('matches registered paths through a symlinked parent', () => {
    mockFs.readdirSync.mockImplementation(((dir: string) =>
      dir === '/dev/.vorn-worktrees/repo'
        ? [{ name: 'royal-stanza-a0494142', isDirectory: () => true }]
        : []) as never)
    mockFs.realpathSync.mockImplementation((() => '/private' + WT_A) as never)
    expect(listOrphanDirs(PROJECT, new Set(['/private' + WT_A]))).toEqual([])
  })
})

describe('reclaimArtifacts', () => {
  it('deletes every build directory inside the worktree and reports the bytes', () => {
    const result = reclaimArtifacts([WT_A], ['node_modules', 'dist'], projects, () => undefined)
    expect(result.succeeded).toEqual([WT_A])
    expect(removedDirs).toEqual([`${WT_A}/node_modules`, `${WT_A}/packages/web/dist`])
    expect(result.freedBytes).toBe(2 * 1024 * 1024)
  })

  it('succeeds without deleting anything when there is no build output', () => {
    const result = reclaimArtifacts([WT_B], ['node_modules'], projects, () => undefined)
    expect(result.succeeded).toEqual([WT_B])
    expect(removedDirs).toEqual([])
  })

  it('refuses a path no project claims as a worktree', () => {
    const result = reclaimArtifacts(
      ['/Users/me/Documents'],
      ['node_modules'],
      projects,
      () => undefined
    )
    expect(result.succeeded).toEqual([])
    expect(result.failed[0].error).toMatch(/not a worktree of any known project/)
    expect(removedDirs).toEqual([])
  })

  it('refuses the main worktree — the project itself is not build output', () => {
    const result = reclaimArtifacts([PROJECT], ['node_modules'], projects, () => undefined)
    expect(result.failed[0].error).toMatch(/not a worktree/)
    expect(removedDirs).toEqual([])
  })

  it('stops a symlinked build directory from reaching outside the worktree', () => {
    mockFs.realpathSync.mockImplementation(((p: string) =>
      p === `${WT_A}/node_modules` ? '/usr/local/lib' : p) as never)
    const result = reclaimArtifacts([WT_A], ['node_modules'], projects, () => undefined)
    expect(result.failed[0].error).toMatch(/outside the worktree/)
    expect(removedDirs).toEqual([])
  })
})

describe('removeWorktrees', () => {
  const sizeOf = () => 5 * 1024 * 1024

  it('removes a worktree and reports the bytes it freed', () => {
    const result = removeWorktrees(
      [{ projectPath: PROJECT, worktreePath: WT_A }],
      sizeOf,
      projects,
      () => undefined
    )
    expect(result.succeeded).toEqual([WT_A])
    expect(result.freedBytes).toBe(5 * 1024 * 1024)
  })

  it('reports a branch as deleted only once it is actually gone', () => {
    refs = refs.filter((r) => !r.startsWith('royal-stanza'))
    const result = removeWorktrees(
      [{ projectPath: PROJECT, worktreePath: WT_A, deleteBranch: true }],
      sizeOf,
      projects,
      () => undefined
    )
    expect(result.deletedBranches).toEqual(['royal-stanza'])
  })

  it('does not claim a branch was deleted when it survived', () => {
    const result = removeWorktrees(
      [{ projectPath: PROJECT, worktreePath: WT_A, deleteBranch: true }],
      sizeOf,
      projects,
      () => undefined
    )
    expect(result.deletedBranches).toEqual([])
  })

  it('refuses a path git does not report as a worktree', () => {
    const result = removeWorktrees(
      [{ projectPath: PROJECT, worktreePath: '/tmp/elsewhere' }],
      sizeOf,
      projects,
      () => undefined
    )
    expect(result.failed[0].error).toMatch(/not a worktree of any known project/)
  })

  it('surfaces a git failure as a per-item error and frees nothing', () => {
    mockExec.mockImplementation(((bin: string, args: string[], opts?: { cwd?: string }) => {
      currentCwd = opts?.cwd ?? ''
      if (args.join(' ').startsWith('worktree remove')) throw new Error('is dirty')
      return route(bin, args)
    }) as never)

    const result = removeWorktrees(
      [{ projectPath: PROJECT, worktreePath: WT_A }],
      sizeOf,
      projects,
      () => undefined
    )
    expect(result.succeeded).toEqual([])
    expect(result.freedBytes).toBe(0)
    expect(result.failed[0].error).toMatch(/git worktree remove failed/)
  })
})

describe('pruneOrphanDirs', () => {
  const sizeOf = () => 1024

  it('deletes a directory git has forgotten', () => {
    mockExec.mockImplementation(((bin: string, args: string[], opts?: { cwd?: string }) => {
      currentCwd = opts?.cwd ?? ''
      // No git directory here — that is what makes it an orphan.
      if (args.join(' ') === 'rev-parse --absolute-git-dir') throw new Error('not a repo')
      return route(bin, args)
    }) as never)

    const result = pruneOrphanDirs([ORPHAN], sizeOf, () => undefined)
    expect(result.succeeded).toEqual([ORPHAN])
    expect(removedDirs).toEqual([ORPHAN])
    expect(result.freedBytes).toBe(1024)
  })

  it('refuses a path git still claims — that one goes through git worktree remove', () => {
    const result = pruneOrphanDirs([WT_A], sizeOf, () => undefined)
    expect(result.failed[0].error).toMatch(/still registered with git/)
    expect(removedDirs).toEqual([])
  })

  it('refuses anything outside the worktree root, whatever git says', () => {
    const result = pruneOrphanDirs(['/Users/me/Documents'], sizeOf, () => undefined)
    expect(result.failed[0].error).toMatch(/outside \.vorn-worktrees/)
    expect(removedDirs).toEqual([])
  })
})

describe('invalidateSizeCache', () => {
  it('drops a measured path so the next read re-runs du', () => {
    measureWorktree(WT_A, ['node_modules'])
    invalidateSizeCache(WT_A)
    const after = shellCalls.length
    measureWorktree(WT_A, ['node_modules'])
    expect(shellCalls.length).toBeGreaterThan(after)
  })

  it('drops everything beneath a prefix', () => {
    measureWorktree(WT_A, ['node_modules'])
    invalidateSizeCache('/dev/.vorn-worktrees')
    const after = shellCalls.length
    measureWorktree(WT_A, ['node_modules'])
    expect(shellCalls.length).toBeGreaterThan(after)
  })
})
