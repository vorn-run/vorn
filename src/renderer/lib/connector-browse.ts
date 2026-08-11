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
  /**
   * Where the row came from, which decides what "Add" can do.
   *
   * An `installed` row is a connector we can see connections for but hold no
   * manifest or package spec for, so there is nothing to open a form against.
   */
  source: 'builtin' | 'catalog' | 'installed'
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
  manifest?: {
    triggers?: Array<{ type: string; label: string; description?: string }>
    actions?: Array<{ type: string; label: string; description?: string }>
    auth?: Array<{ key?: string; label?: string; required?: boolean }>
  }
}

/**
 * What a connector does, in the one shape the list renders.
 *
 * A built-in describes itself with `ConnectorTriggerDef`, a packaged one
 * through its catalog entry, and they already agree on type/label/description.
 * Reducing both here is what lets one card and one panel serve every row rather
 * than branching on where the connector came from.
 */
export interface ConnectorDetails {
  triggers: Array<{ type: string; label: string; description?: string }>
  actions: Array<{ type: string; label: string; description?: string }>
  settings: Array<{ name: string; required: boolean; description?: string }>
  /** Absent on a row nothing describes — an installed package with no manifest. */
  known: boolean
}

const EMPTY_DETAILS: ConnectorDetails = {
  triggers: [],
  actions: [],
  settings: [],
  known: false
}

/**
 * Read what a connector does, whatever kind of row it is.
 *
 * For a packaged connector this comes from the catalog, which is generated from
 * the connector's own manifest upstream — so it is safe to show before anything
 * is installed and cannot drift from what gets installed. A built-in is read
 * from the manifest already in the process.
 */
export function listingDetails(
  listing: ConnectorListing,
  builtIns: BuiltInConnector[] = []
): ConnectorDetails {
  if (listing.source === 'catalog') {
    const entry = listing.catalogItem
    // An entry from a catalog published before these fields existed says
    // nothing rather than saying "no triggers", which would be a lie.
    if (!entry?.triggers && !entry?.actions) return EMPTY_DETAILS
    return {
      triggers: entry.triggers ?? [],
      actions: entry.actions ?? [],
      settings: (entry.env ?? []).map((variable) => ({
        name: variable.name,
        required: variable.required,
        ...(variable.description && { description: variable.description })
      })),
      known: true
    }
  }

  const builtIn = builtIns.find((connector) => connector.id === listing.id)
  if (!builtIn?.manifest) return EMPTY_DETAILS
  return {
    triggers: builtIn.manifest.triggers ?? [],
    actions: builtIn.manifest.actions ?? [],
    settings: (builtIn.manifest.auth ?? []).map((field) => ({
      name: field.label ?? field.key ?? '',
      required: field.required ?? false
    })),
    known: true
  }
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
      source: 'builtin' as const,
      keywords: [],
      connectedCount: countFor(c.id)
    })),
    ...catalog.map((entry) => ({
      ...entry,
      key: `catalog:${entry.id}`,
      category: entry.category ?? UNCATEGORIZED,
      source: 'catalog' as const,
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
      source: 'installed' as const,
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

/**
 * The facets the list offers, in the order they are worth offering.
 *
 * Derived from what is actually present rather than a fixed list, so a category
 * arriving with a new connector shows up without an app release — the whole
 * point of fetching the catalog. "Callable" and "Connected" are not categories
 * but answer the two questions people bring to this page most often: can a step
 * use it, and what do I already have.
 */
export const CALLABLE_FILTER = 'Can be called from a step'
export const CONNECTED_FILTER = 'Connected'

export function connectorCategories(listings: ConnectorListing[]): string[] {
  const categories = [...new Set(listings.map((listing) => listing.category))].sort()
  const facets: string[] = []
  if (listings.some((listing) => listing.capabilities.includes('actions'))) {
    facets.push(CALLABLE_FILTER)
  }
  if (listings.some((listing) => listing.connectedCount > 0)) facets.push(CONNECTED_FILTER)
  return [...categories, ...facets]
}

/** Narrow to one category, or to one of the two derived facets. */
export function filterByCategory(
  listings: ConnectorListing[],
  category: string | undefined
): ConnectorListing[] {
  if (!category) return listings
  if (category === CALLABLE_FILTER) {
    return listings.filter((listing) => listing.capabilities.includes('actions'))
  }
  if (category === CONNECTED_FILTER) {
    return listings.filter((listing) => listing.connectedCount > 0)
  }
  return listings.filter((listing) => listing.category === category)
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

/**
 * How current the catalog is, in the terms someone would ask it.
 *
 * Never fetched is worth saying plainly rather than dressing up: the list is
 * then whatever shipped with the app, which may be missing everything published
 * since, and a soothing timestamp would hide that.
 */
export function describeCatalogAge(
  fetchedAt: number | undefined,
  now: number = Date.now()
): string {
  if (fetchedAt === undefined) return 'Showing the connectors that shipped with Vorn'
  const minutes = Math.floor((now - fetchedAt) / 60_000)
  if (minutes < 1) return 'Updated just now'
  if (minutes < 60) return `Updated ${plural(minutes, 'minute')} ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `Updated ${plural(hours, 'hour')} ago`
  return `Updated ${plural(Math.floor(hours / 24), 'day')} ago`
}

function plural(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? '' : 's'}`
}
