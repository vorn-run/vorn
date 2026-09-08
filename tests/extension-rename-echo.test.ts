// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import type { TerminalSession } from '../packages/shared/src/types'

const renameSession = vi.fn()

const session = (id: string, displayName: string): TerminalSession =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    status: 'idle',
    createdAt: 0,
    displayName
  }) as TerminalSession

/**
 * An extension granted `card.rename` names the card through the host, which
 * broadcasts the session the same way any other change to it is broadcast. The
 * window applies that name through `renameTerminal` — the one path a rename
 * takes — and the guard in it is what stops the round trip from starting again.
 */
describe('a card renamed by an extension', () => {
  beforeEach(() => {
    renameSession.mockClear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      notifyWidgetStatus: vi.fn(),
      renameSession
    }
    act(() => {
      useAppStore.setState({ terminals: new Map(), terminalOrder: [] })
    })
    act(() => useAppStore.getState().addTerminal(session('t1', 'workflow')))
    renameSession.mockClear()
  })

  it('takes the name the host sent, without sending it back', () => {
    act(() => useAppStore.getState().renameTerminal('t1', 'Named by the extension'))

    expect(useAppStore.getState().terminals.get('t1')?.session.displayName).toBe(
      'Named by the extension'
    )
    // The renderer's own write does go to the host -- this is the same action a
    // person's rename uses. What must not happen is a second one.
    expect(renameSession).toHaveBeenCalledTimes(1)

    // The broadcast arriving back carries the name the store already holds.
    act(() => useAppStore.getState().renameTerminal('t1', 'Named by the extension'))
    expect(renameSession).toHaveBeenCalledTimes(1)
  })

  it('leaves a card the host has nothing new to say about untouched', () => {
    const before = useAppStore.getState().terminals
    act(() => useAppStore.getState().renameTerminal('t1', 'workflow'))
    expect(useAppStore.getState().terminals).toBe(before)
    expect(renameSession).not.toHaveBeenCalled()
  })
})
