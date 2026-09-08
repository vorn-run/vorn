// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, cleanup, act, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

vi.hoisted(() => {
  Object.defineProperty(window, 'matchMedia', {
    value: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
    writable: true
  })
})

vi.mock('../src/renderer/hooks/useAgentInstallStatus', () => ({
  useAgentInstallStatus: () => ({ status: { claude: true } })
}))
vi.mock('../src/renderer/hooks/useWorkspaceWorkflows', () => ({
  useWorkspaceWorkflows: () => []
}))

import { useAppStore } from '../src/renderer/stores'
import { CardContextMenu } from '../src/renderer/components/CardContextMenu'
import { __resetExtensionsCacheForTests } from '../src/renderer/lib/use-extensions'
import type { ExtensionActivationState, InstalledConnectorPack } from '../packages/shared/src/types'

const openPane = vi.fn(async () => ({
  extensionId: 'demo',
  paneId: 'report',
  sessionId: 't1',
  url: 'http://127.0.0.1:6000/x/',
  nonce: 'n0'
}))

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
    contributes: {
      panes: [
        { id: 'report', title: 'Report' },
        { id: 'top', title: 'Top' }
      ]
    }
  }) as InstalledConnectorPack

const activation = (over: Partial<ExtensionActivationState> = {}): ExtensionActivationState => ({
  extensionId: 'demo',
  extensionName: 'Demo',
  active: true,
  panes: ['report'],
  footers: [],
  linkHandlers: [],
  ...over
})

function seed(states: ExtensionActivationState[]): void {
  act(() => {
    useAppStore.setState({
      terminals: new Map([
        [
          't1',
          {
            id: 't1',
            session: { id: 't1', agentType: 'claude', projectName: 'p', projectPath: '/p' },
            status: 'idle',
            lastOutputTimestamp: 1
          }
        ]
      ]) as never,
      extensionActivation: new Map(states.length > 0 ? [['t1', states]] : []),
      extensionPanes: new Map(),
      config: { projects: [], defaults: { defaultAgent: 'claude' } } as never
    })
  })
}

async function openMenu(): Promise<void> {
  render(<CardContextMenu terminalId="t1" position={{ x: 10, y: 10 }} onClose={() => {}} />)
  await waitFor(() => expect(screen.getByText('Open pane')).toBeInTheDocument())
  fireEvent.mouseEnter(screen.getByText('Open pane').closest('button') as HTMLElement)
}

describe('offering the panes an extension contributes', () => {
  beforeEach(() => {
    __resetExtensionsCacheForTests()
    openPane.mockClear()
    ;(window as unknown as { api: Record<string, unknown> }).api = {
      listExtensions: vi.fn(async () => [pack()]),
      openExtensionPane: openPane,
      closeExtensionPane: vi.fn(async () => ({ closed: true }))
    }
  })

  afterEach(() => cleanup())

  it('offers nothing while no extension is showing on this card', async () => {
    seed([])
    render(<CardContextMenu terminalId="t1" position={{ x: 10, y: 10 }} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByText('New session')).toBeInTheDocument())
    expect(screen.queryByText('Open pane')).toBeNull()
  })

  // Activation says which contributions show; the title belongs to the pack, so
  // a row that named the id would be naming something nobody wrote down.
  it('names each pane from its pack, and says which extension it came from', async () => {
    seed([activation()])
    await openMenu()

    expect(await screen.findByText('Report')).toBeInTheDocument()
    expect(screen.getByText('Demo')).toBeInTheDocument()
    // Contributed but not active here, so not offered.
    expect(screen.queryByText('Top')).toBeNull()
  })

  it('offers nothing for an extension that is installed but not showing here', async () => {
    seed([activation({ active: false })])
    render(<CardContextMenu terminalId="t1" position={{ x: 10, y: 10 }} onClose={() => {}} />)
    await waitFor(() => expect(screen.getByText('New session')).toBeInTheDocument())
    expect(screen.queryByText('Open pane')).toBeNull()
  })

  it('opens the pane the row names', async () => {
    seed([activation()])
    await openMenu()

    fireEvent.click(await screen.findByText('Report'))
    await waitFor(() => expect(openPane).toHaveBeenCalledWith('demo', 'report', 't1'))
  })

  it('offers to close the one already showing rather than opening it again', async () => {
    seed([activation()])
    act(() => {
      useAppStore.setState({
        extensionPanes: new Map([
          [
            't1',
            {
              open: {
                extensionId: 'demo',
                paneId: 'report',
                sessionId: 't1',
                url: 'http://x/',
                nonce: 'n0'
              },
              title: 'Report',
              extensionName: 'Demo'
            }
          ]
        ]) as never
      })
    })
    await openMenu()

    expect(await screen.findByText('Close Report')).toBeInTheDocument()
  })
})
