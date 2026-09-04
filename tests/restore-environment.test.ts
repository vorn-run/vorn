import { describe, it, expect, vi } from 'vitest'
import { headMoved } from '@vornrun/shared/types'
import { probeEnvironment } from '../packages/server/src/restore-environment'

const tree = (over: Partial<Record<'dirs' | 'branch' | 'head', unknown>> = {}) => ({
  isDirectory: (at: string) => ((over.dirs as string[]) ?? ['/repo', '/repo/.wt/f']).includes(at),
  branch: vi.fn(() => (over.branch as string | null) ?? 'feature'),
  head: vi.fn(() => (over.head as string | null) ?? 'aaaa1111')
})

const agent = {
  projectPath: '/repo',
  worktreePath: '/repo/.wt/f',
  branch: 'feature',
  headCommit: 'aaaa1111'
}

describe('checking a record against the tree before offering Resume', () => {
  it('reports the ordinary case: everything where it was left', () => {
    expect(probeEnvironment(agent, tree())).toEqual({
      worktree: 'ok',
      branch: { recorded: 'feature', actual: 'feature' },
      head: { recorded: 'aaaa1111', actual: 'aaaa1111' }
    })
  })

  it('names both commits when HEAD moved', () => {
    const env = probeEnvironment(agent, tree({ head: 'bbbb2222' }))
    expect(env?.head).toEqual({ recorded: 'aaaa1111', actual: 'bbbb2222' })
    expect(headMoved(env)).toBe(true)
  })

  it('says the worktree is missing and asks git nothing about it', () => {
    const probe = tree({ dirs: ['/repo'] })
    const env = probeEnvironment(agent, probe)
    expect(env?.worktree).toBe('missing')
    expect(env?.head.actual).toBeNull()
    expect(probe.head).not.toHaveBeenCalled()
    expect(probe.branch).not.toHaveBeenCalled()
  })

  it('checks the project when there is no worktree', () => {
    const probe = tree()
    probeEnvironment({ projectPath: '/repo' }, probe)
    expect(probe.head).toHaveBeenCalledWith('/repo')
  })

  it('does not call an unknown commit a move', () => {
    // Records written before HEAD was recorded have nothing to compare.
    expect(headMoved(probeEnvironment({ projectPath: '/repo' }, tree()))).toBe(false)
    expect(headMoved(probeEnvironment(agent, tree({ head: null })))).toBe(false)
    expect(headMoved(undefined)).toBe(false)
  })

  it('leaves a remote session unchecked rather than probing this machine', () => {
    const probe = tree()
    expect(probeEnvironment({ ...agent, remoteHostId: 'box' }, probe)).toBeUndefined()
    expect(probe.head).not.toHaveBeenCalled()
  })
})
