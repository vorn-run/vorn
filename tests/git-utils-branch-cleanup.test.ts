import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn()
}))

vi.mock('node:fs', () => ({
  default: {
    mkdirSync: vi.fn(),
    existsSync: vi.fn(() => true)
  }
}))

import { execFileSync } from 'node:child_process'
import {
  deleteBranches,
  getDefaultBranch,
  isBranchMerged,
  isGeneratedWorktreeBranch,
  removeWorktree
} from '../packages/server/src/git-utils'

const mockExecFileSync = vi.mocked(execFileSync)

/** git args of every call, ignoring which binary path was resolved. */
function callArgs(): string[][] {
  return mockExecFileSync.mock.calls.map((c) => c[1] as string[])
}

beforeEach(() => {
  vi.clearAllMocks()
  mockExecFileSync.mockReturnValue('' as never)
})

describe('isGeneratedWorktreeBranch', () => {
  it('recognises vorn adjective-noun names, with or without the id suffix', () => {
    expect(isGeneratedWorktreeBranch('royal-stanza')).toBe(true)
    expect(isGeneratedWorktreeBranch('gilded-sketch-2a9c9f51')).toBe(true)
    expect(isGeneratedWorktreeBranch('obsidian-study')).toBe(true)
  })

  it('leaves branches the user named alone', () => {
    expect(isGeneratedWorktreeBranch('main')).toBe(false)
    expect(isGeneratedWorktreeBranch('feat/headless-run-diagnostics')).toBe(false)
    expect(isGeneratedWorktreeBranch('chore/release-0.5.3')).toBe(false)
    // Right shape, words that are not in the generator's lists.
    expect(isGeneratedWorktreeBranch('quick-fix')).toBe(false)
    expect(isGeneratedWorktreeBranch('royal-widget')).toBe(false)
  })

  it('does not match a suffix that is not the 8-hex worktree id', () => {
    expect(isGeneratedWorktreeBranch('royal-stanza-v2')).toBe(false)
    expect(isGeneratedWorktreeBranch('royal-stanza-2a9c9f5')).toBe(false)
  })
})

describe('deleteBranches', () => {
  it('uses -d so git refuses anything unmerged', () => {
    deleteBranches('/repo', ['royal-stanza'])
    expect(callArgs()).toContainEqual(['branch', '-d', 'royal-stanza'])
  })

  it('escalates to -D only when forced', () => {
    deleteBranches('/repo', ['royal-stanza'], true)
    expect(callArgs()).toContainEqual(['branch', '-D', 'royal-stanza'])
  })

  it('keeps going after one branch fails and reports both sides', () => {
    mockExecFileSync
      .mockImplementationOnce(() => {
        throw new Error('not fully merged')
      })
      .mockReturnValueOnce('' as never)

    const result = deleteBranches('/repo', ['ivory-relic', 'royal-stanza'])
    expect(result.deleted).toEqual(['royal-stanza'])
    expect(result.failed).toEqual([{ branch: 'ivory-relic', error: 'not fully merged' }])
  })
})

describe('removeWorktree', () => {
  it('leaves the branch alone by default', () => {
    removeWorktree('/repo', '/repo/.vorn-worktrees/p/wt')
    expect(callArgs()).toEqual([['worktree', 'remove', '/repo/.vorn-worktrees/p/wt']])
  })

  it('reads the branch before removal, then deletes it when asked', () => {
    mockExecFileSync.mockReturnValueOnce('royal-stanza\n' as never)

    removeWorktree('/repo', '/repo/.vorn-worktrees/p/wt', false, undefined, true)

    expect(callArgs()).toEqual([
      ['rev-parse', '--abbrev-ref', 'HEAD'],
      ['worktree', 'remove', '/repo/.vorn-worktrees/p/wt'],
      ['branch', '-d', 'royal-stanza']
    ])
  })

  it('never force-deletes the branch just because the removal was forced', () => {
    mockExecFileSync.mockReturnValueOnce('royal-stanza\n' as never)

    removeWorktree('/repo', '/repo/.vorn-worktrees/p/wt', true, undefined, true)

    expect(callArgs()).toContainEqual([
      'worktree',
      'remove',
      '/repo/.vorn-worktrees/p/wt',
      '--force'
    ])
    expect(callArgs()).toContainEqual(['branch', '-d', 'royal-stanza'])
    expect(callArgs()).not.toContainEqual(['branch', '-D', 'royal-stanza'])
  })

  it('does not try to delete a branch when the worktree is detached', () => {
    mockExecFileSync.mockReturnValueOnce('HEAD\n' as never)

    removeWorktree('/repo', '/repo/.vorn-worktrees/p/wt', false, undefined, true)

    expect(callArgs().some((a) => a[0] === 'branch')).toBe(false)
  })

  it('reports failure and skips the branch when the removal itself fails', () => {
    mockExecFileSync.mockReturnValueOnce('royal-stanza\n' as never).mockImplementationOnce(() => {
      throw new Error('worktree is dirty')
    })

    expect(removeWorktree('/repo', '/repo/.vorn-worktrees/p/wt', false, undefined, true)).toBe(
      false
    )
    expect(callArgs().some((a) => a[0] === 'branch')).toBe(false)
  })
})

describe('getDefaultBranch', () => {
  it('prefers the remote HEAD symref', () => {
    mockExecFileSync.mockReturnValueOnce('origin/main\n' as never)
    expect(getDefaultBranch('/repo')).toBe('main')
  })

  it('falls back to a conventional local name when there is no origin/HEAD', () => {
    mockExecFileSync
      .mockImplementationOnce(() => {
        throw new Error('no symref')
      })
      .mockReturnValueOnce('feature-x\nmaster\ntopic\n' as never)
    expect(getDefaultBranch('/repo')).toBe('master')
  })
})

describe('isBranchMerged', () => {
  it('treats a branch as merged into itself without asking git', () => {
    expect(isBranchMerged('/repo', 'main', 'main')).toBe(true)
    expect(mockExecFileSync).not.toHaveBeenCalled()
  })

  it('reads a non-zero exit from merge-base as not merged', () => {
    mockExecFileSync.mockImplementationOnce(() => {
      throw new Error('exit 1')
    })
    expect(isBranchMerged('/repo', 'ivory-relic', 'main')).toBe(false)
  })
})
