// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import type { TerminalSession } from '../packages/shared/src/types'

Object.defineProperty(window, 'api', {
  value: { notifyWidgetStatus: vi.fn() },
  writable: true
})

import { create } from 'zustand'
import { createTerminalsSlice } from '../src/renderer/stores/terminals-slice'
import type { TerminalsSlice } from '../src/renderer/stores/types'
import { staleRev, takesCreated } from '../src/renderer/lib/session-rev'

/**
 * One session heard twice: as the answer to the window's own create and as the
 * `session:created` broadcast every client gets, in either order. The window
 * holds one session, at the newer revision.
 */

function session(rev: number | undefined, displayName: string): TerminalSession {
  return {
    id: 'pane-1',
    agentType: 'claude',
    projectName: 'p',
    projectPath: '/p',
    status: 'running',
    createdAt: 1,
    pid: 9,
    displayName,
    ...(rev === undefined ? {} : { rev })
  }
}

function createStore() {
  return create<TerminalsSlice>()((...a) => ({
    ...createTerminalsSlice(...(a as Parameters<typeof createTerminalsSlice>)),
    terminalOrder: [] as string[],
    minimizedTerminals: new Set<string>(),
    gitDiffStats: new Map()
  }))
}

type Store = ReturnType<typeof createStore>

/** The order the board draws its sessions in, which the slice keeps beside them. */
function order(store: Store): string[] {
  return (store.getState() as unknown as { terminalOrder: string[] }).terminalOrder
}

/** What a create's caller does with the answer. */
function reply(store: Store, s: TerminalSession): void {
  store.getState().addTerminal(s)
}

/** What App.tsx does with `session:created`. */
function broadcast(store: Store, s: TerminalSession): void {
  const held = store.getState().terminals.get(s.id)
  if (takesCreated(held?.session, s)) store.getState().addTerminal(s)
}

function held(store: Store): Array<[string, string | undefined, number | undefined]> {
  return [...store.getState().terminals.values()].map((t) => [
    t.id,
    t.session.displayName,
    t.session.rev
  ])
}

describe('a session heard as a reply and as a broadcast', () => {
  it('reply then broadcast gives one session', () => {
    const store = createStore()
    reply(store, session(2, 'reply'))
    broadcast(store, session(2, 'broadcast'))
    expect(held(store)).toEqual([['pane-1', 'reply', 2]])
    expect(order(store)).toEqual(['pane-1'])
  })

  it('broadcast then reply gives one session', () => {
    const store = createStore()
    broadcast(store, session(2, 'broadcast'))
    reply(store, session(2, 'reply'))
    expect(held(store)).toEqual([['pane-1', 'broadcast', 2]])
    expect(order(store)).toEqual(['pane-1'])
  })

  it('keeps the newer revision whichever arrives last', () => {
    const store = createStore()
    broadcast(store, session(5, 'renamed since'))
    reply(store, session(3, 'as created'))
    expect(held(store)).toEqual([['pane-1', 'renamed since', 5]])

    const other = createStore()
    reply(other, session(3, 'as created'))
    broadcast(other, session(5, 'renamed since'))
    expect(held(other)).toEqual([['pane-1', 'renamed since', 5]])
  })

  it('takes records without a revision as it always did', () => {
    const store = createStore()
    reply(store, session(undefined, 'reply'))
    broadcast(store, session(undefined, 'broadcast'))
    expect(held(store)).toEqual([['pane-1', 'reply', undefined]])
    reply(store, session(undefined, 'again'))
    expect(held(store)).toEqual([['pane-1', 'again', undefined]])
  })

  it('drops an update older than the record held, and moves the revision on with a newer one', () => {
    const store = createStore()
    reply(store, session(4, 'reply'))
    expect(staleRev(store.getState().terminals.get('pane-1')?.session, session(3, 'old'))).toBe(
      true
    )
    expect(staleRev(store.getState().terminals.get('pane-1')?.session, session(5, 'new'))).toBe(
      false
    )
    store.getState().noteSessionRev('pane-1', 5)
    store.getState().noteSessionRev('pane-1', 4)
    expect(store.getState().terminals.get('pane-1')?.session.rev).toBe(5)
  })
})
