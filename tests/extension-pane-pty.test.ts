import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * A pane's program is a terminal, and nothing else about it is a session.
 *
 * It needs the same machinery a session's terminal needs — bytes in and out, a
 * resize, a kill — so it lives in the same maps. What it must never be is
 * listed: no window is told it exists, nothing persists it, and nothing walking
 * a project's sessions should find one and start settling extensions onto it.
 */

vi.mock('../packages/server/src/vornd-sessions', async () =>
  (await import('./helpers/fake-vornd-pty')).vorndModule()
)

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const { ptyManager } = await import('../packages/server/src/pty-manager')
const { fakeVornd } = await import('./helpers/fake-vornd-pty')

// Only what this file made: the manager is a singleton, and sweeping it would
// take another suite's terminals with it.
const opened: string[] = []

beforeEach(() => {
  fakeVornd.reset()
})

afterEach(() => {
  while (opened.length > 0) {
    try {
      ptyManager.killPty(opened.pop() as string)
    } catch {
      /* already gone */
    }
  }
})

function pane(over: { args?: string[]; env?: Record<string, string> } = {}) {
  const session = ptyManager.createExtensionPty({
    command: 'lazygit',
    args: over.args ?? [],
    cwd: process.cwd(),
    displayName: 'Git',
    env: over.env ?? {}
  })
  opened.push(session.id)
  return session
}

describe('a pane program', () => {
  it('is not one of the sessions the app is told about', () => {
    const drawn = pane({ env: { VORN_EXTENSION_TOKEN: 'the-token' } })

    expect(ptyManager.isExtensionPty(drawn.id)).toBe(true)
    expect(ptyManager.getActiveSessions().map((s) => s.id)).not.toContain(drawn.id)
    expect(ptyManager.getLiveSessions().map((s) => s.id)).not.toContain(drawn.id)
  })

  it('still answers to everything a terminal has to answer to', async () => {
    const drawn = pane()
    const program = fakeVornd.last()

    ptyManager.writeToPty(drawn.id, 'q')
    expect(program.written).toEqual(['q'])
    ptyManager.resizePty(drawn.id, 100, 30)
    expect(drawn).toMatchObject({ cols: 100, rows: 30 })
    expect(ptyManager.hasLivePty(drawn.id)).toBe(true)

    ptyManager.killPty(drawn.id)
    expect(ptyManager.isExtensionPty(drawn.id)).toBe(false)
    await new Promise((r) => setImmediate(r))
    expect(program.kills).toEqual(['SIGHUP'])
  })

  it('stops being an extension pane when its program ends', () => {
    const drawn = pane()

    fakeVornd.last().exit(0)

    expect(ptyManager.isExtensionPty(drawn.id)).toBe(false)
    expect(ptyManager.hasLivePty(drawn.id)).toBe(false)
  })

  it('is spawned with what the extension gave it', () => {
    pane({ args: ['--path', '.'], env: { VORN_EXTENSION_TOKEN: 'the-token' } })

    const { id, spec, watched } = fakeVornd.last()
    expect(spec?.argv).toEqual(['lazygit', '--path', '.'])
    expect(spec?.cwd).toBe(process.cwd())
    expect(spec?.env).toMatchObject({ VORN_EXTENSION_TOKEN: 'the-token', VORN_SESSION_ID: id })
    expect(watched).toBe(false)
  })
})
