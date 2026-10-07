import { describe, it, expect, vi, beforeEach } from 'vitest'

/**
 * Which terminal an agent's hook is linked to when the server starts the
 * sessions itself, as it does for a create vornd forwards. Two agents in one folder used to
 * be told apart by the folder alone, so the older one's events went to the
 * newer terminal. An exact identity (the terminal id the launch gave the agent,
 * or the conversation id it was started with) now decides first.
 */

const log = vi.hoisted(() => ({ info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }))
vi.mock('../packages/server/src/logger', () => ({ default: log }))
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

import type { AgentType, HookEvent, TerminalSession } from '@vornrun/shared/types'
import { ptyManager } from '../packages/server/src/pty-manager'
import { hookStatusMapper } from '../packages/server/src/hook-status-mapper'
import { fakeVornd, toldHookLink } from './helpers/fake-vornd-pty'

let folders = 0

/** A folder no other test uses, so earlier terminals never compete. */
function folder(): string {
  folders += 1
  return `/tmp/vorn-hook-linking-${process.pid}-${folders}`
}

async function launch(
  projectPath: string,
  agentType: AgentType = 'claude'
): Promise<TerminalSession> {
  const session = await ptyManager.createPty({
    agentType,
    projectName: 'p',
    projectPath
  } as never)
  // Launched a moment apart, so "most recently launched" is unambiguous.
  await new Promise((resolve) => setTimeout(resolve, 5))
  return session
}

function event(name: string, sessionId: string, cwd: string, terminalId?: string): HookEvent {
  return {
    hook_event_name: name,
    session_id: sessionId,
    cwd,
    ...(terminalId ? { vorn_terminal_id: terminalId } : {})
  }
}

beforeEach(() => {
  fakeVornd.reset()
  hookStatusMapper.clear()
  log.warn.mockClear()
})

describe('two agents in one folder', () => {
  it('links each conversation to the terminal that started it, not the newest', async () => {
    const cwd = folder()
    const older = await launch(cwd)
    const newer = await launch(cwd)
    expect(older.agentSessionId).toBeTruthy()
    expect(newer.agentSessionId).toBeTruthy()

    // The older agent speaks first: the folder alone sent it to the newer terminal.
    expect(
      hookStatusMapper.mapEventToStatus(event('SessionStart', older.agentSessionId!, cwd))
    ).toEqual({ terminalId: older.id, status: 'running' })
    expect(
      hookStatusMapper.mapEventToStatus(event('SessionStart', newer.agentSessionId!, cwd))
    ).toEqual({ terminalId: newer.id, status: 'running' })

    expect(toldHookLink(older.id)).toBe(older.agentSessionId)
    expect(toldHookLink(newer.id)).toBe(newer.agentSessionId)
    expect(log.warn).not.toHaveBeenCalled()
  })

  it('links by the terminal id a hook carries, before its conversation or folder', async () => {
    const cwd = folder()
    const older = await launch(cwd)
    await launch(cwd)

    // A conversation neither was started with, as after /clear in the older one.
    const result = hookStatusMapper.mapEventToStatus(event('PreToolUse', 'cleared', cwd, older.id))
    expect(result).toEqual({ terminalId: older.id, status: 'running' })
    expect(hookStatusMapper.resolveTerminal(event('Stop', 'cleared', cwd))).toBe(older.id)
  })

  it('moves a terminal on to its new conversation when it starts another', async () => {
    const cwd = folder()
    const term = await launch(cwd)
    hookStatusMapper.mapEventToStatus(event('SessionStart', term.agentSessionId!, cwd))

    expect(hookStatusMapper.resolveTerminal(event('SessionStart', 'next', cwd, term.id))).toBe(
      term.id
    )
    expect(hookStatusMapper.getLinkedTerminal(term.agentSessionId!)).toBeUndefined()
    expect(toldHookLink(term.id)).toBe('next')
  })

  it('corrects a link the folder guessed once an exact identity arrives', async () => {
    const cwd = folder()
    const older = await launch(cwd)
    const newer = await launch(cwd)

    // Nothing exact: the newest is guessed, and the guess is logged as one.
    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'mystery', cwd))).toBe(newer.id)
    expect(log.warn).toHaveBeenCalledTimes(1)
    expect(String(log.warn.mock.calls[0][0])).toContain(older.id)

    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'mystery', cwd, older.id))).toBe(
      older.id
    )
    // The newer terminal is free again for its own conversation.
    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'own', cwd, newer.id))).toBe(
      newer.id
    )
    expect(hookStatusMapper.getLinkedTerminal('mystery')).toBe(older.id)
  })

  it('falls back to the folder when the terminal id names no live terminal', async () => {
    const cwd = folder()
    const gemini = await launch(cwd, 'gemini')

    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'x', cwd, 'gone'))).toBe(gemini.id)
    expect(log.warn).not.toHaveBeenCalled()
  })

  it('links nothing when nothing exact exists and no terminal is in the folder', () => {
    expect(hookStatusMapper.resolveTerminal(event('PreToolUse', 'x', folder()))).toBeUndefined()
  })

  it('does not tell the record again for a link it already holds', async () => {
    const cwd = folder()
    const term = await launch(cwd)
    const linkHookSession = vi.spyOn(ptyManager, 'linkHookSession')
    try {
      for (const name of ['SessionStart', 'PreToolUse', 'Stop']) {
        hookStatusMapper.resolveTerminal(event(name, term.agentSessionId!, cwd, term.id))
      }
      expect(linkHookSession).toHaveBeenCalledTimes(1)
    } finally {
      linkHookSession.mockRestore()
    }
  })
})
