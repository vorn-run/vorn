import { describe, it, expect, vi, beforeEach } from 'vitest'

const mockFindUnlinked = vi.fn()
const mockGetActiveSessions = vi.fn(() => [])
const mockLinkHookSession = vi.fn()

vi.mock('../packages/server/src/pty-manager', () => ({
  ptyManager: {
    findUnlinkedSessionsByCwd: (...args: unknown[]) => {
      const found = mockFindUnlinked(...args)
      return found ? [found] : []
    },
    getActiveSessions: () => mockGetActiveSessions(),
    linkHookSession: (id: string, hookSessionId: string) => mockLinkHookSession(id, hookSessionId)
  }
}))
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { hookStatusMapper } from '../packages/server/src/hook-status-mapper'
import type { HookEvent } from '@vornrun/shared/types'

function makeEvent(name: string, sessionId = 'sess-1', cwd = '/project'): HookEvent {
  return {
    hook_event_name: name,
    session_id: sessionId,
    cwd
  } as HookEvent
}

beforeEach(() => {
  vi.clearAllMocks()
  hookStatusMapper.clear()
})

describe('forceLink + getLinkedTerminal', () => {
  it('creates and retrieves session -> terminal mapping', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    expect(hookStatusMapper.getLinkedTerminal('sess-1')).toBe('term-1')
  })

  it('returns undefined for unknown session', () => {
    expect(hookStatusMapper.getLinkedTerminal('unknown')).toBeUndefined()
  })
})

describe('tryLink', () => {
  it('links session to terminal found by cwd', () => {
    const session = { id: 'term-1', hookSessionId: undefined, statusSource: undefined }
    mockFindUnlinked.mockReturnValueOnce(session)
    const result = hookStatusMapper.tryLink('sess-1', '/project')
    expect(result).toBe('term-1')
    // Through the pty manager, which sets it or has vornd set it.
    expect(mockLinkHookSession).toHaveBeenCalledWith('term-1', 'sess-1')
    // statusSource is set by promoteToHookStatus(), not tryLink()
    expect(session.statusSource).toBeUndefined()
  })

  it('returns cached mapping if already linked', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.tryLink('sess-1', '/project')
    expect(result).toBe('term-1')
    expect(mockFindUnlinked).not.toHaveBeenCalled()
  })

  it('returns undefined when no unlinked terminal found', () => {
    mockFindUnlinked.mockReturnValueOnce(undefined)
    expect(hookStatusMapper.tryLink('sess-1', '/project')).toBeUndefined()
  })
})

describe('mapEventToStatus', () => {
  it('SessionStart -> running (and triggers tryLink)', () => {
    const session = { id: 'term-1', hookSessionId: undefined, statusSource: undefined }
    mockFindUnlinked.mockReturnValueOnce(session)
    const result = hookStatusMapper.mapEventToStatus(makeEvent('SessionStart'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'running' })
  })

  it('PreToolUse -> running', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('PreToolUse'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'running' })
  })

  it('PostToolUse -> running', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('PostToolUse'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'running' })
  })

  it('PostToolUseFailure -> error', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('PostToolUseFailure'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'error' })
  })

  it('Notification -> waiting', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('Notification'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'waiting' })
  })

  it('PermissionRequest -> waiting', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('PermissionRequest'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'waiting' })
  })

  it('Stop -> idle', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('Stop'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'idle' })
  })

  it('SessionEnd -> idle and removes session', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('SessionEnd'))
    expect(result).toEqual({ terminalId: 'term-1', status: 'idle' })
    expect(hookStatusMapper.getLinkedTerminal('sess-1')).toBeUndefined()
  })

  it('unknown event -> null', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    const result = hookStatusMapper.mapEventToStatus(makeEvent('UnknownEvent'))
    expect(result).toBeNull()
  })

  it('unlinked session -> null', () => {
    mockFindUnlinked.mockReturnValueOnce(undefined)
    const result = hookStatusMapper.mapEventToStatus(makeEvent('PreToolUse', 'unknown-sess'))
    expect(result).toBeNull()
  })
})

describe('removeSession + clear', () => {
  it('removeSession removes specific mapping', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    hookStatusMapper.removeSession('sess-1')
    expect(hookStatusMapper.getLinkedTerminal('sess-1')).toBeUndefined()
  })

  it('clear removes all mappings', () => {
    hookStatusMapper.forceLink('sess-1', 'term-1')
    hookStatusMapper.forceLink('sess-2', 'term-2')
    hookStatusMapper.clear()
    expect(hookStatusMapper.getLinkedTerminal('sess-1')).toBeUndefined()
    expect(hookStatusMapper.getLinkedTerminal('sess-2')).toBeUndefined()
  })
})
