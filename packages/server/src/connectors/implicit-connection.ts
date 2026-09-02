import {
  SDK_FILTER_KEYS,
  connectionConnectorId,
  type InstalledConnectorPack,
  type SourceConnection
} from '@vornrun/shared/types'

/**
 * The connection a connector that needs no sign-in is used through.
 *
 * A `none` rung has nothing to ask anybody, so a form would be an empty
 * question and "Add connection" a step with no content. Installing such a
 * connector makes the one connection its steps bind to; removing it takes that
 * connection with it. Steps still address a connection id underneath — the app
 * made it, nobody had to.
 *
 * Driven by what is on disk rather than by which operation happened, so the
 * same call settles an install, an update that changed the rung, and a removal.
 */
export interface ImplicitConnectionDeps {
  /** What is installed now, or nothing once the files are gone. */
  describe: (connectorId: string) => InstalledConnectorPack | undefined
  list: () => SourceConnection[]
  create: (params: {
    connectorId: string
    name: string
    filters: Record<string, unknown>
    syncIntervalMinutes: number
    statusMapping: Record<string, never>
  }) => unknown
  remove: (connectionId: string) => void
  /** Told only when something actually changed. */
  changed: () => void
}

/** Whether the app made this connection itself, rather than a person. */
export function isImplicit(conn: SourceConnection): boolean {
  return conn.filters.implicit === true
}

export function syncImplicitConnection(
  connectorId: string,
  deps: ImplicitConnectionDeps,
  mcpConnectorId = 'mcp'
): void {
  let pack: InstalledConnectorPack | undefined
  try {
    pack = deps.describe(connectorId)
  } catch {
    // No data directory yet: nothing is installed, so there is nothing to keep.
    return
  }

  const existing = deps.list().filter((conn) => connectionConnectorId(conn) === connectorId)

  // Gone, or no longer a rung that connects itself: the connection the app made
  // is the app's to withdraw. One somebody made by hand is not.
  if (!pack || pack.auth?.rung !== 'none') {
    const implicit = existing.filter(isImplicit)
    for (const conn of implicit) deps.remove(conn.id)
    if (implicit.length > 0) deps.changed()
    return
  }

  // Already connected — by the app or by hand — is already answered.
  if (existing.length > 0) return

  deps.create({
    connectorId: mcpConnectorId,
    name: pack.name,
    filters: {
      [SDK_FILTER_KEYS.connectorId]: pack.id,
      [SDK_FILTER_KEYS.version]: pack.version,
      ...(pack.icon && { [SDK_FILTER_KEYS.icon]: JSON.stringify(pack.icon) }),
      // Marks the connection as the app's own, so a list can leave it out of
      // rows that are about connections somebody chose to make.
      implicit: true
    },
    // Nothing polls a connector that only serves actions; a trigger-bearing one
    // is connected by hand, where the schedule is a question worth asking.
    syncIntervalMinutes: 0,
    statusMapping: {}
  })
}
