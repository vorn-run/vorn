// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { act, waitFor } from '@testing-library/react'
import { useAppStore } from '../src/renderer/stores'
import {
  hydrateExtensions as hydrate,
  __resetExtensionHydrationForTests
} from '../src/renderer/lib/extension-hydration'
import type {
  ExtensionActivationState,
  ExtensionFooterReading,
  TerminalSession
} from '../packages/shared/src/types'

const reading: ExtensionFooterReading = {
  extensionId: 'demo',
  extensionName: 'Demo',
  footerId: 'checks',
  title: 'Checks',
  items: [{ label: 'tests', value: 'passing', tone: 'ok' }],
  computedAt: '2026-09-08T12:00:00.000Z'
}

const activation: ExtensionActivationState = {
  extensionId: 'demo',
  extensionName: 'Demo',
  active: true,
  panes: ['report'],
  footers: ['checks'],
  linkHandlers: []
}

/** The store, as the slice hands it in. */
const hydrateExtensions = (sessionId: string): Promise<void> =>
  hydrate(sessionId, () => useAppStore.getState())

const session = (id: string): TerminalSession =>
  ({
    id,
    projectName: 'p',
    projectPath: '/p',
    agentType: 'claude',
    status: 'idle',
    createdAt: 0
  }) as TerminalSession

let askedActivation: ReturnType<typeof vi.fn>
let askedFooters: ReturnType<typeof vi.fn>

describe('asking the host what a card already holds', () => {
  beforeEach(() => {
    __resetExtensionHydrationForTests()
    askedActivation = vi.fn(async () => [activation])
    askedFooters = vi.fn(async () => [reading])
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      notifyWidgetStatus: vi.fn(),
      extensionActivation: askedActivation,
      extensionFooterItems: askedFooters
    }
    act(() => {
      useAppStore.setState({
        terminals: new Map(),
        terminalOrder: [],
        extensionFooters: new Map(),
        extensionActivation: new Map()
      })
    })
  })

  afterEach(() => __resetExtensionHydrationForTests())

  // Pushed only when something changes, so a window that opens on a settled
  // branch would show nothing at all until it happened to move.
  it('asks once when a session appears, and puts the answer on the card', async () => {
    act(() => useAppStore.getState().addTerminal(session('t1')))

    await waitFor(() => {
      expect(useAppStore.getState().extensionFooters.get('t1')).toHaveLength(1)
    })
    expect(useAppStore.getState().extensionActivation.get('t1')).toHaveLength(1)
    expect(askedActivation).toHaveBeenCalledWith('t1')
    expect(askedFooters).toHaveBeenCalledWith('t1')
  })

  // A card remounts on every maximize, tab switch and drag; asking there would
  // rewrite a map every other card reads, each time.
  it('does not ask again for a session it has already asked about', async () => {
    act(() => useAppStore.setState({ terminals: new Map([['t1', { id: 't1' }]]) as never }))
    await hydrateExtensions('t1')
    await hydrateExtensions('t1')
    expect(askedActivation).toHaveBeenCalledTimes(1)
  })

  it('keeps nothing for a session that closed while the host was answering', async () => {
    act(() => useAppStore.setState({ terminals: new Map() }))
    await hydrateExtensions('gone')
    expect(useAppStore.getState().extensionFooters.has('gone')).toBe(false)
  })

  it('asks again for an id that comes back', async () => {
    act(() => useAppStore.getState().addTerminal(session('t1')))
    await waitFor(() => expect(askedActivation).toHaveBeenCalledTimes(1))

    act(() => useAppStore.getState().removeTerminal('t1'))
    act(() => useAppStore.getState().addTerminal(session('t1')))

    await waitFor(() => expect(askedActivation).toHaveBeenCalledTimes(2))
  })

  it('leaves the card alone when the host cannot answer', async () => {
    askedActivation.mockRejectedValueOnce(new Error('no server'))
    askedFooters.mockRejectedValueOnce(new Error('no server'))
    act(() => useAppStore.setState({ terminals: new Map([['t1', { id: 't1' }]]) as never }))

    await hydrateExtensions('t1')

    expect(useAppStore.getState().extensionFooters.has('t1')).toBe(false)
    expect(useAppStore.getState().extensionActivation.has('t1')).toBe(false)
  })
})
