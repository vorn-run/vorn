import { describe, it, expect, vi } from 'vitest'
import type { TerminalSession } from '@vornrun/shared/types'
import { HeadRefresh, HEAD_REFRESH_MS } from '../packages/server/src/head-commit'

const session = (over: Partial<TerminalSession> = {}): TerminalSession =>
  ({ id: 'a', projectPath: '/repo', ...over }) as TerminalSession

describe('keeping the recorded HEAD following the tree', () => {
  it('reads it the first time and writes it onto the session', () => {
    const read = vi.fn(() => 'abc123')
    const s = session()
    new HeadRefresh(read).refresh([s], 1000)
    expect(read).toHaveBeenCalledWith('/repo')
    expect(s.headCommit).toBe('abc123')
  })

  it('prefers the worktree over the project', () => {
    const read = vi.fn(() => 'abc123')
    new HeadRefresh(read).refresh([session({ worktreePath: '/repo/.wt/x' })], 1000)
    expect(read).toHaveBeenCalledWith('/repo/.wt/x')
  })

  it('does not ask again inside the window, however many saves fire', () => {
    // Saves run every 500 ms on a busy board; ten sessions would be ten
    // subprocesses a second without this.
    const read = vi.fn(() => 'abc123')
    const heads = new HeadRefresh(read)
    const s = session()
    heads.refresh([s], 1000)
    heads.refresh([s], 1500)
    heads.refresh([s], 1000 + HEAD_REFRESH_MS - 1)
    expect(read).toHaveBeenCalledTimes(1)
    heads.refresh([s], 1000 + HEAD_REFRESH_MS)
    expect(read).toHaveBeenCalledTimes(2)
  })

  it('follows a commit the agent made', () => {
    let head = 'before'
    const heads = new HeadRefresh(() => head)
    const s = session()
    heads.refresh([s], 1000)
    head = 'after'
    heads.refresh([s], 1000 + HEAD_REFRESH_MS)
    expect(s.headCommit).toBe('after')
  })

  it('asks again at once after being invalidated', () => {
    const read = vi.fn(() => 'abc123')
    const heads = new HeadRefresh(read)
    const s = session()
    heads.refresh([s], 1000)
    heads.invalidate('a')
    heads.refresh([s], 1001)
    expect(read).toHaveBeenCalledTimes(2)
  })

  it('keeps the last known value when git cannot answer', () => {
    let head: string | null = 'abc123'
    const heads = new HeadRefresh(() => head)
    const s = session()
    heads.refresh([s], 1000)
    head = null
    heads.refresh([s], 1000 + HEAD_REFRESH_MS)
    expect(s.headCommit).toBe('abc123')
  })

  it('never runs git for a remote session', () => {
    const read = vi.fn(() => 'abc123')
    new HeadRefresh(read).refresh([session({ remoteHostId: 'box' })], 1000)
    expect(read).not.toHaveBeenCalled()
  })

  it('throttles per session, not across them', () => {
    const read = vi.fn(() => 'abc123')
    new HeadRefresh(read).refresh([session({ id: 'a' }), session({ id: 'b' })], 1000)
    expect(read).toHaveBeenCalledTimes(2)
  })
})
