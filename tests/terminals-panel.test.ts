// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import { claimedTerminalIds } from '../src/renderer/hooks/usePanelTerminals'
import { activePanelTerminalId } from '../src/renderer/stores/types'

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

function seed(ids: string[]): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, { id, session: session(id), status: 'idle', lastOutputTimestamp: 1 })
  }
  act(() => {
    useAppStore.setState({
      terminals: terminals as never,
      terminalOrder: ids,
      terminalsPanes: new Map(),
      minimizedTerminals: new Set(),
      maximizedPaneId: null,
      focusedTerminalId: null,
      previewTerminalId: null,
      selectedTerminalId: null,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      devicePanes: new Map()
    })
  })
}

/**
 * A session can hold several shells beside its agent. They are sessions in
 * their own right — the panel only records which ones it has claimed, and
 * letting go of that claim is the whole of "extract this terminal".
 */
describe('the terminals panel', () => {
  const s = () => useAppStore.getState()

  beforeEach(() => {
    localStorage.clear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn()
    }
    seed(['owner', 'sh1', 'sh2', 'other'])
  })

  it('holds shells in order, with the newest in front', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })
    const pane = s().terminalsPanes.get('owner')!

    expect(pane.terminals).toEqual(['sh1', 'sh2'])
    expect(activePanelTerminalId(pane)).toBe('sh2')
  })

  it('shows a shell it already holds rather than listing it twice', () => {
    // Two tabs for one terminal would be two slots claiming one id, and the
    // registry is last-writer-wins — one of them would simply go blank.
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
      s().openTerminalsPane('owner', 'sh1')
    })
    const pane = s().terminalsPanes.get('owner')!

    expect(pane.terminals).toEqual(['sh1', 'sh2'])
    expect(activePanelTerminalId(pane)).toBe('sh1')
  })

  it('hides every claimed shell, and only those', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })

    const claimed = claimedTerminalIds({ terminalsPanes: s().terminalsPanes })
    expect([...claimed].sort()).toEqual(['sh1', 'sh2'])
    expect(claimed.has('owner')).toBe(false)
    expect(claimed.has('other')).toBe(false)
  })

  it('lets go of a shell on extraction, keeping the rest', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })

    act(() => s().extractPanelTerminal('owner', 'sh1'))
    const pane = s().terminalsPanes.get('owner')!

    // The terminal itself is untouched — it was a session all along, and is now
    // simply no longer claimed.
    expect(pane.terminals).toEqual(['sh2'])
    expect(s().terminals.has('sh1')).toBe(true)
    expect(claimedTerminalIds({ terminalsPanes: s().terminalsPanes }).has('sh1')).toBe(false)
  })

  it('lands on a neighbour when the shell in front is extracted', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
      s().setActivePanelTerminal('owner', 1)
    })

    act(() => s().extractPanelTerminal('owner', 'sh2'))
    expect(activePanelTerminalId(s().terminalsPanes.get('owner')!)).toBe('sh1')
  })

  it('closes the panel when its last shell leaves', () => {
    // A panel with no shells is a box taking up a pane — the same rule the
    // browser's last tab follows.
    act(() => s().openTerminalsPane('owner', 'sh1'))

    act(() => s().extractPanelTerminal('owner', 'sh1'))
    expect(s().terminalsPanes.has('owner')).toBe(false)
  })

  it('lets go of a shell closed from its own tab', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })

    act(() => s().removeTerminal('sh1'))

    // Closing one is the other direction from closing the session: the panel
    // survives and has to release the claim. Left behind, the tab outlives the
    // terminal — falling back to its raw id for a name, with nothing to draw.
    const pane = s().terminalsPanes.get('owner')!
    expect(pane.terminals).toEqual(['sh2'])
    expect(activePanelTerminalId(pane)).toBe('sh2')
    expect(s().terminals.has('sh1')).toBe(false)
  })

  it('closes the panel when its last shell is closed from its tab', () => {
    act(() => s().openTerminalsPane('owner', 'sh1'))

    act(() => s().removeTerminal('sh1'))
    expect(s().terminalsPanes.has('owner')).toBe(false)
    // And nothing may still be pointing at the pane that just went.
    expect(s().maximizedPaneId).toBeNull()
  })

  it('releases a maximized panel when its last shell is closed', () => {
    act(() => s().openTerminalsPane('owner', 'sh1'))
    act(() => s().setMaximizedPane('terminals:owner'))

    act(() => s().removeTerminal('sh1'))
    // A maximize pointing at a pane that no longer draws hides every sibling
    // behind nothing at all.
    expect(s().maximizedPaneId).toBeNull()
  })

  it('drops a shell that never came back, and keeps its panel for the rest', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })
    // sh1 is gone from the live set — the restart-shaped version of the same
    // bug, where there is no removal to hang the cleanup off.
    act(() =>
      useAppStore.setState({
        terminals: new Map([...s().terminals].filter(([id]) => id !== 'sh1')) as never,
        knownSessionIds: new Set([...s().terminals.keys()].filter((id) => id !== 'sh1'))
      })
    )

    act(() => s().setVisibleTerminalIds(['owner']))
    expect(s().terminalsPanes.get('owner')?.terminals).toEqual(['sh2'])
  })

  it('keeps a shell taken out after the launch sync', () => {
    // A shell opened now is a session the sync never heard of. Pruning against
    // that answer alone dropped it straight back out of the panel it was just
    // added to, which reads as the panel refusing to open a shell at all.
    act(() =>
      useAppStore.setState({
        knownSessionIds: new Set(['owner']) as never
      })
    )
    act(() => s().openTerminalsPane('owner', 'sh1'))
    act(() => s().setVisibleTerminalIds(['owner']))
    expect(s().terminalsPanes.get('owner')?.terminals).toEqual(['sh1'])
  })

  it('drops a panel whose shells all failed to come back', () => {
    act(() => s().openTerminalsPane('owner', 'sh1'))
    act(() =>
      useAppStore.setState({
        terminals: new Map([...s().terminals].filter(([id]) => id !== 'sh1')) as never,
        knownSessionIds: new Set([...s().terminals.keys()].filter((id) => id !== 'sh1'))
      })
    )

    act(() => s().setVisibleTerminalIds(['owner']))
    expect(s().terminalsPanes.has('owner')).toBe(false)
  })

  it('takes its shells with it when the panel is closed', async () => {
    const kill = vi.fn().mockResolvedValue(undefined)
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api: Record<string, unknown> }).api,
      killTerminal: kill
    }
    const { closeTerminalsPanel } = await import('../src/renderer/lib/terminal-close')
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })

    await act(async () => {
      await closeTerminalsPanel('owner')
    })

    // Dropping the claim alone would leave two live ptys loose on the grid as
    // top-level cards, and the control that made them would offer to make a
    // third.
    expect(s().terminalsPanes.has('owner')).toBe(false)
    expect(s().terminals.has('sh1')).toBe(false)
    expect(s().terminals.has('sh2')).toBe(false)
    expect(kill.mock.calls.map((c) => c[0]).sort()).toEqual(['sh1', 'sh2'])
    // The session that owned the panel is untouched — only its shells went.
    expect(s().terminals.has('owner')).toBe(true)
  })

  it('reads a corrupted active tab as the first one', async () => {
    const { parsePersistedPanels } = await import('../src/renderer/stores/ui-slice')
    // `?? 0` only catches null and undefined. A string or a NaN here reaches
    // the card as an index, which looks up undefined and is dereferenced.
    const restored = parsePersistedPanels({
      owner: { terminals: ['sh1'], activeTab: 'nope' as unknown as number }
    })
    expect(restored.get('owner')?.activeTab).toBe(0)
  })

  it('takes its shells down with the session, and forgets the panel', () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
    })

    act(() => s().removeTerminal('owner'))

    // The shells are that session's, so they go with it — and nothing may be
    // left claiming ids that no longer exist, or those terminals would be
    // hidden from every surface with nothing left to un-hide them.
    expect(s().terminalsPanes.has('owner')).toBe(false)
    expect(s().terminals.has('sh1')).toBe(false)
    expect(s().terminals.has('sh2')).toBe(false)
    expect(s().terminalOrder).toEqual(['other'])
  })

  it('releases focus and selection pointing at a shell that went with its session', () => {
    act(() => s().openTerminalsPane('owner', 'sh1'))
    act(() =>
      useAppStore.setState({ focusedTerminalId: 'sh1', selectedTerminalId: 'sh1' } as never)
    )

    act(() => s().removeTerminal('owner'))
    expect(s().focusedTerminalId).toBeNull()
    expect(s().selectedTerminalId).toBeNull()
  })

  it("leaves another session's panel alone", () => {
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('other', 'sh2')
    })

    act(() => s().removeTerminal('owner'))
    expect(s().terminalsPanes.get('other')?.terminals).toEqual(['sh2'])
    expect(s().terminals.has('sh2')).toBe(true)
  })

  it('survives a reload with the same shells and the same one in front', async () => {
    const { parsePersistedPanels } = await import('../src/renderer/stores/ui-slice')
    act(() => {
      s().openTerminalsPane('owner', 'sh1')
      s().openTerminalsPane('owner', 'sh2')
      s().setActivePanelTerminal('owner', 0)
    })

    const restored = parsePersistedPanels(
      JSON.parse(localStorage.getItem('vorn:terminalPanels') as string)
    )
    expect(restored.get('owner')).toEqual({ terminals: ['sh1', 'sh2'], activeTab: 0 })
  })

  it('drops a persisted panel that has no shells left', async () => {
    const { parsePersistedPanels } = await import('../src/renderer/stores/ui-slice')
    // Its ids are what hid those terminals; an empty one is a pane that would
    // render nothing.
    const restored = parsePersistedPanels({ owner: { terminals: [], activeTab: 0 } })
    expect(restored.has('owner')).toBe(false)
  })

  it('clamps a persisted active tab that points past the end', async () => {
    const { parsePersistedPanels } = await import('../src/renderer/stores/ui-slice')
    const restored = parsePersistedPanels({ owner: { terminals: ['sh1'], activeTab: 7 } })
    expect(restored.get('owner')?.activeTab).toBe(0)
  })
})
