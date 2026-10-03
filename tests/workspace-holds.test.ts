import { describe, expect, it } from 'vitest'
import { holdWorkspace, isWorkspaceHeld } from '../packages/server/src/workspace-holds'

describe('workspace holds', () => {
  it('holds a path until every holder lets go, whatever its spelling', () => {
    const first = holdWorkspace('/repo/.vorn-worktrees/repo/wt')
    const second = holdWorkspace('/repo/.vorn-worktrees/repo/wt/')
    expect(isWorkspaceHeld('/repo/.vorn-worktrees/repo/wt')).toBe(true)
    first()
    first()
    expect(isWorkspaceHeld('/repo/.vorn-worktrees/repo/wt')).toBe(true)
    second()
    expect(isWorkspaceHeld('/repo/.vorn-worktrees/repo/wt')).toBe(false)
  })

  it('holds nothing else', () => {
    const release = holdWorkspace('/a')
    expect(isWorkspaceHeld('/b')).toBe(false)
    release()
  })
})
