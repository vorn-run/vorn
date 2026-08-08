import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn(() => '')
}))

vi.mock('node:fs', () => ({
  default: {
    realpathSync: vi.fn((p: string) => p),
    readdirSync: vi.fn(() => []),
    rmSync: vi.fn(),
    statSync: vi.fn(() => ({ size: 0, mtime: new Date(0) })),
    mkdirSync: vi.fn(),
    existsSync: vi.fn(() => true)
  }
}))

import fs from 'node:fs'
import { execFileSync } from 'node:child_process'
import {
  assertInsideWorktree,
  assertRemovablePath,
  findOwningWorktree,
  collectStaleBranches,
  computeVerdict,
  worktreeBaseDir,
  type VerdictInput
} from '../packages/server/src/worktree-inventory'

const mockFs = vi.mocked(fs)

beforeEach(() => {
  vi.clearAllMocks()
  mockFs.realpathSync.mockImplementation(((p: string) => p) as never)
})

const GB = 1024 ** 3

function input(overrides: Partial<VerdictInput> = {}): VerdictInput {
  return {
    isMain: false,
    kind: 'registered',
    isDirty: false,
    isMerged: true,
    hasUpstream: true,
    activeSessionCount: 0,
    isPinned: false,
    sizeBytes: GB,
    artifactBytes: GB / 2,
    idleDays: 90,
    ...overrides
  }
}

describe('computeVerdict', () => {
  it('never offers to touch the main worktree', () => {
    const verdict = computeVerdict(input({ isMain: true }), 14)
    expect(verdict.level).toBe('keep')
    expect(verdict.freesBytes).toBe(0)
    expect(verdict.autoSelect).toBe(false)
  })

  it('keeps a worktree with live sessions, even when it is merged and idle', () => {
    const verdict = computeVerdict(input({ activeSessionCount: 2 }), 14)
    expect(verdict.level).toBe('keep')
    expect(verdict.reasons).toEqual(['2 active sessions'])
  })

  it('live sessions outrank uncommitted changes', () => {
    const verdict = computeVerdict(input({ activeSessionCount: 1, isDirty: true }), 14)
    expect(verdict.level).toBe('keep')
  })

  it('keeps a pinned worktree', () => {
    expect(computeVerdict(input({ isPinned: true }), 14).level).toBe('keep')
  })

  it('flags a directory git has forgotten as an orphan worth its full size', () => {
    const verdict = computeVerdict(input({ kind: 'orphan-dir', isMerged: false }), 14)
    expect(verdict.level).toBe('orphan')
    expect(verdict.freesBytes).toBe(GB)
    // Never pre-selected: deleting it bypasses git entirely.
    expect(verdict.autoSelect).toBe(false)
  })

  it('sends uncommitted changes to review and frees nothing', () => {
    const verdict = computeVerdict(input({ isDirty: true }), 14)
    expect(verdict.level).toBe('review')
    expect(verdict.freesBytes).toBe(0)
    expect(verdict.autoSelect).toBe(false)
  })

  it('sends unmerged, never-pushed work to review — removal would be unrecoverable', () => {
    const verdict = computeVerdict(input({ isMerged: false, hasUpstream: false }), 14)
    expect(verdict.level).toBe('review')
    expect(verdict.reasons).toContain('unmerged and never pushed')
  })

  it('offers build output only when the branch is unmerged but pushed', () => {
    const verdict = computeVerdict(input({ isMerged: false, hasUpstream: true }), 14)
    expect(verdict.level).toBe('reclaim')
    expect(verdict.freesBytes).toBe(GB / 2)
    expect(verdict.autoSelect).toBe(false)
  })

  it('marks a merged, clean, idle worktree removable and pre-selects it', () => {
    const verdict = computeVerdict(input({ idleDays: 92 }), 14)
    expect(verdict.level).toBe('remove')
    expect(verdict.freesBytes).toBe(GB)
    expect(verdict.autoSelect).toBe(true)
    expect(verdict.reasons).toEqual(['merged', 'idle 92 days'])
  })

  it('still allows removing a merged worktree touched yesterday, but does not pre-select it', () => {
    const verdict = computeVerdict(input({ idleDays: 1 }), 14)
    expect(verdict.level).toBe('remove')
    expect(verdict.autoSelect).toBe(false)
    expect(verdict.reasons).toEqual(['merged', 'idle 1 day'])
  })

  it('does not pre-select when the age is unknown', () => {
    expect(computeVerdict(input({ idleDays: null }), 14).autoSelect).toBe(false)
  })

  it('honours a zero threshold by pre-selecting every removable worktree', () => {
    expect(computeVerdict(input({ idleDays: 0 }), 0).autoSelect).toBe(true)
  })
})

describe('assertRemovablePath', () => {
  const root = '/Users/dev/.vorn-worktrees/vorn'

  it('accepts a worktree directory', () => {
    expect(() => assertRemovablePath(`${root}/royal-stanza-a0494142`)).not.toThrow()
  })

  it('accepts a build directory inside a worktree', () => {
    expect(() => assertRemovablePath(`${root}/royal-stanza-a0494142/node_modules`)).not.toThrow()
  })

  it('refuses anything outside the worktree root', () => {
    expect(() => assertRemovablePath('/Users/dev/vorn')).toThrow(/outside \.vorn-worktrees/)
    expect(() => assertRemovablePath('/')).toThrow(/outside \.vorn-worktrees/)
  })

  it('refuses the worktree root itself and a whole project folder', () => {
    expect(() => assertRemovablePath('/Users/dev/.vorn-worktrees')).toThrow(
      /not a worktree directory/
    )
    expect(() => assertRemovablePath(root)).toThrow(/not a worktree directory/)
  })

  it('refuses an empty path', () => {
    expect(() => assertRemovablePath('')).toThrow(/empty path/)
    expect(() => assertRemovablePath('   ')).toThrow(/empty path/)
  })

  it('resolves symlinks first, so a link out of the root cannot be followed', () => {
    mockFs.realpathSync.mockImplementation((() => '/Users/dev/vorn/src') as never)
    expect(() => assertRemovablePath(`${root}/escape/node_modules`)).toThrow(
      /outside \.vorn-worktrees/
    )
  })

  it('still guards a path that does not exist yet', () => {
    mockFs.realpathSync.mockImplementation((() => {
      throw new Error('ENOENT')
    }) as never)
    expect(() => assertRemovablePath('/etc/passwd')).toThrow(/outside \.vorn-worktrees/)
    expect(() => assertRemovablePath(`${root}/gone`)).not.toThrow()
  })
})

describe('assertInsideWorktree', () => {
  const wt = '/Users/dev/.vorn-worktrees/vorn/royal-stanza-a0494142'

  it('accepts the worktree itself and anything beneath it', () => {
    expect(() => assertInsideWorktree(wt, wt)).not.toThrow()
    expect(() => assertInsideWorktree(`${wt}/packages/server/node_modules`, wt)).not.toThrow()
  })

  it('refuses a sibling whose name merely starts the same', () => {
    expect(() => assertInsideWorktree(`${wt}-backup/node_modules`, wt)).toThrow(
      /outside the worktree/
    )
  })

  it('refuses a path that escapes upward', () => {
    expect(() => assertInsideWorktree('/Users/dev/vorn/node_modules', wt)).toThrow(
      /outside the worktree/
    )
  })

  it('refuses an empty target', () => {
    expect(() => assertInsideWorktree('', wt)).toThrow(/empty path/)
  })

  it('compares real paths, so a symlink out of the worktree is caught', () => {
    mockFs.realpathSync.mockImplementation(((p: string) =>
      p === `${wt}/node_modules` ? '/Users/dev/vorn/node_modules' : p) as never)
    expect(() => assertInsideWorktree(`${wt}/node_modules`, wt)).toThrow(/outside the worktree/)
  })
})

describe('findOwningWorktree', () => {
  const projects = [{ name: 'eclat', path: '/Users/dev/eclat', preferredAgents: [] }]
  const porcelain = [
    'worktree /Users/dev/eclat',
    'HEAD abc',
    'branch refs/heads/main',
    '',
    'worktree /Users/dev/eclat-codex-oauth',
    'HEAD def',
    'branch refs/heads/codex-oauth'
  ].join('\n')

  beforeEach(() => {
    vi.mocked(execFileSync).mockReturnValue(porcelain as never)
  })

  it('finds a worktree git owns even though it sits outside .vorn-worktrees', () => {
    const owner = findOwningWorktree('/Users/dev/eclat-codex-oauth', projects, () => undefined)
    expect(owner).toEqual({
      projectPath: '/Users/dev/eclat',
      worktreePath: '/Users/dev/eclat-codex-oauth',
      remote: undefined
    })
  })

  it('never returns the main worktree — the project itself is not a cleanup target', () => {
    expect(findOwningWorktree('/Users/dev/eclat', projects, () => undefined)).toBeNull()
  })

  it('returns null for a path no project claims', () => {
    expect(findOwningWorktree('/Users/dev/somewhere-else', projects, () => undefined)).toBeNull()
  })

  it('lists each project only once across a batch', () => {
    const cache = new Map()
    findOwningWorktree('/Users/dev/eclat-codex-oauth', projects, () => undefined, cache)
    const callsAfterFirst = vi.mocked(execFileSync).mock.calls.length
    findOwningWorktree('/Users/dev/eclat-codex-oauth', projects, () => undefined, cache)
    expect(vi.mocked(execFileSync).mock.calls.length).toBe(callsAfterFirst)
  })
})

describe('worktreeBaseDir', () => {
  it('sits beside the project, not inside it', () => {
    expect(worktreeBaseDir('/Users/dev/vorn')).toBe('/Users/dev/.vorn-worktrees/vorn')
  })

  it('uses posix separators for a remote host', () => {
    const remote = { id: 'h1', label: 'box', hostname: 'h', user: 'u', port: 22 }
    expect(worktreeBaseDir('/srv/code/vorn', remote)).toBe('/srv/code/.vorn-worktrees/vorn')
  })
})

describe('collectStaleBranches', () => {
  const info = new Map([
    ['main', { upstream: 'origin/main', committerDate: '2026-08-07T16:18:15-06:00' }],
    ['royal-stanza', { upstream: null, committerDate: '2026-05-07T18:22:48-06:00' }],
    ['ivory-relic', { upstream: 'origin/ivory-relic', committerDate: '2026-08-07T16:18:15-06:00' }],
    ['gilded-sketch-2a9c9f51', { upstream: null, committerDate: '2026-05-07T18:22:48-06:00' }],
    ['feat/headless-run-diagnostics', { upstream: 'origin/feat', committerDate: null }],
    ['obsidian-study', { upstream: null, committerDate: null }]
  ])

  it('reports only branches vorn generated, and only those without a worktree', () => {
    const stale = collectStaleBranches(
      info,
      new Set(['main', 'obsidian-study']),
      new Set(['royal-stanza', 'gilded-sketch-2a9c9f51']),
      'main'
    )
    expect(stale.map((b) => b.name)).toEqual([
      'gilded-sketch-2a9c9f51',
      'ivory-relic',
      'royal-stanza'
    ])
  })

  it('never proposes a branch the user named themselves', () => {
    const stale = collectStaleBranches(info, new Set(), new Set(), 'main')
    expect(stale.map((b) => b.name)).not.toContain('feat/headless-run-diagnostics')
  })

  it('carries merged and upstream state through for the UI', () => {
    const stale = collectStaleBranches(info, new Set(), new Set(['royal-stanza']), 'main')
    const royal = stale.find((b) => b.name === 'royal-stanza')
    expect(royal).toMatchObject({ isMerged: true, hasUpstream: false })
    const ivory = stale.find((b) => b.name === 'ivory-relic')
    expect(ivory).toMatchObject({ isMerged: false, hasUpstream: true })
  })

  it('leaves the default branch alone even when it looks generated', () => {
    const stale = collectStaleBranches(info, new Set(), new Set(), 'royal-stanza')
    expect(stale.map((b) => b.name)).not.toContain('royal-stanza')
  })
})
