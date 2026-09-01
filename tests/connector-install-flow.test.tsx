// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorCatalogItem, ConnectorPackSummary } from '../src/shared/types'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: Object.assign(
    (selector: (state: unknown) => unknown) => selector({ config: { workflows: [], projects: [] } }),
    { getState: () => ({}) }
  )
}))

const { ConnectorSettings } = await import(
  '../src/renderer/components/settings/ConnectorSettings'
)

const CATALOG: ConnectorCatalogItem = {
  id: 'acme',
  name: 'Acme',
  description: 'Acme tickets',
  packageName: '@vornrun/connector-acme',
  capabilities: ['actions'],
  category: 'Development',
  keywords: [],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-acme'] }
}

const PREVIEW: ConnectorPackSummary = {
  id: 'acme',
  name: 'Acme',
  version: '1.2.0',
  token: 'staged-1',
  triggers: [],
  actions: [{ type: 'closeTicket', label: 'Close ticket' }],
  env: []
}

const inspectConnectorPack = vi.fn()
const installConnectorPack = vi.fn()

beforeEach(() => {
  vi.clearAllMocks()
  inspectConnectorPack.mockResolvedValue({ ok: true, preview: PREVIEW })
  installConnectorPack.mockResolvedValue({ ok: true, pack: { ...PREVIEW, path: '/p', bytes: 1 } })
  ;(window as unknown as { api: unknown }).api = {
    listConnectors: vi.fn().mockResolvedValue([]),
    listConnections: vi.fn().mockResolvedValue([]),
    getConnectorStatus: vi.fn().mockResolvedValue([]),
    listConnectorCatalog: vi.fn().mockResolvedValue({ items: [CATALOG], fetchedAt: Date.now() }),
    listConnectorPacks: vi.fn().mockResolvedValue([]),
    onConnectorInstallProgress: vi.fn().mockReturnValue(() => {}),
    inspectConnectorPack,
    installConnectorPack
  }
})

/** Press the install action on the catalog row; with no connections it leads. */
async function pressInstall(): Promise<void> {
  render(<ConnectorSettings />)
  const row = await screen.findByRole('button', { name: /^Install$/ })
  fireEvent.click(row)
}

describe('installing from a catalog row', () => {
  it('checks the pack and shows what it would install before keeping it', async () => {
    await pressInstall()

    await waitFor(() => expect(inspectConnectorPack).toHaveBeenCalledTimes(1))
    // Nothing is committed by pressing Install: the sheet is the decision.
    expect(installConnectorPack).not.toHaveBeenCalled()
    expect(await screen.findByText('Close ticket')).toBeInTheDocument()
  })

  it('installs the files the sheet described, not the source they came from', async () => {
    await pressInstall()
    // The sheet's own action, which sits beside a Cancel the row does not have.
    const sheet = (await screen.findByRole('button', { name: 'Cancel' }))
      .parentElement as HTMLElement
    fireEvent.click(within(sheet).getByRole('button', { name: 'Install' }))

    await waitFor(() => expect(installConnectorPack).toHaveBeenCalledTimes(1))
    expect(installConnectorPack).toHaveBeenCalledWith({ kind: 'staged', token: 'staged-1' })
  })

  it('says on the row why a pack was refused', async () => {
    inspectConnectorPack.mockResolvedValue({ ok: false, error: 'declares dependencies' })

    await pressInstall()

    expect(await screen.findByText(/declares dependencies/)).toBeInTheDocument()
    expect(installConnectorPack).not.toHaveBeenCalled()
  })

  it('closes the sheet and says so when the install call itself fails', async () => {
    installConnectorPack.mockRejectedValue(new Error('the server went away'))

    await pressInstall()
    const sheet = (await screen.findByRole('button', { name: 'Cancel' }))
      .parentElement as HTMLElement
    fireEvent.click(within(sheet).getByRole('button', { name: 'Install' }))

    expect(await screen.findByText(/the server went away/)).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull())
  })
})
