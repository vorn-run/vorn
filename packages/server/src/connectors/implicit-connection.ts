import {
  SDK_FILTER_KEYS,
  connectionConnectorId,
  isImplicitConnection,
  type InstalledConnectorPack,
  type SourceConnection
} from '@vornrun/shared/types'

// A rung-none connector is connected by installing it; the app makes that one connection and withdraws it with the pack.
export interface ImplicitConnectionDeps {
  list: () => SourceConnection[]
  create: (params: {
    connectorId: string
    name: string
    filters: Record<string, unknown>
    syncIntervalMinutes: number
    statusMapping: Record<string, never>
  }) => SourceConnection
  remove: (connectionId: string) => void
  /** Told only when something actually changed. */
  changed: () => void
}

/** Settle the connection for what is installed now; returns the one it made, if any. */
export function syncImplicitConnection(
  connectorId: string,
  pack: InstalledConnectorPack | undefined,
  deps: ImplicitConnectionDeps,
  mcpConnectorId = 'mcp'
): SourceConnection | undefined {
  const existing = deps.list().filter((conn) => connectionConnectorId(conn) === connectorId)

  if (!pack || pack.auth?.rung !== 'none') {
    const implicit = existing.filter(isImplicitConnection)
    for (const conn of implicit) deps.remove(conn.id)
    if (implicit.length > 0) deps.changed()
    return undefined
  }

  if (existing.length > 0) return undefined

  return deps.create({
    connectorId: mcpConnectorId,
    name: pack.name,
    filters: {
      [SDK_FILTER_KEYS.connectorId]: pack.id,
      [SDK_FILTER_KEYS.version]: pack.version,
      ...(pack.icon && { [SDK_FILTER_KEYS.icon]: JSON.stringify(pack.icon) }),
      [SDK_FILTER_KEYS.implicit]: true
    },
    // Nothing polls a connector that only serves actions.
    syncIntervalMinutes: 0,
    statusMapping: {}
  })
}
