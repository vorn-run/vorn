// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { TerminalState } from '../src/renderer/stores/types'
import type { AgentStatus } from '../src/shared/types'

vi.mock('../src/renderer/components/AgentIcon', () => ({
  AgentIcon: () => <div data-testid="agent-icon" />
}))

const toggleMinimized = vi.fn()
const setActiveTabId = vi.fn()
let terminal: TerminalState | undefined
let editorPanes = new Map<string, { filePath: string; sessionId: string }>()
let browserPanes = new Map<string, { tabs: string[]; activeTab: number; sessionId: string }>()
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) => {
    const state = {
      terminals: { get: () => terminal },
      editorPanes,
      browserPanes,
      toggleMinimized,
      setActiveTabId
    }
    return selector ? selector(state) : state
  }
}))

import { MinimizedPill } from '../src/renderer/components/MinimizedPill'

function term(status: AgentStatus): TerminalState {
  return {
    id: 't1',
    session: {
      id: 't1',
      agentType: 'claude',
      projectName: 'p',
      projectPath: '/p',
      status,
      createdAt: Date.now()
    },
    status,
    lastOutputTimestamp: Date.now()
  } as unknown as TerminalState
}

describe('MinimizedPill status dot', () => {
  it('animates the dot only when the agent is running', () => {
    terminal = term('running')
    const { container } = render(<MinimizedPill terminalId="t1" />)
    const dot = container.querySelector('span.rounded-full')
    expect(dot).not.toBeNull()
    expect(dot?.className).toContain('animate-pulse')
  })

  it('leaves the dot static for non-running statuses', () => {
    for (const s of ['idle', 'waiting', 'error'] as const) {
      terminal = term(s)
      const { container, unmount } = render(<MinimizedPill terminalId="t1" />)
      const dot = container.querySelector('span.rounded-full')
      expect(dot?.className).not.toContain('animate-pulse')
      unmount()
    }
  })
})

/**
 * A minimized card is not a minimized session, and the pill has to say so. Both
 * sit in the same dock row, so borrowed chrome is not a cosmetic slip — it is
 * the pill claiming a state the thing does not have.
 */
describe('MinimizedPill for a popped-out card', () => {
  beforeEach(() => {
    terminal = term('running')
    editorPanes = new Map()
    browserPanes = new Map()
  })

  it("wears the file's own name, and no agent or status chrome", () => {
    editorPanes.set('card:t1:0', { filePath: '/repo/server.ts', sessionId: 't1' })
    const { container, queryByTestId, getByText } = render(<MinimizedPill terminalId="card:t1:0" />)

    getByText('server.ts')
    // A file has no agent and is never "running": showing either would be the
    // pill reporting on the session while naming the file.
    expect(queryByTestId('agent-icon')).toBeNull()
    expect(container.querySelector('span.rounded-full.animate-pulse')).toBeNull()
  })

  it('names a popped-out page by its host', () => {
    browserPanes.set('card:t1:1', {
      tabs: ['https://vorn.dev/docs'],
      activeTab: 0,
      sessionId: 't1'
    })
    const { getByText, queryByTestId } = render(<MinimizedPill terminalId="card:t1:1" />)

    getByText('vorn.dev')
    expect(queryByTestId('agent-icon')).toBeNull()
  })

  it('keeps the agent chrome for a session, which does have a status', () => {
    const { getByTestId, container } = render(<MinimizedPill terminalId="t1" />)
    getByTestId('agent-icon')
    expect(container.querySelector('span.rounded-full')).not.toBeNull()
  })

  it('restores the card and hands focus to the session it came from', () => {
    editorPanes.set('card:t1:0', { filePath: '/repo/server.ts', sessionId: 't1' })
    const { getByRole } = render(<MinimizedPill terminalId="card:t1:0" />)

    fireEvent.click(getByRole('button'))
    expect(toggleMinimized).toHaveBeenCalledWith('card:t1:0')
    // Not the card id: the tab strip holds sessions only, so focusing the card
    // there would select a tab that does not exist.
    expect(setActiveTabId).toHaveBeenCalledWith('t1')
  })
})
