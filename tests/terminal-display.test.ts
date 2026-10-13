import { describe, it, expect } from 'vitest'
import { getBranchLabel } from '../packages/web/src/renderer/lib/terminal-display'

describe('getBranchLabel', () => {
  it('returns undefined when no branch', () => {
    expect(getBranchLabel({})).toBeUndefined()
    expect(getBranchLabel({ branch: undefined })).toBeUndefined()
  })

  it('returns branch for non-worktree sessions', () => {
    expect(getBranchLabel({ branch: 'main' })).toBe('main')
    expect(getBranchLabel({ branch: 'feature/foo', isWorktree: false })).toBe('feature/foo')
  })

  it('returns worktreeName for worktree sessions', () => {
    expect(
      getBranchLabel({
        branch: 'main-worktree-abc123',
        isWorktree: true,
        worktreeName: 'vivid-nova'
      })
    ).toBe('vivid-nova')
  })

  it('falls back to branch when worktree has no name', () => {
    expect(getBranchLabel({ branch: 'main-worktree-abc123', isWorktree: true })).toBe(
      'main-worktree-abc123'
    )

    expect(
      getBranchLabel({ branch: 'main-worktree-abc123', isWorktree: true, worktreeName: '' })
    ).toBe('main-worktree-abc123')
  })

  it('returns worktreeName even when branch is missing (detached HEAD)', () => {
    expect(getBranchLabel({ isWorktree: true, worktreeName: 'vivid-nova' })).toBe('vivid-nova')
  })
})
