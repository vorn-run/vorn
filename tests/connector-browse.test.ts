import { describe, it, expect } from 'vitest'
import {
  buildConnectorListings,
  filterConnectorListings,
  groupConnectorListings,
  type BuiltInConnector
} from '../src/renderer/lib/connector-browse'
import type { ConnectorCatalogItem, SourceConnection } from '../src/shared/types'

const builtIn = (id: string, name: string): BuiltInConnector => ({
  id,
  name,
  capabilities: ['triggers']
})

const catalogItem = (
  id: string,
  name: string,
  extra: Partial<ConnectorCatalogItem> = {}
): ConnectorCatalogItem => ({
  id,
  name,
  description: `${name} connector`,
  packageName: `@vornrun/connector-${id}`,
  capabilities: ['triggers'],
  launch: { command: 'npx', args: ['-y', `@vornrun/connector-${id}`] },
  ...extra
})

const connection = (overrides: Partial<SourceConnection>): SourceConnection =>
  ({
    id: 'c1',
    connectorId: 'mcp',
    name: 'conn',
    filters: {},
    syncIntervalMinutes: 5,
    statusMapping: {},
    ...overrides
  }) as SourceConnection

describe('buildConnectorListings', () => {
  it('presents built-ins and packaged connectors as one list', () => {
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [catalogItem('kusto', 'Azure Data Explorer')],
      []
    )

    expect(listings.map((l) => l.name)).toEqual(['Azure Data Explorer', 'GitHub'])
  })

  it('carries the launch spec only for packaged ones, which is what Add branches on', () => {
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [catalogItem('kusto', 'Kusto')],
      []
    )

    expect(listings.find((l) => l.id === 'github')?.catalogItem).toBeUndefined()
    expect(listings.find((l) => l.id === 'kusto')?.catalogItem?.launch).toEqual({
      command: 'npx',
      args: ['-y', '@vornrun/connector-kusto']
    })
  })

  it('counts a packaged connection by the connector it installed, not its connector id', () => {
    // Packaged connectors are all stored as `mcp`, so counting by connectorId
    // would credit every one of them to every other.
    const listings = buildConnectorListings(
      [],
      [catalogItem('kusto', 'Kusto'), catalogItem('other', 'Other')],
      [
        connection({ id: 'a', filters: { sdkConnectorId: 'kusto' } }),
        connection({ id: 'b', filters: { sdkConnectorId: 'kusto' } })
      ]
    )

    expect(listings.find((l) => l.id === 'kusto')?.connectedCount).toBe(2)
    expect(listings.find((l) => l.id === 'other')?.connectedCount).toBe(0)
  })

  it('floats connected connectors up, so the list also serves finding one already set up', () => {
    const listings = buildConnectorListings(
      [builtIn('alpha', 'Alpha'), builtIn('zulu', 'Zulu')],
      [],
      [connection({ connectorId: 'zulu' })]
    )

    expect(listings.map((l) => l.id)).toEqual(['zulu', 'alpha'])
  })

  it('orders by name rather than registration, so positions stay put as the catalog grows', () => {
    const listings = buildConnectorListings(
      [builtIn('c', 'Charlie'), builtIn('a', 'Alpha')],
      [catalogItem('b', 'Bravo')],
      []
    )

    expect(listings.map((l) => l.name)).toEqual(['Alpha', 'Bravo', 'Charlie'])
  })
})

describe('filterConnectorListings', () => {
  const listings = buildConnectorListings(
    [builtIn('github', 'GitHub')],
    [
      catalogItem('kusto', 'Azure Data Explorer', {
        description: 'Trigger from a KQL query',
        keywords: ['kusto', 'adx', 'logs'],
        capabilities: ['triggers', 'actions']
      })
    ],
    []
  )

  it('returns everything for an empty or blank query', () => {
    expect(filterConnectorListings(listings, '')).toHaveLength(2)
    expect(filterConnectorListings(listings, '   ')).toHaveLength(2)
  })

  it('finds a connector by a name nobody would guess from the product name', () => {
    expect(filterConnectorListings(listings, 'kusto').map((l) => l.id)).toEqual(['kusto'])
  })

  it('finds a connector by what it does, not only by what it is called', () => {
    expect(filterConnectorListings(listings, 'kql').map((l) => l.id)).toEqual(['kusto'])
    expect(filterConnectorListings(listings, 'logs').map((l) => l.id)).toEqual(['kusto'])
  })

  it('ignores case, since nobody types product capitalisation', () => {
    expect(filterConnectorListings(listings, 'GITHUB').map((l) => l.id)).toEqual(['github'])
  })

  it('narrows on each extra word rather than widening', () => {
    expect(filterConnectorListings(listings, 'azure explorer')).toHaveLength(1)
    expect(filterConnectorListings(listings, 'azure github')).toHaveLength(0)
  })

  it('returns nothing rather than everything when there is no match', () => {
    expect(filterConnectorListings(listings, 'nonexistent')).toEqual([])
  })
})

describe('groupConnectorListings', () => {
  it('groups by category and keeps each group in list order', () => {
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [
        catalogItem('kusto', 'Kusto', { category: 'Data' }),
        catalogItem('grafana', 'Grafana', { category: 'Data' })
      ],
      []
    )

    const groups = groupConnectorListings(listings)
    // Group order follows the name-sorted listings: GitHub precedes Grafana.
    expect(groups.map((g) => g.category)).toEqual(['Built in', 'Data'])
    expect(groups[1].listings.map((l) => l.name)).toEqual(['Grafana', 'Kusto'])
  })

  it('files an uncategorised package rather than dropping it from the list', () => {
    const groups = groupConnectorListings(
      buildConnectorListings([], [catalogItem('kusto', 'Kusto')], [])
    )

    expect(groups).toHaveLength(1)
    expect(groups[0].listings.map((l) => l.id)).toEqual(['kusto'])
  })

  it('produces nothing to render when a search matched nothing', () => {
    expect(groupConnectorListings([])).toEqual([])
  })
})

describe('connectors that are installed but not in the catalog', () => {
  it('still gets a row, so a package installed by name is not invisible', () => {
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [],
      [connection({ filters: { sdkConnectorId: 'jira' } })]
    )

    const jira = listings.find((l) => l.id === 'jira')
    expect(jira?.category).toBe('Installed')
    expect(jira?.connectedCount).toBe(1)
  })

  it('is marked so the UI does not offer to add one', () => {
    // There is no manifest and no package spec behind such a row, so there is
    // nothing for an add form to open against.
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [catalogItem('kusto', 'Azure Data Explorer')],
      [connection({ filters: { sdkConnectorId: 'jira' } })]
    )

    expect(listings.find((l) => l.id === 'jira')?.source).toBe('installed')
    expect(listings.find((l) => l.id === 'github')?.source).toBe('builtin')
    expect(listings.find((l) => l.id === 'kusto')?.source).toBe('catalog')
  })

  it('keeps one row per connector rather than one per connection', () => {
    const listings = buildConnectorListings(
      [],
      [],
      [
        connection({ id: 'a', filters: { sdkConnectorId: 'jira' } }),
        connection({ id: 'b', filters: { sdkConnectorId: 'jira' } })
      ]
    )

    expect(listings).toHaveLength(1)
    expect(listings[0].connectedCount).toBe(2)
  })

  it('does not duplicate a connector the catalog already lists', () => {
    const listings = buildConnectorListings(
      [],
      [catalogItem('kusto', 'Kusto')],
      [connection({ filters: { sdkConnectorId: 'kusto' } })]
    )

    expect(listings).toHaveLength(1)
    expect(listings[0].catalogItem).toBeDefined()
  })

  it('carries the icon the connection stored, so the row is not blank', () => {
    const listings = buildConnectorListings(
      [],
      [],
      [
        connection({
          filters: {
            sdkConnectorId: 'jira',
            sdkIcon: JSON.stringify({ viewBox: '0 0 24 24', paths: ['M0 0h24v24H0z'] })
          }
        })
      ]
    )

    expect(listings[0].icon).toEqual({ viewBox: '0 0 24 24', paths: ['M0 0h24v24H0z'] })
  })

  it('leaves a plain MCP connection under mcp rather than inventing a row', () => {
    const listings = buildConnectorListings(
      [builtIn('mcp', 'MCP')],
      [],
      [connection({ connectorId: 'mcp', filters: {} })]
    )

    expect(listings).toHaveLength(1)
    expect(listings[0].id).toBe('mcp')
    expect(listings[0].connectedCount).toBe(1)
  })
})
