import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * VORN_SESSION_ID reaches an agent's terminal, and reaches it *only* from the
 * spawn site.
 *
 * The browser MCP tools resolve which session is calling them from this one
 * variable and take no session argument, so it is the whole of the isolation
 * boundary: if the id leaked into the ambient environment, every child of every
 * session would inherit the same value and an agent could read another
 * session's browser pane.
 */

vi.mock('../packages/server/src/vornd-sessions', async () =>
  (await import('./helpers/fake-vornd-pty')).vorndModule()
)
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(async () => 'main'),
  getGitHead: vi.fn(async () => 'cafe0000'),
  checkoutBranch: vi.fn(async () => {}),
  createWorktree: vi.fn(),
  isGitRepo: vi.fn(async () => false)
}))

import { ptyManager } from '../packages/server/src/pty-manager'
import { fakeVornd } from './helpers/fake-vornd-pty'

/** The env of the nth session vornd was asked to start. */
function envOf(call: number): Record<string, string> {
  return fakeVornd.spawn.mock.calls[call][1].env
}

describe('VORN_SESSION_ID injection', () => {
  beforeEach(() => {
    fakeVornd.reset()
  })

  it('gives an agent session its own id in the environment', async () => {
    const session = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)

    expect(envOf(0).VORN_SESSION_ID).toBe(session.id)
  })

  it('gives two sessions different ids', async () => {
    const a = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)
    const b = await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)

    // Two agents must never resolve to the same browser pane.
    expect(envOf(0).VORN_SESSION_ID).toBe(a.id)
    expect(envOf(1).VORN_SESSION_ID).toBe(b.id)
    expect(a.id).not.toBe(b.id)
  })

  it('gives a plain shell session an id too', async () => {
    const session = ptyManager.createShellPty('/tmp')

    // Shell sessions own a browser pane like any other, so they need the same
    // identity — and the spread must not be shadowed by `integration.env`.
    expect(envOf(0).VORN_SESSION_ID).toBe(session.id)
  })

  it('does not leak the id into the ambient process environment', async () => {
    await ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)

    // `filterEnv` only strips keys, so anything set on process.env here would be
    // inherited by every session spawned afterwards.
    expect(process.env.VORN_SESSION_ID).toBeUndefined()
  })
})
