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
  packUrl: 'https://packs.example/slack-1.2.0.vorn.tgz',
  sha256: 'a'.repeat(64),
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
  packUrl: 'https://packs.example/discord-1.0.0.vorn.tgz',
  keywords: ['chat'],
  actions: [{ type: 'post', label: 'Post message' }],
  launch: { command: 'npx', args: [] }
}
// Nothing vouched for this one; it is findable but not offered first.
delete (DISCORD as { verified?: unknown }).verified

// A connector this machine is connected to is offered once, as a live action, never also as an install.
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
  items: [DISCORD, SLACK, PACKDEMO],
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
  localStorage.clear()
  packs.length = 0
  listConnectorCatalog.mockResolvedValue({
    items: [DISCORD, SLACK, PACKDEMO],
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

const connector = (name: RegExp) => screen.findByRole('button', { name })

describe('steps from connectors nobody has installed', () => {
  it('folds each connector under its name, the checked ones first', async () => {
    draw()
    const slack = await connector(/Slack/)
    const discord = await connector(/Discord/)
    expect(slack.compareDocumentPosition(discord) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.getAllByText('install on add')).toHaveLength(2)
    expect(screen.queryByText('Post message')).toBeNull()
  })

  it('says nothing about a connector this machine is already connected to', async () => {
    draw()
    await connector(/Slack/)
    expect(screen.queryByRole('button', { name: /Pack Demo/ })).toBeNull()
    expect(screen.queryByText('Echo')).toBeNull()
  })

  it('finds a step by what its connector talks about', async () => {
    draw()
    await connector(/Slack/)
    fireEvent.change(screen.getByPlaceholderText('Search steps and actions'), {
      target: { value: 'chat' }
    })

    expect(screen.getByRole('button', { name: /Post message.*Slack/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Post message.*Discord/ })).toBeInTheDocument()
    expect(screen.queryByText('Agent')).toBeNull()
  })

  it('hands back the connector, so the step knows what to ask for', async () => {
    const { onPick } = draw()
    fireEvent.click(await connector(/Slack/))
    fireEvent.click(screen.getByText('Post message'))

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
    expect(screen.queryByRole('button', { name: /Slack/ })).toBeNull()
  })

  it('promises a connection rather than an install once the files are on disk', async () => {
    packs.push({ id: 'slack' })
    draw()

    expect(await connector(/Slack.*add connection/)).toBeInTheDocument()
    // Discord is still only in the catalog, so it still promises the install.
    expect(screen.getAllByText('install on add')).toHaveLength(1)
  })

  it('shows what Check now found, without the panel being reopened', async () => {
    draw()
    fireEvent.click(await connector(/Slack/))
    expect(screen.getByText('Post message')).toBeInTheDocument()

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

    expect(screen.queryByRole('button', { name: /Slack/ })).toBeNull()
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
