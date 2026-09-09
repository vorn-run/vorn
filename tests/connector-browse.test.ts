import { describe, it, expect } from 'vitest'
import { isImplicitConnection } from '../packages/shared/src/types'
import {
  buildConnectorListings,
  connectorKinds,
  filterByKind,
  filterConnectorListings,
  kindOf,
  groupConnections,
  connectorCategories,
  describeCatalogAge,
  filterByCategory,
  listingDetails,
  connectorAuthRungs,
  filterByAuthRung,
  CALLABLE_FILTER,
  CONNECTED_FILTER,
  type BuiltInConnector
} from '../src/renderer/lib/connector-browse'
import type {
  ConnectorAuthRung,
  ConnectorCatalogItem,
  InstalledConnectorPack,
  SourceConnection
} from '../src/shared/types'

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

describe('groupConnections', () => {
  it('puts two connections to one connector under one heading', () => {
    // The count belongs beside the things it counts, not on a catalog card.
    const listings = buildConnectorListings([], [catalogItem('ado', 'Azure DevOps')], [])
    const groups = groupConnections(
      [
        connection({ id: 'c1', connectorId: 'ado', name: 'Platform' }),
        connection({ id: 'c2', connectorId: 'ado', name: 'Escalations' })
      ],
      listings
    )

    expect(groups).toHaveLength(1)
    expect(groups[0]).toMatchObject({ connectorId: 'ado', name: 'Azure DevOps' })
    expect(groups[0].connections.map((c) => c.id)).toEqual(['c1', 'c2'])
  })

  it('carries the version, so a group can say what it is running', () => {
    const listings = buildConnectorListings(
      [],
      [catalogItem('ado', 'Azure DevOps', { version: '0.1.0' })],
      []
    )
    const groups = groupConnections([connection({ connectorId: 'ado' })], listings)
    expect(groups[0].version).toBe('0.1.0')
  })

  it('still shows a connection whose connector the catalog does not list', () => {
    // A package installed by name, or one the catalog has dropped, still polls
    // and can still fail. Hiding it would hide a running thing.
    const groups = groupConnections([connection({ id: 'c1', connectorId: 'mystery' })], [])

    expect(groups).toHaveLength(1)
    expect(groups[0]).toMatchObject({ connectorId: 'mystery', name: 'mystery' })
    // Nothing to open an Add form against.
    expect(groups[0].listing).toBeUndefined()
  })

  it('separates connectors rather than lumping every connection together', () => {
    const groups = groupConnections(
      [
        connection({ id: 'c1', connectorId: 'ado' }),
        connection({ id: 'c2', connectorId: 'kusto' }),
        connection({ id: 'c3', connectorId: 'ado' })
      ],
      []
    )
    expect(groups.map((g) => g.connectorId)).toEqual(['ado', 'kusto'])
    expect(groups[0].connections).toHaveLength(2)
  })

  it('groups a packaged connection under the connector it installed', () => {
    // The connection's own connectorId is `mcp`; what matters is the package.
    const listings = buildConnectorListings([], [catalogItem('kusto', 'Kusto')], [])
    const groups = groupConnections(
      [
        connection({
          id: 'c1',
          connectorId: 'mcp',
          filters: { sdkConnectorId: 'kusto' }
        } as never)
      ],
      listings
    )
    expect(groups[0]).toMatchObject({ connectorId: 'kusto', name: 'Kusto' })
  })

  it('has nothing to group when there are no connections', () => {
    expect(groupConnections([], [])).toEqual([])
  })
})

describe('listingDetails', () => {
  const packaged = catalogItem('ado', 'Azure DevOps', {
    version: '0.1.0',
    triggers: [{ type: 'workItem', label: 'A work item matches', description: 'Once per item' }],
    actions: [],
    env: [
      { name: 'ADO_ORGANIZATION', required: true, description: 'Name or URL' },
      { name: 'ADO_TOP', required: false }
    ]
  })

  const listingFor = (item: ConnectorCatalogItem) =>
    buildConnectorListings([], [item], []).find((listing) => listing.id === item.id)!

  it('reads what a packaged connector says about itself', () => {
    const details = listingDetails(listingFor(packaged))
    expect(details.known).toBe(true)
    expect(details.triggers).toEqual([
      { type: 'workItem', label: 'A work item matches', description: 'Once per item' }
    ])
    expect(details.actions).toEqual([])
    expect(details.settings).toEqual([
      { name: 'ADO_ORGANIZATION', required: true, description: 'Name or URL' },
      { name: 'ADO_TOP', required: false }
    ])
  })

  it('reaches the same shape for a built-in, so one card serves every row', () => {
    const github: BuiltInConnector = {
      id: 'github',
      name: 'GitHub',
      capabilities: ['triggers', 'actions'],
      manifest: {
        triggers: [{ type: 'issue', label: 'An issue is opened' }],
        actions: [{ type: 'comment', label: 'Comment on an issue' }],
        auth: [{ key: 'token', label: 'GITHUB_TOKEN', required: true }]
      }
    }
    const listing = buildConnectorListings([github], [], [])[0]
    const details = listingDetails(listing, [github])

    expect(details.known).toBe(true)
    expect(details.triggers).toEqual([{ type: 'issue', label: 'An issue is opened' }])
    expect(details.actions).toEqual([{ type: 'comment', label: 'Comment on an issue' }])
    expect(details.settings).toEqual([{ name: 'GITHUB_TOKEN', required: true }])
  })

  it('says it knows nothing rather than claiming there is nothing', () => {
    // An entry from a catalog published before these fields existed. Rendering
    // "no triggers" for it would be a confident lie.
    const details = listingDetails(listingFor(catalogItem('old', 'Old')))
    expect(details.known).toBe(false)
    expect(details.triggers).toEqual([])
  })

  it('knows nothing about a package installed by name, which carries no manifest', () => {
    const listings = buildConnectorListings(
      [],
      [],
      [connection({ id: 'c1', connectorId: 'mcp', name: 'thing' })]
    )
    const installed = listings.find((listing) => listing.source === 'installed')
    if (installed) expect(listingDetails(installed).known).toBe(false)
  })
})

describe('connectorCategories', () => {
  it('offers the categories actually present, so a new one needs no release', () => {
    const listings = buildConnectorListings(
      [],
      [
        catalogItem('a', 'A', { category: 'Development' }),
        catalogItem('b', 'B', { category: 'Data' })
      ],
      []
    )
    expect(connectorCategories(listings)).toEqual(['Data', 'Development'])
  })

  it('offers "callable" only when something can actually be called', () => {
    const watcher = catalogItem('a', 'A', { capabilities: ['triggers'] })
    const doer = catalogItem('b', 'B', { capabilities: ['triggers', 'actions'] })

    expect(connectorCategories(buildConnectorListings([], [watcher], []))).not.toContain(
      CALLABLE_FILTER
    )
    expect(connectorCategories(buildConnectorListings([], [watcher, doer], []))).toContain(
      CALLABLE_FILTER
    )
  })

  it('offers "connected" only once something is', () => {
    const item = catalogItem('a', 'A')
    expect(connectorCategories(buildConnectorListings([], [item], []))).not.toContain(
      CONNECTED_FILTER
    )
    const connected = buildConnectorListings([], [item], [connection({ connectorId: 'a' })])
    expect(connectorCategories(connected)).toContain(CONNECTED_FILTER)
  })
})

describe('filterByCategory', () => {
  const listings = () =>
    buildConnectorListings(
      [],
      [
        catalogItem('a', 'A', { category: 'Development', capabilities: ['triggers'] }),
        catalogItem('b', 'B', { category: 'Data', capabilities: ['triggers', 'actions'] })
      ],
      [connection({ connectorId: 'b' })]
    )

  it('returns everything when nothing is picked', () => {
    expect(filterByCategory(listings(), undefined)).toHaveLength(2)
  })

  it('narrows to one category', () => {
    expect(filterByCategory(listings(), 'Data').map((l) => l.id)).toEqual(['b'])
  })

  it('narrows to what a workflow step can call', () => {
    expect(filterByCategory(listings(), CALLABLE_FILTER).map((l) => l.id)).toEqual(['b'])
  })

  it('narrows to what is already set up, which is the other half of this page', () => {
    expect(filterByCategory(listings(), CONNECTED_FILTER).map((l) => l.id)).toEqual(['b'])
  })

  it('returns nothing rather than everything for a category nothing is in', () => {
    expect(filterByCategory(listings(), 'Nonexistent')).toEqual([])
  })
})

describe('describeCatalogAge', () => {
  const now = Date.UTC(2026, 7, 10, 12, 0, 0)

  it('says plainly when the published list has never been read', () => {
    // The alternative is a reassuring timestamp for a list that may be missing
    // everything published since the app was built.
    expect(describeCatalogAge(undefined, now)).toBe('Showing the connectors that shipped with Vorn')
  })

  it('reads in the units someone would ask in', () => {
    expect(describeCatalogAge(now - 30_000, now)).toBe('Updated just now')
    expect(describeCatalogAge(now - 60_000, now)).toBe('Updated 1 minute ago')
    expect(describeCatalogAge(now - 25 * 60_000, now)).toBe('Updated 25 minutes ago')
    expect(describeCatalogAge(now - 2 * 3_600_000, now)).toBe('Updated 2 hours ago')
    expect(describeCatalogAge(now - 3 * 86_400_000, now)).toBe('Updated 3 days ago')
  })

  it('does not say "1 minutes"', () => {
    expect(describeCatalogAge(now - 3_600_000, now)).toBe('Updated 1 hour ago')
    expect(describeCatalogAge(now - 86_400_000, now)).toBe('Updated 1 day ago')
  })
})

const packOf = (id: string, rung?: ConnectorAuthRung): InstalledConnectorPack => ({
  id,
  name: id,
  version: '1.0.0',
  path: `/packs/${id}`,
  installedAt: 0,
  bytes: 1,
  triggers: [],
  actions: [],
  env: [],
  ...(rung && { auth: { rung } })
})

const RECEIPT = {
  schema: 1 as const,
  version: '1.2.0',
  checkedAt: '2026-09-02T00:00:00Z',
  checks: ['manifest', 'no-runtime-deps']
}

describe('what a listing says about signing in', () => {
  it('carries the rung and the receipt the catalog published', () => {
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('slack', 'Slack', { authRung: 'key', verified: RECEIPT })],
      []
    )

    expect(listing.authRung).toBe('key')
    expect(listing.verified).toEqual(RECEIPT)
  })

  it('believes the pack on disk over the catalog, since the pack is what runs', () => {
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('slack', 'Slack', { authRung: 'key' })],
      [],
      [packOf('slack', 'cli')]
    )

    expect(listing.authRung).toBe('cli')
  })

  it('says nothing rather than none when neither declares a rung', () => {
    const [listing] = buildConnectorListings([], [catalogItem('slack', 'Slack')], [])

    expect(listing.authRung).toBeUndefined()
    expect(listing.verified).toBeUndefined()
  })

  it('reads a side-loaded pack its rung from its own manifest', () => {
    const [listing] = buildConnectorListings([], [], [], [packOf('echo', 'none')])

    expect(listing.authRung).toBe('none')
  })

  it('offers only the rungs on the page, in the order they ask for less', () => {
    const listings = buildConnectorListings(
      [],
      [
        catalogItem('a', 'A', { authRung: 'key' }),
        catalogItem('b', 'B', { authRung: 'none' }),
        catalogItem('c', 'C')
      ],
      []
    )

    expect(connectorAuthRungs(listings)).toEqual(['none', 'key'])
    expect(filterByAuthRung(listings, 'key').map((l) => l.id)).toEqual(['a'])
    expect(filterByAuthRung(listings, undefined)).toHaveLength(3)
  })
})

describe('a connection nobody asked for', () => {
  const implicit = connection({ id: 'imp', filters: { sdkConnectorId: 'echo', implicit: true } })
  const chosen = connection({ id: 'mine', filters: { sdkConnectorId: 'echo' } })

  it('leaves the count at nothing, so no row offers another of it', () => {
    const [listing] = buildConnectorListings([], [catalogItem('echo', 'Echo')], [implicit])

    expect(listing.connectedCount).toBe(0)
    expect(listing.implicitlyConnected).toBe(true)
    expect(isImplicitConnection(implicit)).toBe(true)
    expect(isImplicitConnection(chosen)).toBe(false)
  })

  it('still sorts a ready connector above one that needs setting up', () => {
    const listings = buildConnectorListings(
      [],
      [catalogItem('alpha', 'Alpha'), catalogItem('echo', 'Echo')],
      [implicit]
    )

    expect(listings.map((l) => l.id)).toEqual(['echo', 'alpha'])
  })

  it('counts the connections someone made beside it', () => {
    const [listing] = buildConnectorListings([], [catalogItem('echo', 'Echo')], [implicit, chosen])

    expect(listing.connectedCount).toBe(1)
    expect(listing.implicitlyConnected).toBe(true)
  })

  it('keeps it out of the connections someone can manage', () => {
    expect(groupConnections([implicit], [])).toEqual([])

    const groups = groupConnections([implicit, chosen], [])
    expect(groups).toHaveLength(1)
    expect(groups[0].connections.map((c) => c.id)).toEqual(['mine'])
  })
})

describe('what a listing is', () => {
  const REVIEW = {
    panes: [{ id: 'report', title: 'Report' }],
    footers: [{ id: 'checks', title: 'Checks', every: 30 }]
  }

  it('reads a catalog entry as what the catalog says it is', () => {
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('review', 'Review', { kind: 'extension', contributes: REVIEW })],
      []
    )
    expect(listing.kind).toBe('extension')
    expect(listing.contributes).toEqual(REVIEW)
  })

  it('reads a side-loaded pack as what its own manifest says', () => {
    const installed = {
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
      contributes: REVIEW,
      permissions: ['git.read' as const]
    }
    const [listing] = buildConnectorListings([], [], [], [installed])
    expect(listing.kind).toBe('extension')
    expect(listing.permissions).toEqual(['git.read'])
  })

  // The files on disk are what runs, so a catalog published before a pack
  // changed kind cannot keep describing something it no longer is.
  it('lets the pack on disk answer over the catalog', () => {
    const installed = {
      id: 'review',
      name: 'Review',
      version: '0.2.0',
      kind: 'extension' as const,
      path: '/packs/review',
      installedAt: 0,
      bytes: 1,
      triggers: [],
      actions: [],
      env: [],
      contributes: REVIEW
    }
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('review', 'Review', { kind: 'connector' })],
      [],
      [installed]
    )
    expect(listing.kind).toBe('extension')
    expect(kindOf(listing)).toBe('extension')
  })

  it('reads anything that says nothing as a connector', () => {
    const listings = buildConnectorListings(
      [builtIn('github', 'GitHub')],
      [catalogItem('kusto', 'Azure Data Explorer')],
      [],
      [],
      [{ id: 'playwright', name: 'Playwright', command: 'npx', args: [] }]
    )
    expect(listings.every((listing) => listing.kind === 'connector')).toBe(true)
  })

  it('offers only the kinds it actually has', () => {
    const connectorsOnly = buildConnectorListings([], [catalogItem('kusto', 'Kusto')], [])
    expect(connectorKinds(connectorsOnly)).toEqual(['connector'])

    const both = buildConnectorListings(
      [],
      [catalogItem('kusto', 'Kusto'), catalogItem('review', 'Review', { kind: 'extension' })],
      []
    )
    expect(connectorKinds(both)).toEqual(['connector', 'extension'])
    expect(filterByKind(both, 'extension').map((l) => l.id)).toEqual(['review'])
    expect(filterByKind(both, undefined)).toHaveLength(2)
  })

  // An extension is worth finding by what it adds, the way a connector is by what it triggers on.
  it('finds an extension by the name of a pane it adds', () => {
    const listings = buildConnectorListings(
      [],
      [
        catalogItem('review', 'Review', {
          kind: 'extension',
          description: 'Beside the terminal',
          contributes: REVIEW
        })
      ],
      []
    )
    expect(filterConnectorListings(listings, 'checks').map((l) => l.id)).toEqual(['review'])
  })

  // An extension is worth finding by what it reaches, said the way the page says it.
  it('finds an extension by what it asks to read', () => {
    const listings = buildConnectorListings(
      [],
      [
        catalogItem('review', 'Review', {
          kind: 'extension',
          contributes: REVIEW,
          permissions: ['terminal.read']
        })
      ],
      []
    )
    expect(filterConnectorListings(listings, 'terminal output').map((l) => l.id)).toEqual([
      'review'
    ])
  })

  // Saying "no triggers" about a thing that never has any would be a lie; it
  // says what it adds instead, and having said it, it is described.
  it('counts an extension that states its contributions as described', () => {
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('review', 'Review', { kind: 'extension', contributes: REVIEW })],
      []
    )
    expect(listingDetails(listing).known).toBe(true)
  })

  // Its kind is the answer; a catalog that has not listed its panes yet still
  // describes something, and the connector's "Add it to see" is unreachable here.
  it('counts an extension that lists nothing as described', () => {
    const [listing] = buildConnectorListings(
      [],
      [catalogItem('review', 'Review', { kind: 'extension' })],
      []
    )
    expect(listingDetails(listing).known).toBe(true)
  })
})

describe('an action carried before it is installed', () => {
  it('keeps the arguments a step will ask for', () => {
    const [listing] = buildConnectorListings(
      [],
      [
        catalogItem('slack', 'Slack', {
          actions: [
            {
              type: 'post',
              label: 'Post message',
              inputs: [{ key: 'text', label: 'Text', type: 'string', required: true }]
            }
          ]
        })
      ],
      []
    )

    expect(listingDetails(listing).actions[0].inputs).toEqual([
      { key: 'text', label: 'Text', type: 'string', required: true }
    ])
  })
})
