// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type {
  ConnectorCatalogItem,
  ExtensionActivationState,
  InstalledConnectorPack
} from '../src/shared/types'

/**
 * How many cards an extension is actually on.
 *
 * The count is the one number on this page a person could check by looking, so
 * it has to mean what it says: cards, not sessions, and only the ones this
 * window opened. A shell inside a card's panel is a session of its own and
 * deliberately not a card, which is the way the fraction used to overstate both
 * of its halves.
 */

const state = {
  config: { workflows: [], projects: [] },
  extensionActivation: new Map<string, ExtensionActivationState[]>(),
  terminalsPanes: new Map<string, { terminals: string[]; activeTab: number }>()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: Object.assign((selector: (s: typeof state) => unknown) => selector(state), {
    getState: () => state
  })
}))

const { ConnectorSettings } = await import('../src/renderer/components/settings/ConnectorSettings')

const REVIEW: ConnectorCatalogItem = {
  id: 'review',
  name: 'Review',
  description: 'Read the diff beside the terminal.',
  kind: 'extension',
  packageName: '@vornrun/extension-review',
  version: '0.1.0',
  packUrl: 'https://packs.test/review-0.1.0.vorn.tgz',
  capabilities: [],
  category: 'Development',
  keywords: [],
  launch: { command: 'node', args: ['/packs/review/index.js'] },
  contributes: { footers: [{ id: 'checks', title: 'Checks', every: 30 }] },
  permissions: ['terminal.read']
}

const PACK = {
  id: 'review',
  name: 'Review',
  version: '0.1.0',
  kind: 'extension' as const,
  path: '/packs/review',
  installedAt: 0,
  bytes: 1,
  triggers: [],
  actions: [],
  env: [],
  contributes: REVIEW.contributes,
  permissions: REVIEW.permissions
} as unknown as InstalledConnectorPack

/** What the host says about one session: active, and drawing something. */
function drawing(active = true): ExtensionActivationState {
  return {
    extensionId: 'review',
    extensionName: 'Review',
    active,
    panes: [],
    footers: active ? ['checks'] : [],
    linkHandlers: []
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  state.extensionActivation = new Map()
  state.terminalsPanes = new Map()
  ;(window as unknown as { api: unknown }).api = {
    listConnectors: vi.fn().mockResolvedValue([]),
    listConnections: vi.fn().mockResolvedValue([]),
    getConnectorStatus: vi.fn().mockResolvedValue([]),
    listConnectorCatalog: vi.fn().mockResolvedValue({ items: [REVIEW], fetchedAt: Date.now() }),
    listConnectorPacks: vi.fn().mockResolvedValue([PACK]),
    onConnectorInstallProgress: vi.fn().mockReturnValue(() => {})
  }
})

describe('the cards an extension is on', () => {
  it('counts one card per session it draws on', async () => {
    state.extensionActivation = new Map([
      ['card-1', [drawing()]],
      ['card-2', [drawing()]]
    ])
    render(<ConnectorSettings />)
    expect(await screen.findByText(/on 2 of 2 open cards/)).toBeInTheDocument()
  })

  // The shell in a card's panel is drawn inside that card, so counting it would
  // report two cards where a person sees one.
  it('leaves out a shell claimed by a card panel', async () => {
    state.extensionActivation = new Map([
      ['card-1', [drawing()]],
      ['shell-1', [drawing()]]
    ])
    state.terminalsPanes = new Map([['card-1', { terminals: ['shell-1'], activeTab: 0 }]])
    render(<ConnectorSettings />)
    expect(await screen.findByText(/on 1 of 1 open card/)).toBeInTheDocument()
  })

  // Installed and drawing nowhere is a fact worth saying; silence reads as unknown.
  it('says so when it is installed and on no card', async () => {
    state.extensionActivation = new Map([['card-1', [drawing(false)]]])
    render(<ConnectorSettings />)
    expect(await screen.findByText(/on no open card/)).toBeInTheDocument()
  })

  // Active with nothing listed draws nothing, whatever the flag says.
  it('does not count a card it draws nothing on', async () => {
    const silent: ExtensionActivationState = { ...drawing(), footers: [] }
    state.extensionActivation = new Map([['card-1', [silent]]])
    render(<ConnectorSettings />)
    await waitFor(() => expect(screen.getByText(/on no open card/)).toBeInTheDocument())
  })
})
