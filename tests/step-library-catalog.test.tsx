// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { act, render, cleanup, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { ConnectorCatalogItem } from '../src/shared/types'

const connections = [
  { id: 'http-1', name: 'reporting API', connectorId: 'http', filters: {} },
  { id: 'c1', name: 'Pack Demo', connectorId: 'mcp', filters: { sdkConnectorId: 'packdemo' } }
]

const packs: Array<{ id: string }> = []

vi.mock('../src/renderer/lib/use-connections', () => ({
  useConnections: () => connections,
  useInstalledPacks: () => packs,
  useConnectorIdFor: () => null,
  useConnectionIconFor: () => undefined
}))

const SLACK: ConnectorCatalogItem = {
  id: 'slack',
  name: 'Slack',
  description: 'Messages and channels',
  packageName: '@vornrun/connector-slack',
  capabilities: ['actions'],
  keywords: ['chat'],
  verified: {
    schema: 1,
    version: '1.2.0',
    checkedAt: '2026-09-02T00:00:00Z',
    checks: ['manifest']
  },
  actions: [{ type: 'post', label: 'Post message' }],
  launch: { command: 'npx', args: [] }
}

const DISCORD: ConnectorCatalogItem = {
  ...SLACK,
  id: 'discord',
  name: 'Discord',
  packageName: '@vornrun/connector-discord',
  keywords: ['chat'],
  actions: [{ type: 'post', label: 'Post message' }],
  launch: { command: 'npx', args: [] }
}
// Nothing vouched for this one; it is findable but not offered first.
delete (DISCORD as { verified?: unknown }).verified

// The connector this machine already has a connection to must not be offered
// twice — once as a live action, once as something to install.
const PACKDEMO: ConnectorCatalogItem = {
  ...SLACK,
  id: 'packdemo',
  name: 'Pack Demo',
  packageName: '@vornrun/connector-packdemo',
  actions: [{ type: 'echo', label: 'Echo' }],
  launch: { command: 'npx', args: [] }
}

const listConnectionActions = vi.fn(async () => [])
const listConnectorCatalog = vi.fn(async () => ({
  items: [SLACK, DISCORD, PACKDEMO],
  templates: [],
  mcpServers: []
}))
const refreshCatalog = vi.fn()
;(window as unknown as { api: Record<string, unknown> }).api = {
  ...(window as unknown as { api?: Record<string, unknown> }).api,
  listConnectionActions,
  listConnectorCatalog,
  refreshConnectorCatalog: refreshCatalog
}

const { StepLibrary } =
  await import('../src/renderer/components/workflow-editor/panels/StepLibrary')
type LibraryScope = Parameters<typeof StepLibrary>[0]['scope']
const { __resetCatalogCacheForTests, refreshConnectorCatalog } =
  await import('../src/renderer/lib/use-connector-catalog')

beforeEach(() => {
  __resetCatalogCacheForTests()
  vi.clearAllMocks()
  packs.length = 0
  listConnectorCatalog.mockResolvedValue({
    items: [SLACK, DISCORD, PACKDEMO],
    templates: [],
    mcpServers: []
  })
})
afterEach(cleanup)

const draw = (scope: LibraryScope = { bodyOnly: false, insideBranch: false }) => {
  const onPick = vi.fn()
  const utils = render(<StepLibrary scope={scope} onPick={onPick} onClose={vi.fn()} />)
  return { ...utils, onPick }
}

describe('steps from connectors nobody has installed', () => {
  it('offers a checked connector as plainly as an installed one', async () => {
    draw()
    expect(await screen.findAllByText('Post message')).toHaveLength(2)
    expect(screen.getAllByText('install on add').length).toBeGreaterThan(0)
  })

  it('keeps the unvouched ones under their own heading', async () => {
    draw()
    expect(await screen.findByText('More from the catalog')).toBeInTheDocument()
  })

  it('says nothing about a connector this machine is already connected to', async () => {
    draw()
    await screen.findByText('More from the catalog')
    expect(screen.queryByText('Echo')).toBeNull()
  })

  it('finds a step by what its connector talks about', async () => {
    draw()
    await screen.findByText('More from the catalog')
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'chat' }
    })

    expect(screen.getAllByText('Post message')).toHaveLength(2)
    expect(screen.queryByText('Agent')).toBeNull()
  })

  it('hands back the connector, so the step knows what to ask for', async () => {
    const { onPick } = draw()
    const [first] = await screen.findAllByText('Post message')
    fireEvent.click(first)

    expect(onPick).toHaveBeenCalledWith({
      kind: 'catalogAction',
      connectorId: 'slack',
      action: 'post',
      actionLabel: 'Post message'
    })
  })

  it('leaves a loop body to the steps it can repeat', async () => {
    draw({ bodyOnly: true, insideBranch: false })
    await Promise.resolve()
    expect(screen.queryByText('Post message')).toBeNull()
    expect(screen.queryByText('More from the catalog')).toBeNull()
  })

  it('promises a connection rather than an install once the files are on disk', async () => {
    packs.push({ id: 'slack' })
    draw()
    await screen.findAllByText('Post message')

    expect(screen.getAllByText('add connection').length).toBeGreaterThan(0)
    // Discord is still only in the catalog, so its row still promises the install.
    expect(screen.getAllByText('install on add')).toHaveLength(1)
  })

  it('shows what Check now found, without the panel being reopened', async () => {
    draw()
    await screen.findAllByText('Post message')

    refreshCatalog.mockResolvedValue({
      items: [{ ...SLACK, actions: [{ type: 'status', label: 'Set status' }] }],
      templates: [],
      mcpServers: []
    })
    await act(async () => {
      await refreshConnectorCatalog()
    })

    expect(await screen.findByText('Set status')).toBeInTheDocument()
    expect(screen.queryByText('Post message')).toBeNull()
  })

  it('offers none of this when a step is being swapped in place', async () => {
    draw({ bodyOnly: false, insideBranch: false, replacing: true })
    await Promise.resolve()

    expect(screen.queryByText('Post message')).toBeNull()
    expect(screen.queryByText('More from the catalog')).toBeNull()
    expect(screen.queryByText('Call reporting API')).toBeNull()
  })
})

describe('a saved profile as a step', () => {
  it('offers the call beside the request it is a shortcut for', async () => {
    draw()
    expect(await screen.findByText('Call reporting API')).toBeInTheDocument()
  })

  it('picks the profile rather than asking for it again', async () => {
    const { onPick } = draw()
    fireEvent.click(await screen.findByText('Call reporting API'))

    expect(onPick).toHaveBeenCalledWith({
      kind: 'httpProfile',
      profileConnectionId: 'http-1',
      profileName: 'reporting API'
    })
  })
})
