import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * VORN_SESSION_ID reaches an agent's PTY, and reaches it *only* from the spawn
 * site.
 *
 * The browser MCP tools resolve which session is calling them from this one
 * variable and take no session argument, so it is the whole of the isolation
 * boundary: if the id leaked into the ambient environment, every child of every
 * session would inherit the same value and an agent could read another
 * session's browser pane.
 */

// `packages/server` pins its own node-pty (1.2.0-beta.14) while the root pins
// beta.13, so the bare specifier 'node-pty' resolves to two different files
// depending on who asks. Mocking it from here would patch the *root* copy and
// leave pty-manager's nested one untouched — the spawn would be real and these
// assertions would silently pass against zero recorded calls. Name the nested
// copy explicitly. If the versions are ever deduped this path stops existing
// and the mock fails loudly, which is the outcome we want.
const spawn = vi.hoisted(() =>
  vi.fn(() => ({
    pid: 4242,
    onData: vi.fn(),
    onExit: vi.fn(),
    write: vi.fn(),
    resize: vi.fn(),
    kill: vi.fn()
  }))
)

vi.mock('../packages/server/node_modules/node-pty', () => ({
  default: { spawn },
  spawn
}))
vi.mock('../packages/server/src/git-utils', () => ({
  getGitBranch: vi.fn(() => 'main'),
  checkoutBranch: vi.fn(),
  createWorktree: vi.fn(),
  isGitRepo: vi.fn(() => false)
}))

/** The env of the nth `pty.spawn` call. */
function envOf(call: number): Record<string, string> {
  const args = spawn.mock.calls[call] as unknown as [string, string[], { env: NodeJS.ProcessEnv }]
  return args[2].env as Record<string, string>
}

describe('VORN_SESSION_ID injection', () => {
  beforeEach(() => {
    spawn.mockClear()
    vi.resetModules()
  })

  it('gives an agent session its own id in the environment', async () => {
    const { ptyManager } = await import('../packages/server/src/pty-manager')
    const session = ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)

    expect(envOf(0).VORN_SESSION_ID).toBe(session.id)
  })

  it('gives two sessions different ids', async () => {
    const { ptyManager } = await import('../packages/server/src/pty-manager')
    const a = ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)
    const b = ptyManager.createPty({
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
    const { ptyManager } = await import('../packages/server/src/pty-manager')
    const session = ptyManager.createShellPty('/tmp')

    // Shell sessions own a browser pane like any other, so they need the same
    // identity — and the spread must not be shadowed by `integration.env`.
    expect(envOf(0).VORN_SESSION_ID).toBe(session.id)
  })

  it('does not leak the id into the ambient process environment', async () => {
    const { ptyManager } = await import('../packages/server/src/pty-manager')
    ptyManager.createPty({
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/tmp'
    } as never)

    // `filterEnv` only strips keys, so anything set on process.env here would be
    // inherited by every session spawned afterwards.
    expect(process.env.VORN_SESSION_ID).toBeUndefined()
  })
})
