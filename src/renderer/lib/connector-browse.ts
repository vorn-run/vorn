import type {
  ConnectorCatalogItem,
  InstalledConnectorPack,
  SdkConnectorIcon,
  SourceConnection
} from '../../shared/types'
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
   * An `installed` row is a pack on disk the catalog does not carry — dropped
   * in from a file, or published since the last fetch. It brings its own
   * manifest, so it describes itself and can be connected to like any other.
   */
  source: 'builtin' | 'catalog' | 'installed'
  /** Extra search terms, so a connector is findable by what it talks to. */
  keywords: string[]
  connectedCount: number
  /** Set when adding this one means installing a package. */
  catalogItem?: ConnectorCatalogItem
  /** Set for a packaged connector that is installed but not in the catalog. */
  icon?: SdkConnectorIcon
  /** The pack on disk, when this connector has been installed as one. */
  pack?: InstalledConnectorPack
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
  // A pack on disk is the one source that cannot be stale: its manifest is the
  // one the installed files serve, so it is preferred over the catalog's copy.
  if (listing.pack) {
    return {
      triggers: listing.pack.triggers.map((trigger) => ({
        type: trigger.type,
        label: trigger.label,
        ...(trigger.description !== undefined && { description: trigger.description })
      })),
      actions: listing.pack.actions,
      settings: listing.pack.env.map((variable) => ({
        name: variable.name,
        required: variable.required,
        ...(variable.description !== undefined && { description: variable.description })
      })),
      known: true
    }
  }

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

/** What a pack offers, in the vocabulary the catalog uses for the same thing. */
function packCapabilities(pack: InstalledConnectorPack): string[] {
  return [
    ...(pack.triggers.length > 0 ? ['triggers'] : []),
    ...(pack.actions.length > 0 ? ['actions'] : [])
  ]
}

/**
 * The connectors that can be added: the built-ins and the catalog.
 *
 * Only kinds of connector, never the connections someone already made — those
 * are a different noun with a list of their own, and mixing them is how one
 * connector ends up on a page three times.
 *
 * Connected ones sort first, since a connector you already use is the one you
 * are most likely to want another of, and the rest sort by name so position is
 * stable as the catalog grows rather than depending on registration order.
 */
export function buildConnectorListings(
  builtIns: BuiltInConnector[],
  catalog: ConnectorCatalogItem[],
  connections: SourceConnection[],
  packs: InstalledConnectorPack[] = []
): ConnectorListing[] {
  const countFor = (id: string) =>
    connections.filter((conn) => connectionConnectorId(conn) === id).length
  const packFor = (id: string) => packs.find((pack) => pack.id === id)

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
      catalogItem: entry,
      ...(packFor(entry.id) && { pack: packFor(entry.id) })
    })),
    // A pack the catalog does not carry — side-loaded from a file, or published
    // after the last catalog fetch. It describes itself, so unlike the rows this
    // arm was written for it can be shown in full rather than as a bare id.
    ...packs
      .filter((pack) => !catalog.some((entry) => entry.id === pack.id))
      .map((pack) => ({
        key: `installed:${pack.id}`,
        id: pack.id,
        name: pack.name,
        ...(pack.description !== undefined && { description: pack.description }),
        capabilities: packCapabilities(pack),
        category: 'Installed',
        source: 'installed' as const,
        keywords: [],
        connectedCount: countFor(pack.id),
        ...(pack.icon !== undefined && { icon: pack.icon }),
        pack
      }))
  ]

  return listings.sort((a, b) => {
    if (a.connectedCount > 0 !== b.connectedCount > 0) return a.connectedCount > 0 ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

/** A connector someone actually uses, with the connections they made to it. */
export interface ConnectionGroup {
  connectorId: string
  name: string
  icon?: SdkConnectorIcon
  version?: string
  /** Absent for a package installed by name, or one the catalog has dropped. */
  listing?: ConnectorListing
  connections: SourceConnection[]
}

/**
 * Group connections under the connector they belong to.
 *
 * Two connections to the same connector are one heading with two rows under it,
 * which is the only place a count belongs — beside the things it counts, rather
 * than on a card in the catalog that is trying to sell you a third.
 *
 * A connection whose connector is in neither the catalog nor the built-ins is
 * still a connection: it polls, it fails, it can be deleted. It gets a group
 * named after its connector id, with the icon the connection itself stored.
 * Groups keep the order the connections arrive in, which is the order the
 * server returns them.
 */
export function groupConnections(
  connections: SourceConnection[],
  listings: ConnectorListing[] = []
): ConnectionGroup[] {
  const byConnector = new Map<string, ConnectionGroup>()

  for (const conn of connections) {
    const id = connectionConnectorId(conn)
    const existing = byConnector.get(id)
    if (existing) {
      existing.connections.push(conn)
      continue
    }

    const listing = listings.find((entry) => entry.id === id)
    byConnector.set(id, {
      connectorId: id,
      name: listing?.name ?? id,
      ...((listing?.catalogItem?.icon ?? connectionIcon(conn))
        ? { icon: listing?.catalogItem?.icon ?? connectionIcon(conn) }
        : {}),
      ...(listing?.catalogItem?.version && { version: listing.catalogItem.version }),
      ...(listing && { listing }),
      connections: [conn]
    })
  }

  return [...byConnector.values()]
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
