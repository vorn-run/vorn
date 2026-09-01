// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession } from '../packages/shared/src/types'

const resumeSession = vi.fn()
Object.defineProperty(window, 'api', {
  value: { resumeSession, notifyWidgetStatus: vi.fn() },
  writable: true
})

vi.mock('../src/renderer/components/Toast', () => {
  const toast = Object.assign(vi.fn(), { error: vi.fn() })
  return { toast }
})

import { useAppStore } from '../src/renderer/stores'
import { resumeEndedSession } from '../src/renderer/lib/session-resume'
import { toast } from '../src/renderer/components/Toast'

/**
 * Resuming a pane whose conversation is already open somewhere else.
 *
 * The server hands back the session writing it rather than starting a second
 * agent. What the board must not do with that answer is key two panes by one id:
 * they draw the same terminal twice, and closing either closes both.
 */

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'cold',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'running',
    createdAt: 1,
    ...over
  } as TerminalSession
}

beforeEach(() => {
  vi.clearAllMocks()
  useAppStore.setState({ terminals: new Map(), terminalOrder: [] })
})

describe('a resume bound to a session already on the board', () => {
  beforeEach(() => {
    const store = useAppStore.getState()
    store.addTerminal(session({ id: 'running', agentSessionId: 'transcript-a' }))
    store.addTerminal(session({ id: 'cold', agentSessionId: 'transcript-a' }), {
      reason: 'app-closed',
      at: 1,
      replayed: true
    })
    resumeSession.mockResolvedValue({
      ok: true,
      session: session({ id: 'running', agentSessionId: 'transcript-a' }),
      boundTo: 'running'
    })
  })

  it('leaves one slot per session, not two under one id', async () => {
    await resumeEndedSession('cold')
    const { terminalOrder } = useAppStore.getState()
    expect(terminalOrder).toEqual(['running'])
  })

  it('lets the ended pane go and brings the running one forward', async () => {
    await resumeEndedSession('cold')
    const state = useAppStore.getState()
    expect(state.terminals.has('cold')).toBe(false)
    expect(state.focusedTerminalId).toBe('running')
  })

  it('says so, because nobody asked for the pane to change what it shows', async () => {
    await resumeEndedSession('cold')
    expect(toast).toHaveBeenCalledWith('That conversation was already running. This pane shows it.')
  })
})

describe('a resume bound to a session this client has never drawn', () => {
  it('adopts it into the slot the ended pane was in', async () => {
    const store = useAppStore.getState()
    store.addTerminal(session({ id: 'cold' }), { reason: 'app-closed', at: 1, replayed: true })
    resumeSession.mockResolvedValue({
      ok: true,
      session: session({ id: 'elsewhere' }),
      boundTo: 'elsewhere'
    })

    await resumeEndedSession('cold')

    const state = useAppStore.getState()
    expect(state.terminalOrder).toEqual(['elsewhere'])
    expect(state.terminals.has('cold')).toBe(false)
  })
})

describe('an ordinary resume', () => {
  it('keeps replacing in place, with the same id', async () => {
    const store = useAppStore.getState()
    store.addTerminal(session({ id: 'cold' }), { reason: 'app-closed', at: 1, replayed: true })
    resumeSession.mockResolvedValue({ ok: true, session: session({ id: 'cold' }) })

    await resumeEndedSession('cold')

    expect(useAppStore.getState().terminalOrder).toEqual(['cold'])
    expect(toast).not.toHaveBeenCalled()
  })
})
