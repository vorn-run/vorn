import type {
  ConnectorAuthRung,
  ConnectorCatalogActionInput,
  ConnectorCatalogItem,
  ConnectorCatalogVerification,
  ConnectorKind,
  ConnectorPackSummary,
  ExtensionActivation,
  ExtensionContributions,
  ExtensionPermission,
  InstalledConnectorPack,
  McpServerCatalogEntry,
  SdkConnectorIcon,
  SourceConnection
} from '../../shared/types'
import { isImplicitConnection } from '../../shared/types'
import { connectionConnectorId, connectionIcon } from './connection-icon'
import { EXTENSION_PERMISSION } from './extension-copy'

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
  /** Whether this polls a service or contributes to a card. A built-in is always a connector. */
  kind: ConnectorKind
  /** `installed` is a pack the catalog does not carry; it brings its own manifest. */
  source: 'builtin' | 'catalog' | 'installed' | 'mcp'
  /** Set for an MCP server, which is a launch line rather than a package. */
  mcpServer?: McpServerCatalogEntry
  /** Extra search terms, so a connector is findable by what it talks to. */
  keywords: string[]
  connectedCount: number
  /** Set when adding this one means installing a package. */
  catalogItem?: ConnectorCatalogItem
  /** Set for a packaged connector that is installed but not in the catalog. */
  icon?: SdkConnectorIcon
  /** The pack on disk, when this connector has been installed as one. */
  pack?: InstalledConnectorPack
  // What setting this one up will ask of you.
  authRung?: ConnectorAuthRung
  /** What the factory checked, when this connector has been through it. */
  verified?: ConnectorCatalogVerification
  // Connected without anyone being asked to connect it — a connector that signs in with nothing is ready the moment it.
  implicitlyConnected?: boolean
  /** What an extension adds to a card, what it may touch, and where it shows. */
  contributes?: ExtensionContributions
  permissions?: ExtensionPermission[]
  activates?: ExtensionActivation
}

/**
 * What a row is, deciding between the pack on disk and the catalog's claim.
 *
 * The installed files answer for themselves, the same precedence the sign-in
 * rung already follows: a catalog published before a pack changed kind would
 * otherwise describe something that is no longer what it says.
 */
export function kindOf(listing: {
  pack?: InstalledConnectorPack
  catalogItem?: { kind?: ConnectorKind }
}): ConnectorKind {
  return listing.pack?.kind ?? listing.catalogItem?.kind ?? 'connector'
}

// Whether a checked pack is the one the row already described, down to the version, the sign-in and the receipt.
export function matchesListing(preview: ConnectorPackSummary, listing: ConnectorListing): boolean {
  return (
    preview.id === listing.id &&
    preview.version === listing.catalogItem?.version &&
    // An absent rung on either side is unknown rather than none, and unknown is what the sheet is for.
    preview.auth?.rung !== undefined &&
    preview.auth.rung === listing.authRung &&
    listing.verified?.version === preview.version &&
    preview.installedVersion === undefined
  )
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
  /** Arguments ride along, so a step can be offered before anything is installed. */
  actions: Array<{
    type: string
    label: string
    description?: string
    inputs?: ConnectorCatalogActionInput[]
  }>
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
  // A pack's manifest is what the installed files serve, so it beats the catalog's copy.
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
    // nothing rather than saying "no triggers", which would be a lie. An
    // extension states what it adds instead, and having said it is known.
    if (!entry?.triggers && !entry?.actions) {
      return listing.kind === 'extension' || listing.contributes
        ? { ...EMPTY_DETAILS, known: true }
        : EMPTY_DETAILS
    }
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
  packs: InstalledConnectorPack[] = [],
  mcpServers: McpServerCatalogEntry[] = []
): ConnectorListing[] {
  // One pass over the connections; an implicit one was never asked for, so it is not counted as "another".
  const owned = new Map<string, { count: number; implicit: boolean }>()
  for (const conn of connections) {
    const id = connectionConnectorId(conn)
    const entry = owned.get(id) ?? { count: 0, implicit: false }
    if (isImplicitConnection(conn)) entry.implicit = true
    else entry.count += 1
    owned.set(id, entry)
  }
  const countFor = (id: string) => owned.get(id)?.count ?? 0
  const implicitFor = (id: string) => owned.get(id)?.implicit ?? false
  const packFor = (id: string) => packs.find((pack) => pack.id === id)

  const listings: ConnectorListing[] = [
    ...builtIns.map((c) => ({
      key: c.id,
      id: c.id,
      name: c.name,
      capabilities: c.capabilities,
      category: 'Built in',
      kind: 'connector' as const,
      source: 'builtin' as const,
      keywords: [],
      connectedCount: countFor(c.id),
      implicitlyConnected: implicitFor(c.id)
    })),
    ...catalog.map((entry) => {
      const pack = packFor(entry.id)
      return {
        ...entry,
        key: `catalog:${entry.id}`,
        category: entry.category ?? UNCATEGORIZED,
        kind: kindOf({ ...(pack && { pack }), catalogItem: entry }),
        source: 'catalog' as const,
        keywords: entry.keywords ?? [],
        connectedCount: countFor(entry.id),
        implicitlyConnected: implicitFor(entry.id),
        catalogItem: entry,
        // The entry above says what installing would bring; the files on disk
        // answer for themselves, so each of these overrides it where it exists.
        ...(pack?.auth?.rung !== undefined && { authRung: pack.auth.rung }),
        ...(pack?.contributes !== undefined && { contributes: pack.contributes }),
        ...(pack?.permissions !== undefined && { permissions: pack.permissions }),
        ...(pack?.activates !== undefined && { activates: pack.activates }),
        ...(pack && { pack })
      }
    }),
    // Side-loaded, or published since the last fetch; it describes itself either way.
    // A pack sharing a built-in's id is refused at install, and skipped here too
    // so an older one on disk cannot draw a second row beside the connector it shadows.
    ...packs
      .filter(
        (pack) =>
          !catalog.some((entry) => entry.id === pack.id) &&
          !builtIns.some((builtIn) => builtIn.id === pack.id)
      )
      .map((pack) => ({
        key: `installed:${pack.id}`,
        id: pack.id,
        name: pack.name,
        ...(pack.description !== undefined && { description: pack.description }),
        capabilities: packCapabilities(pack),
        category: 'Installed',
        kind: kindOf({ pack }),
        source: 'installed' as const,
        keywords: [],
        connectedCount: countFor(pack.id),
        implicitlyConnected: implicitFor(pack.id),
        ...(pack.auth?.rung !== undefined && { authRung: pack.auth.rung }),
        ...(pack.icon !== undefined && { icon: pack.icon }),
        ...(pack.contributes !== undefined && { contributes: pack.contributes }),
        ...(pack.permissions !== undefined && { permissions: pack.permissions }),
        ...(pack.activates !== undefined && { activates: pack.activates }),
        pack
      })),
    // A server Vorn starts and speaks MCP to. Its connections are `mcp` rows
    // stamped with the server's id, which is what this counts against.
    ...mcpServers.map((server) => ({
      key: `mcp:${server.id}`,
      id: server.id,
      name: server.name,
      ...(server.description !== undefined && { description: server.description }),
      capabilities: ['actions'],
      category: server.category ?? 'MCP servers',
      kind: 'connector' as const,
      source: 'mcp' as const,
      keywords: server.keywords ?? [],
      connectedCount: countFor(server.id),
      implicitlyConnected: implicitFor(server.id),
      mcpServer: server
    }))
  ]

  // Usable-now sorts first, whether that took a connection or nothing at all.
  const ready = (listing: ConnectorListing): boolean =>
    listing.connectedCount > 0 || listing.implicitlyConnected === true
  return listings.sort((a, b) => {
    if (ready(a) !== ready(b)) return ready(a) ? -1 : 1
    return a.name.localeCompare(b.name)
  })
}

/** A heading and the connectors under it, for a list too long to scan flat. */
export interface ListingSection {
  category: string
  listings: ConnectorListing[]
}

// Cut the list into the sections it already sorts itself into.
export function groupListingsByCategory(listings: ConnectorListing[]): ListingSection[] {
  const sections = new Map<string, ConnectorListing[]>()
  for (const listing of listings) {
    const existing = sections.get(listing.category)
    if (existing) existing.push(listing)
    else sections.set(listing.category, [listing])
  }
  return [...sections].map(([category, entries]) => ({ category, listings: entries }))
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
 *
 * A connection the app made for a connector that signs in with nothing is left
 * out: nobody chose it, there is nothing in it to edit, and deleting it would
 * only break the connector it was made for.
 */
export function groupConnections(
  connections: SourceConnection[],
  listings: ConnectorListing[] = []
): ConnectionGroup[] {
  const byConnector = new Map<string, ConnectionGroup>()

  for (const conn of connections.filter((connection) => !isImplicitConnection(connection))) {
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
    const contributes = listing.contributes
    const haystack = [
      listing.name,
      listing.description ?? '',
      listing.category,
      ...listing.capabilities,
      ...listing.keywords,
      // An extension is worth finding by what it adds, the way a connector is by what it triggers on.
      ...[
        ...(contributes?.panes ?? []),
        ...(contributes?.footers ?? []),
        ...(contributes?.linkHandlers ?? [])
      ].map((contribution) => contribution.title),
      // And by what it reaches, in the words the page uses rather than the manifest's.
      ...(listing.permissions ?? []).map((name) => EXTENSION_PERMISSION[name].phrase)
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

// What a rung asks of you, from the reader's side: a filter label, a row badge, and the detail page's sentence.
export const AUTH_RUNG: Record<
  ConnectorAuthRung,
  { label: string; badge: string; detail: string }
> = {
  none: {
    label: 'Needs no sign-in',
    badge: 'no sign-in',
    detail: 'Nothing — ready as soon as it is installed.'
  },
  cli: {
    label: 'Signs in with a CLI',
    badge: 'CLI login',
    detail: 'Signs in with a CLI'
  },
  key: { label: 'Needs a key', badge: 'key', detail: 'Needs a key' },
  browser: {
    label: 'Signs in through a Vorn window',
    badge: 'browser',
    detail: 'Signs in through a Vorn window'
  },
  oauth: {
    label: 'Signs in with OAuth',
    badge: 'OAuth',
    detail: 'Signs in with OAuth'
  }
}

/** The rungs actually present, so the filter never offers an empty answer. */
export function connectorAuthRungs(listings: ConnectorListing[]): ConnectorAuthRung[] {
  const order: ConnectorAuthRung[] = ['none', 'cli', 'key', 'browser', 'oauth']
  return order.filter((rung) => listings.some((listing) => listing.authRung === rung))
}

/** What a kind is called where a person picks one. */
export const CONNECTOR_KIND: Record<ConnectorKind, { label: string; badge?: string }> = {
  connector: { label: 'Connectors' },
  // Only the extension wears a badge; a row without one is the ordinary case.
  extension: { label: 'Extensions', badge: 'extension' }
}

/** The kinds actually present, so the filter never offers an empty answer. */
export function connectorKinds(listings: ConnectorListing[]): ConnectorKind[] {
  const order: ConnectorKind[] = ['connector', 'extension']
  return order.filter((kind) => listings.some((listing) => listing.kind === kind))
}

/** Narrow to what polls a service, or to what shows on a card. */
export function filterByKind(
  listings: ConnectorListing[],
  kind: ConnectorKind | undefined
): ConnectorListing[] {
  if (!kind) return listings
  return listings.filter((listing) => listing.kind === kind)
}

/** Narrow to the connectors that sign in a particular way. */
export function filterByAuthRung(
  listings: ConnectorListing[],
  rung: ConnectorAuthRung | undefined
): ConnectorListing[] {
  if (!rung) return listings
  return listings.filter((listing) => listing.authRung === rung)
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
