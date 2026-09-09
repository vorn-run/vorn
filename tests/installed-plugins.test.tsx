// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { InstalledPlugins } from '../src/renderer/components/settings/InstalledPlugins'
import { buildConnectorListings, type ConnectorListing } from '../src/renderer/lib/connector-browse'
import type { ConnectorCatalogItem, InstalledConnectorPack } from '../src/shared/types'
import type { RowActivity } from '../src/renderer/lib/use-row-action'

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
  contributes: {
    panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }],
    footers: [{ id: 'checks', title: 'Checks', every: 30 }]
  },
  permissions: ['terminal.read'],
  activates: { workspaceContains: ['package.json'] }
}

const ADO: ConnectorCatalogItem = {
  id: 'ado',
  name: 'Azure DevOps',
  description: 'Trigger workflows from the work items a WIQL query returns.',
  packageName: '@vornrun/connector-ado',
  version: '0.1.0',
  packUrl: 'https://packs.test/ado-0.1.0.vorn.tgz',
  capabilities: ['triggers'],
  category: 'Development',
  keywords: [],
  launch: { command: 'npx', args: ['-y', '@vornrun/connector-ado'] },
  triggers: [{ type: 'workItem', label: 'Work item matches the query' }] as never,
  actions: [],
  env: []
}

function pack(over: Partial<InstalledConnectorPack> = {}): InstalledConnectorPack {
  return {
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    path: '/packs/review',
    installedAt: 0,
    bytes: 1,
    triggers: [],
    actions: [],
    env: [],
    contributes: REVIEW.contributes,
    permissions: REVIEW.permissions,
    activates: REVIEW.activates,
    ...over
  } as InstalledConnectorPack
}

const ADO_PACK = pack({
  id: 'ado',
  name: 'Azure DevOps',
  kind: 'connector',
  path: '/packs/ado',
  triggers: [{ type: 'workItem', label: 'Work item matches the query' }] as never,
  contributes: undefined,
  permissions: undefined,
  activates: undefined
})

/** No action is running and none has failed: the ordinary row. */
const idle: RowActivity = {
  busy: {},
  failed: {},
  run: vi.fn(),
  state: () => ({})
}

function listings(packs: InstalledConnectorPack[], connections = []): ConnectorListing[] {
  return buildConnectorListings([], [REVIEW, ADO], connections, packs)
}

function setup(packs: InstalledConnectorPack[], over: Record<string, unknown> = {}) {
  const onSelect = vi.fn()
  const onInstall = vi.fn()
  const onAdd = vi.fn()
  const onRemove = vi.fn()
  const onBrowse = vi.fn()
  const utils = render(
    <InstalledPlugins
      listings={listings(packs)}
      builtIns={[]}
      activity={idle}
      onSelect={onSelect}
      onInstall={onInstall}
      onAdd={onAdd}
      onRemove={onRemove}
      onBrowse={onBrowse}
      {...over}
    />
  )
  return { ...utils, onSelect, onInstall, onAdd, onRemove, onBrowse }
}

describe('what is installed', () => {
  it('lists only what has files', () => {
    // Only the extension is installed; the connector is a catalog row like any other.
    const { getByText, queryByText } = setup([pack()])

    expect(getByText('Review')).toBeInTheDocument()
    // A catalog row nobody installed belongs in Browse, not here.
    expect(queryByText('Azure DevOps')).toBeNull()
  })

  it('groups by kind, extensions first', () => {
    const { getByText, container } = setup([pack(), ADO_PACK])

    const headings = [...container.querySelectorAll('h3')].map((h) => h.textContent)
    expect(headings).toEqual(['Extensions', 'Connectors'])
    expect(getByText('Azure DevOps')).toBeInTheDocument()
  })

  it('says nothing is installed, and offers the catalog instead', () => {
    const { getByText, onBrowse } = setup([])

    fireEvent.click(getByText('Browse'))
    expect(onBrowse).toHaveBeenCalled()
  })

  it('drops the headings when only one kind is installed', () => {
    const { container } = setup([pack()])
    expect(container.querySelectorAll('h3')).toHaveLength(0)
  })

  it('counts an extension against the cards this window has open', () => {
    const { getByText } = setup([pack()], { activeCards: { review: 1 }, openCards: 3 })
    expect(getByText(/on 1 of 3 open cards/)).toBeInTheDocument()
  })

  it('says so when an installed extension is on no card', () => {
    const { getByText } = setup([pack()], { activeCards: { review: 0 }, openCards: 2 })
    expect(getByText(/on no open card/)).toBeInTheDocument()
  })

  it('counts a connector by its connections, and offers one when it has none', () => {
    const { getByRole, getByText, onAdd } = setup([ADO_PACK])

    expect(getByText(/no connection yet/)).toBeInTheDocument()
    fireEvent.click(getByRole('button', { name: /Add connection/ }))
    expect(onAdd).toHaveBeenCalled()
  })

  it('never offers a connection for an extension', () => {
    const { queryByRole } = setup([pack()])
    expect(queryByRole('button', { name: /Add connection/ })).toBeNull()
  })

  it('offers the update only when the catalog carries a newer version', () => {
    const current = setup([pack()])
    expect(current.queryByRole('button', { name: 'Update' })).toBeNull()
    current.unmount()

    const behind = setup([pack({ version: '0.0.9' })])
    expect(behind.getByRole('button', { name: 'Update' })).toBeInTheDocument()
    expect(behind.getByText(/v0.1.0 available/)).toBeInTheDocument()
  })

  it('holds Uninstall while the row is removing, and says what it is doing', () => {
    const removing: RowActivity = { ...idle, state: () => ({ phrase: 'Removing…' }) }
    const { getByRole, getByText } = setup([pack()], { activity: removing })

    expect(getByRole('button', { name: /Uninstall/ })).toBeDisabled()
    expect(getByText('Removing…')).toBeInTheDocument()
  })

  it('opens the detail page from the row', () => {
    const { getByRole, onSelect } = setup([pack()])
    fireEvent.click(getByRole('button', { name: 'About Review' }))
    expect(onSelect).toHaveBeenCalled()
  })
})
