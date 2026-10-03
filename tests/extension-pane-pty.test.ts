import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * A pane's program is a PTY, and nothing else about it is a session.
 *
 * It needs the same machinery a session's terminal needs — bytes in and out, a
 * resize, a kill — so it lives in the same maps. What it must never be is
 * listed: no window is told it exists, nothing persists it, and nothing walking
 * a project's sessions should find one and start settling extensions onto it.
 */

const { spawnMock } = vi.hoisted(() => {
  class FakePty {
    pid = 4242
    write(): void {}
    kill(): void {}
    resize(): void {}
    onData(): { dispose: () => void } {
      return { dispose: () => {} }
    }
    onExit(): { dispose: () => void } {
      return { dispose: () => {} }
    }
  }
  return { spawnMock: vi.fn(() => new FakePty()) }
})

// The nested copy by name: `packages/server` pins its own, and mocking the bare
// specifier would patch the root one and leave the spawns real.
vi.mock('../packages/server/node_modules/node-pty', () => ({
  spawn: spawnMock,
  default: { spawn: spawnMock }
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const { ptyManager } = await import('../packages/server/src/pty-manager')

// Only what this file made: the manager is a singleton, and sweeping it would
// take another suite's terminals with it.
const opened: string[] = []

beforeEach(() => {
  spawnMock.mockClear()
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

  it('still answers to everything a terminal has to answer to', () => {
    const drawn = pane()

    expect(() => ptyManager.writeToPty(drawn.id, 'q')).not.toThrow()
    expect(ptyManager.hasLivePty(drawn.id)).toBe(true)
    expect(() => ptyManager.killPty(drawn.id)).not.toThrow()
    expect(ptyManager.isExtensionPty(drawn.id)).toBe(false)
  })

  it('is spawned with what the extension gave it', () => {
    pane({ args: ['--path', '.'], env: { VORN_EXTENSION_TOKEN: 'the-token' } })

    const [command, args, options] = spawnMock.mock.calls.at(-1) as unknown as [
      string,
      string[],
      { env: Record<string, string> }
    ]
    expect(command).toBe('lazygit')
    expect(args).toEqual(['--path', '.'])
    expect(options.env.VORN_EXTENSION_TOKEN).toBe('the-token')
  })
})
