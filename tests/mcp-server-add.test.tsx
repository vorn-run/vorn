// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorManifest, McpServerCatalogEntry } from '../src/shared/types'

vi.mock('../src/renderer/stores', () => ({
  useAppStore: Object.assign(
    (selector: (state: unknown) => unknown) =>
      selector({ config: { workflows: [], projects: [] } }),
    { getState: () => ({}) }
  )
}))

const { ConnectorSettings } = await import('../src/renderer/components/settings/ConnectorSettings')

const SERVER: McpServerCatalogEntry = {
  id: 'playwright',
  name: 'Playwright',
  description: 'Drive a browser',
  command: 'npx',
  args: ['-y', '@playwright/mcp']
}

/** The built-in the generic stdio form is rendered against. */
const MCP_CONNECTOR = {
  id: 'mcp',
  name: 'MCP Server',
  icon: 'plug',
  capabilities: ['actions'],
  manifest: {
    id: 'mcp',
    name: 'MCP Server',
    auth: [
      { key: 'command', label: 'Command', type: 'text', required: true },
      { key: 'args', label: 'Arguments', type: 'text' }
    ],
    triggers: [],
    actions: []
  } as unknown as ConnectorManifest
}

const createConnection = vi.fn()

function stubApi(connectors: unknown[]): void {
  ;(window as unknown as { api: unknown }).api = {
    listConnectors: vi.fn().mockResolvedValue(connectors),
    listConnections: vi.fn().mockResolvedValue([]),
    getConnectorStatus: vi.fn().mockResolvedValue([]),
    listConnectorCatalog: vi.fn().mockResolvedValue({
      items: [],
      templates: [],
      mcpServers: [SERVER],
      fetchedAt: Date.now()
    }),
    listConnectorPacks: vi.fn().mockResolvedValue([]),
    onConnectorInstallProgress: vi.fn().mockReturnValue(() => {}),
    encryptString: vi.fn().mockResolvedValue('sealed'),
    createConnection
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  createConnection.mockResolvedValue(undefined)
  stubApi([MCP_CONNECTOR])
})

/** With no connections yet the directory leads, so the row is already there. */
async function pressAddServer(): Promise<void> {
  render(<ConnectorSettings />)
  fireEvent.click(await screen.findByRole('button', { name: /Add server/ }))
}

describe('adding a listed MCP server', () => {
  it('arrives at the manual form with the launch line already written', async () => {
    await pressAddServer()

    const command = (await screen.findByDisplayValue('npx')) as HTMLInputElement
    expect(command).toBeInTheDocument()
    expect(screen.getByDisplayValue('["-y","@playwright/mcp"]')).toBeInTheDocument()
  })

  it('stamps the server it belongs to, so its row can count the connection', async () => {
    await pressAddServer()
    await screen.findByDisplayValue('npx')

    fireEvent.click(screen.getByRole('button', { name: 'Connect' }))

    await waitFor(() => expect(createConnection).toHaveBeenCalledTimes(1))
    const created = createConnection.mock.calls[0][0]
    expect(created.connectorId).toBe('mcp')
    // Without this the row shows no connections and never says "Add another".
    expect(created.filters.sdkConnectorId).toBe('playwright')
  })

  it('says so rather than showing nothing when the MCP connector is missing', async () => {
    stubApi([])
    await pressAddServer()

    expect(await screen.findByText(/MCP connector is not available/)).toBeInTheDocument()
  })
})
