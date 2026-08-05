import type { ConnectorCatalogItem, SdkConnectorIcon, SourceConnection } from '../../shared/types'
import { connectionConnectorId, connectionIcon } from './connection-icon'

/**
 * One row in the connector list, whether it came from a built-in connector, a
 * package in the catalog, or a package someone installed by name.
 *
 * All three are presented identically on purpose: a person picking a connector
 * has no reason to care which of them runs in-process and which is installed
 * from a package, and asking them to look in two places is how a first-party
 * connector ends up harder to find than a third-party one.
 */
export interface ConnectorListing {
  key: string
  id: string
  name: string
  description?: string
  capabilities: string[]
  category: string
  /** Extra search terms, so a connector is findable by what it talks to. */
  keywords: string[]
  connectedCount: number
  /** Set when adding this one means installing a package. */
  catalogItem?: ConnectorCatalogItem
  /** Set for a packaged connector that is installed but not in the catalog. */
  icon?: SdkConnectorIcon
}

export interface BuiltInConnector {
  id: string
  name: string
  capabilities: string[]
}

const UNCATEGORIZED = 'Other'

/**
 * Merge the built-in connectors, the catalog and anything already installed
 * into a single list.
 *
 * Connected connectors sort first — the list is also how you find something
 * already set up — and the rest sort by name so position is stable as the
 * catalog grows rather than depending on registration order.
 */
export function buildConnectorListings(
  builtIns: BuiltInConnector[],
  catalog: ConnectorCatalogItem[],
  connections: SourceConnection[]
): ConnectorListing[] {
  const countFor = (id: string) =>
    connections.filter((conn) => connectionConnectorId(conn) === id).length

  const listings: ConnectorListing[] = [
    ...builtIns.map((c) => ({
      key: c.id,
      id: c.id,
      name: c.name,
      capabilities: c.capabilities,
      category: 'Built in',
      keywords: [],
      connectedCount: countFor(c.id)
    })),
    ...catalog.map((entry) => ({
      ...entry,
      key: `catalog:${entry.id}`,
      category: entry.category ?? UNCATEGORIZED,
      keywords: entry.keywords ?? [],
      connectedCount: countFor(entry.id),
      catalogItem: entry
    })),
    // A connector installed by package name, or one dropped from the catalog,
    // still has working connections. Without a row of its own it would be
    // missing from the list that is meant to show what is set up.
    ...installedListings(builtIns, catalog, connections)
  ]

  return listings.sort((a, b) => {
    if (a.connectedCount > 0 !== b.connectedCount > 0) return a.connectedCount > 0 ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

function installedListings(
  builtIns: BuiltInConnector[],
  catalog: ConnectorCatalogItem[],
  connections: SourceConnection[]
): ConnectorListing[] {
  const known = new Set([...builtIns.map((c) => c.id), ...catalog.map((c) => c.id)])
  const seen = new Map<string, ConnectorListing>()

  for (const conn of connections) {
    const id = connectionConnectorId(conn)
    if (known.has(id)) continue
    const existing = seen.get(id)
    if (existing) {
      existing.connectedCount += 1
      continue
    }
    seen.set(id, {
      key: `installed:${id}`,
      id,
      name: id,
      capabilities: [],
      category: 'Installed',
      keywords: [],
      connectedCount: 1,
      icon: connectionIcon(conn)
    })
  }

  return [...seen.values()]
}

/**
 * Narrow the list to what someone typed.
 *
 * Matches name, description, capabilities and keywords, so a connector is
 * reachable by what it does ("query", "logs") and not only by a product name
 * you would have to already know.
 */
export function filterConnectorListings(
  listings: ConnectorListing[],
  query: string
): ConnectorListing[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return listings

  return listings.filter((listing) => {
    const haystack = [
      listing.name,
      listing.description ?? '',
      listing.category,
      ...listing.capabilities,
      ...listing.keywords
    ]
      .join(' ')
      .toLowerCase()
    // Every term must match, so typing more narrows rather than widens.
    return terms.every((term) => haystack.includes(term))
  })
}

/** Listings grouped under their category, in the order the listings arrive. */
export function groupConnectorListings(
  listings: ConnectorListing[]
): Array<{ category: string; listings: ConnectorListing[] }> {
  const groups = new Map<string, ConnectorListing[]>()
  for (const listing of listings) {
    const existing = groups.get(listing.category)
    if (existing) existing.push(listing)
    else groups.set(listing.category, [listing])
  }
  return [...groups].map(([category, entries]) => ({ category, listings: entries }))
}
