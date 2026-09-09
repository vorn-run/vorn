// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act } from '@testing-library/react'
const destroyTerminal = vi.hoisted(() => vi.fn())
const toasted = vi.hoisted(() => vi.fn())

vi.mock('../src/renderer/lib/terminal-registry', () => ({ destroyTerminal }))
vi.mock('../src/renderer/components/Toast', () => ({ toast: toasted }))

import { useAppStore } from '../src/renderer/stores'
import { selectPaneFlags } from '../src/renderer/stores/ui-slice'
import { extensionPaneId } from '../src/renderer/lib/pane-id'
import { __resetExtensionsCacheForTests } from '../src/renderer/lib/use-extensions'
import { __resetExtensionHydrationForTests } from '../src/renderer/lib/extension-hydration'
import type {
  ExtensionActivationState,
  ExtensionFooterReading,
  ExtensionOpenPane,
  InstalledConnectorPack
} from '../packages/shared/src/types'

const session = (id: string) =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    createdAt: 0,
    displayName: id
  }) as never

const pack = (): InstalledConnectorPack =>
  ({
    id: 'demo',
    name: 'Demo',
    version: '0.1.0',
    kind: 'extension',
    path: '/packs/demo',
    installedAt: 0,
    bytes: 1,
    triggers: [],
    actions: [],
    env: [],
    contributes: { panes: [{ id: 'report', title: 'Report' }] }
  }) as InstalledConnectorPack

const reading = (over: Partial<ExtensionFooterReading> = {}): ExtensionFooterReading => ({
  extensionId: 'demo',
  extensionName: 'Demo',
  footerId: 'checks',
  title: 'Checks',
  items: [{ label: 'tests', value: 'passing', tone: 'ok' }],
  computedAt: '2026-09-08T12:00:00.000Z',
  ...over
})

const activation = (over: Partial<ExtensionActivationState> = {}): ExtensionActivationState => ({
  extensionId: 'demo',
  extensionName: 'Demo',
  active: true,
  panes: ['report'],
  footers: ['checks'],
  linkHandlers: [],
  ...over
})

const opened = (over: Partial<ExtensionOpenPane> = {}): ExtensionOpenPane => ({
  extensionId: 'demo',
  paneId: 'report',
  sessionId: 't1',
  url: 'http://127.0.0.1:6000/extensions/demo/pane/report/n0/',
  nonce: 'n0',
  ...over
})

let openPane: ReturnType<typeof vi.fn>
let closePane: ReturnType<typeof vi.fn>

function seed(ids: string[] = ['t1']): void {
  const terminals = new Map()
  for (const id of ids) {
    terminals.set(id, { id, session: session(id), status: 'idle', lastOutputTimestamp: 1 })
  }
  act(() => {
    useAppStore.setState({
      terminals,
      filesPanes: new Set(),
      editorPanes: new Map(),
      browserPanes: new Map(),
      browserMemory: new Map(),
      devicePanes: new Map(),
      extensionPanes: new Map(),
      extensionFooters: new Map(),
      extensionActivation: new Map(),
      terminalsPanes: new Map(),
      cardSplits: {},
      minimizedTerminals: new Set(),
      maximizedPaneId: null,
      terminalOrder: ids,
      visibleTerminalIds: [],
      knownSessionIds: new Set(ids)
    })
  })
}

describe('what a card holds for its extensions', () => {
  beforeEach(() => {
    localStorage.clear()
    __resetExtensionsCacheForTests()
    __resetExtensionHydrationForTests()
    destroyTerminal.mockClear()
    toasted.mockClear()
    openPane = vi.fn(async () => opened())
    closePane = vi.fn(async () => ({ closed: true }))
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      ...(window as unknown as { api?: Record<string, unknown> }).api,
      notifyWidgetStatus: vi.fn(),
      reorderSessions: vi.fn(),
      listExtensions: vi.fn(async () => [pack()]),
      extensionActivation: vi.fn(async () => [activation()]),
      extensionFooterItems: vi.fn(async () => [reading()]),
      openExtensionPane: openPane,
      closeExtensionPane: closePane
    }
    seed(['t1', 't2'])
  })

  afterEach(() => {
    __resetExtensionHydrationForTests()
  })

  it('keeps a reading, and drops the entry when there is nothing left to say', () => {
    act(() => useAppStore.getState().setExtensionFooters('t1', [reading()]))
    expect(useAppStore.getState().extensionFooters.get('t1')).toHaveLength(1)

    act(() => useAppStore.getState().setExtensionFooters('t1', []))
    expect(useAppStore.getState().extensionFooters.has('t1')).toBe(false)
  })

  // Every poll of every session answers for a card with no extensions, and
  // rebuilding the map each time would invalidate every band on screen.
  it('writes nothing when an empty answer says what the store already says', () => {
    const before = useAppStore.getState().extensionFooters
    act(() => useAppStore.getState().setExtensionFooters('t1', []))
    expect(useAppStore.getState().extensionFooters).toBe(before)

    const beforeStates = useAppStore.getState().extensionActivation
    act(() => useAppStore.getState().setExtensionActivation('t1', []))
    expect(useAppStore.getState().extensionActivation).toBe(beforeStates)
  })

  it('opens a pane on what the host granted, and names it from the pack', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })

    expect(openPane).toHaveBeenCalledWith('demo', 'report', 't1')
    const pane = useAppStore.getState().extensionPanes.get('t1')
    expect(pane?.open.nonce).toBe('n0')
    expect(pane?.title).toBe('Report')
    expect(pane?.extensionName).toBe('Demo')
    expect(selectPaneFlags(useAppStore.getState(), 't1').extension).toBe(true)
  })

  it('hands the grant back when it closes, and releases the pane from placement', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })
    act(() => useAppStore.setState({ maximizedPaneId: extensionPaneId('t1') }))

    act(() => useAppStore.getState().closeExtensionPane('t1'))

    expect(closePane).toHaveBeenCalledWith('n0')
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
    expect(useAppStore.getState().maximizedPaneId).toBeNull()
  })

  it('closes the pane already showing rather than opening it twice', async () => {
    await act(async () => {
      await useAppStore.getState().toggleExtensionPane('t1', 'demo', 'report')
    })
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(true)

    await act(async () => {
      await useAppStore.getState().toggleExtensionPane('t1', 'demo', 'report')
    })
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
    expect(openPane).toHaveBeenCalledTimes(1)
  })

  // A card shows one, so asking for a second gives the first one's grant back
  // rather than leaving the host holding a pane nothing draws.
  it('gives up the pane it was showing when another is asked for', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })
    openPane.mockResolvedValueOnce(opened({ paneId: 'top', nonce: 'n1', url: undefined }))

    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'top')
    })

    expect(closePane).toHaveBeenCalledWith('n0')
    expect(useAppStore.getState().extensionPanes.get('t1')?.open.nonce).toBe('n1')
  })

  it('lets go of everything an extension held when the session closes', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })
    act(() => {
      useAppStore.getState().setExtensionFooters('t1', [reading()])
      useAppStore.getState().setExtensionActivation('t1', [activation()])
    })

    act(() => useAppStore.getState().removeTerminal('t1'))

    const state = useAppStore.getState()
    expect(state.extensionPanes.has('t1')).toBe(false)
    expect(state.extensionFooters.has('t1')).toBe(false)
    expect(state.extensionActivation.has('t1')).toBe(false)
  })

  // A session that never came back leaves no removal to hang the cleanup off,
  // so the reconcile is the only thing that can let go of what it held.
  it('prunes what a session that never came back was holding', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })
    act(() => useAppStore.getState().setExtensionFooters('t1', [reading()]))

    // Gone from the board as well: the reconcile treats a session the store
    // still holds as live, which is what keeps one opened after the sync.
    act(() => {
      const terminals = new Map(useAppStore.getState().terminals)
      terminals.delete('t1')
      useAppStore.setState({ terminals })
    })
    act(() => useAppStore.getState().setKnownSessions(['t2']))

    const state = useAppStore.getState()
    expect(state.extensionPanes.has('t1')).toBe(false)
    expect(state.extensionFooters.has('t1')).toBe(false)
  })

  it("lets go of a program pane's terminal when the pane closes", async () => {
    openPane.mockResolvedValueOnce(opened({ paneId: 'top', url: undefined, terminalId: 'x1' }))
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'top')
    })

    act(() => useAppStore.getState().closeExtensionPane('t1'))

    expect(closePane).toHaveBeenCalledWith('n0')
    // Left registered it holds an xterm and a GPU context nobody can reach.
    expect(destroyTerminal).toHaveBeenCalledWith('x1')
  })

  it("lets go of a program pane's terminal when the session it belonged to closes", async () => {
    openPane.mockResolvedValueOnce(opened({ paneId: 'top', url: undefined, terminalId: 'x1' }))
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'top')
    })

    act(() => useAppStore.getState().removeTerminal('t1'))

    expect(destroyTerminal).toHaveBeenCalledWith('x1')
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
  })

  it('says why when the host refuses to open a pane', async () => {
    openPane.mockRejectedValueOnce(new Error('the page server is not running'))

    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })

    expect(toasted).toHaveBeenCalledWith('the page server is not running', 'error')
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
  })

  it('closes a pane whose extension no longer shows on this card', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })

    act(() => useAppStore.getState().setExtensionActivation('t1', [activation({ active: false })]))

    expect(closePane).toHaveBeenCalledWith('n0')
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
  })

  it('keeps the pane when its extension still shows', async () => {
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'report')
    })

    act(() => useAppStore.getState().setExtensionActivation('t1', [activation()]))

    expect(closePane).not.toHaveBeenCalled()
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(true)
  })

  it('closes the pane a program terminal belonged to when that program exits', async () => {
    openPane.mockResolvedValueOnce(opened({ paneId: 'top', url: undefined, terminalId: 'x1' }))
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'top')
    })

    let closed = false
    act(() => {
      closed = useAppStore.getState().closeExtensionPaneForTerminal('x1')
    })

    expect(closed).toBe(true)
    expect(useAppStore.getState().extensionPanes.has('t1')).toBe(false)
    // A terminal no pane is showing is a session, and this must not claim it.
    expect(useAppStore.getState().closeExtensionPaneForTerminal('t2')).toBe(false)
  })

  it('gives up every pane when the server that granted them is replaced', async () => {
    openPane.mockResolvedValueOnce(opened({ paneId: 'top', url: undefined, terminalId: 'x1' }))
    await act(async () => {
      await useAppStore.getState().openExtensionPane('t1', 'demo', 'top')
    })

    act(() => useAppStore.getState().dropExtensionPanes())

    expect(useAppStore.getState().extensionPanes.size).toBe(0)
    expect(destroyTerminal).toHaveBeenCalledWith('x1')
    // Nothing to hand back: the grant died with the server that minted it.
    expect(closePane).not.toHaveBeenCalled()
  })
})
