// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { CreateTerminalPayload, TerminalSession } from '../packages/shared/src/types'

const createTerminal = vi.fn()

Object.defineProperty(window, 'api', {
  value: {
    createTerminal: (...args: unknown[]) => createTerminal(...args),
    notifyWidgetStatus: vi.fn()
  },
  writable: true
})

vi.mock('../src/renderer/components/Toast', () => ({
  toast: { error: vi.fn(), success: vi.fn() }
}))

import { launchAgentFromShell } from '../src/renderer/lib/session-utils'
import { useAppStore } from '../src/renderer/stores'

function shell(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'term-1',
    agentType: 'shell',
    projectName: 'vorn',
    projectPath: '/home/j/vorn',
    status: 'running',
    createdAt: 0,
    pid: 1,
    ...over
  }
}

function lastPayload(): CreateTerminalPayload {
  return createTerminal.mock.calls.at(-1)?.[0] as CreateTerminalPayload
}

beforeEach(() => {
  createTerminal.mockReset()
  createTerminal.mockResolvedValue(shell({ id: 'agent-1', agentType: 'claude' }))
  useAppStore.setState({ terminals: new Map(), terminalOrder: [] })
})

describe('launchAgentFromShell', () => {
  it('starts the agent in the shell’s current directory', async () => {
    // A `cd` before the prompt must be honoured, which is the whole point of
    // tracking shellCwd.
    await launchAgentFromShell(
      shell({ shellCwd: '/home/j/vorn/packages/server' }),
      'claude',
      'fix the shim'
    )
    expect(lastPayload().projectPath).toBe('/home/j/vorn/packages/server')
    expect(lastPayload().initialPrompt).toBe('fix the shim')
    expect(lastPayload().agentType).toBe('claude')
  })

  it('falls back to the worktree, then the project path', async () => {
    await launchAgentFromShell(shell({ worktreePath: '/home/j/wt/feat' }), 'codex', 'go')
    expect(lastPayload().projectPath).toBe('/home/j/wt/feat')

    await launchAgentFromShell(shell(), 'codex', 'go')
    expect(lastPayload().projectPath).toBe('/home/j/vorn')
  })

  it('never sends branch or worktree flags', async () => {
    // createPty checks out payload.branch against the project when no
    // worktree is supplied, which would move the user's checkout under them.
    await launchAgentFromShell(
      shell({ branch: 'feat/x', worktreePath: '/home/j/wt/feat', isWorktree: true }),
      'claude',
      'go'
    )
    const payload = lastPayload()
    expect(payload.branch).toBeUndefined()
    expect(payload.useWorktree).toBeUndefined()
    expect(payload.existingWorktreePath).toBeUndefined()
  })

  it('adds the new session to the store and makes it active', async () => {
    await launchAgentFromShell(shell(), 'claude', 'go')
    const state = useAppStore.getState()
    expect(state.terminals.has('agent-1')).toBe(true)
    expect(state.activeTabId).toBe('agent-1')
  })

  it('does not throw when the launch fails', async () => {
    createTerminal.mockRejectedValue(new Error('spawn failed'))
    await expect(launchAgentFromShell(shell(), 'claude', 'go')).resolves.toBeUndefined()
    expect(useAppStore.getState().terminals.size).toBe(0)
  })
})
