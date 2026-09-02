import { useEffect, useState } from 'react'
import type { InstalledConnectorPack, SdkConnectorIcon, SourceConnection } from '../../shared/types'
import { connectionConnectorId } from '../../shared/types'
import { connectionIcon } from './connection-icon'

/**
 * Module-level cache of `window.api.listConnections()` shared across every
 * component that needs to resolve a connector id from a connection id.
 *
 * Nodes, sidebar rows, and setting panels used to each issue their own
 * `listConnections()` IPC call on mount — with many connector-seeded
 * workflows that became N IPC roundtrips. This cache fetches once, mirrors
 * it into any React component that subscribes via `useConnections()`, and
 * re-fetches on `config:changed` so new/deleted connections propagate.
 */

let cache: SourceConnection[] | null = null
/** Installed packs, so a glyph can come from the files that run rather than only the connection. */
let packCache: InstalledConnectorPack[] = []
const listeners = new Set<(c: SourceConnection[]) => void>()
let initPromise: Promise<void> | null = null
let unsubscribeConfigChange: (() => void) | null = null

async function refresh(): Promise<void> {
  const conns = await window.api.listConnections()
  // A missing pack list costs a glyph, never the connections themselves.
  packCache = (await window.api.listConnectorPacks?.().catch(() => [])) ?? []
  cache = conns
  for (const l of listeners) l(conns)
}

function ensureInit(): Promise<void> {
  if (initPromise) return initPromise
  initPromise = refresh().catch(() => {
    initPromise = null // let future mounts retry
  })
  // Subscribe once to config changes so the cache stays fresh.
  if (!unsubscribeConfigChange && typeof window.api?.onConfigChanged === 'function') {
    unsubscribeConfigChange = window.api.onConfigChanged(() => {
      void refresh()
    })
  }
  return initPromise
}

export function useConnections(): SourceConnection[] {
  const [value, setValue] = useState<SourceConnection[]>(cache ?? [])
  useEffect(() => {
    void ensureInit()
    listeners.add(setValue)
    // If the cache was populated before this component mounted, seed
    // synchronously rather than waiting for the next refresh tick.
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: seeds from a cache populated before this component mounted
    if (cache && value !== cache) setValue(cache)
    return () => {
      listeners.delete(setValue)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return value
}

/** Resolve a connectionId → connectorId via the cache. Returns null when
 *  the cache hasn't warmed up yet or the connection was deleted. */
export function useConnectorIdFor(connectionId: string | null | undefined): string | null {
  const connections = useConnections()
  if (!connectionId) return null
  const connection = connections.find((c) => c.id === connectionId)
  // The real id, not the `mcp` every packaged connector is stored under.
  return connection ? connectionConnectorId(connection) : null
}

/** How a connector is drawn: the id behind an `mcp` row, and the glyph it ships. */
export interface ConnectorLook {
  connectorId: string
  icon?: SdkConnectorIcon
}

/** The id and glyph behind a connection, for callers holding the connection list already. */
export function connectorLookFor(
  connections: SourceConnection[],
  connectionId: string | null | undefined
): ConnectorLook | undefined {
  if (!connectionId) return undefined
  const connection = connections.find((c) => c.id === connectionId)
  if (!connection) return undefined
  return { connectorId: connectionConnectorId(connection), icon: iconForConnection(connection) }
}

/** The same resolution as a hook, for surfaces that hold only a connection id. */
export function useConnectorLook(
  connectionId: string | null | undefined
): ConnectorLook | undefined {
  return connectorLookFor(useConnections(), connectionId)
}

/**
 * The glyph a connector ships, found by its id rather than a connection.
 *
 * A task records which connector it came from but not which connection, so the
 * installed pack is the only place its mark can come from.
 */
export function glyphForConnectorId(
  connectorId: string | null | undefined
): SdkConnectorIcon | undefined {
  if (!connectorId) return undefined
  return packCache.find((pack) => pack.id === connectorId)?.icon
}

/** `glyphForConnectorId` as a hook, so a row redraws once the packs load. */
export function useConnectorGlyph(
  connectorId: string | null | undefined
): SdkConnectorIcon | undefined {
  useConnections()
  return glyphForConnectorId(connectorId)
}

/**
 * Resolve a connectionId → the connector's own glyph, for the nodes and panels
 * that hold only a connection id. Undefined means "use the built-in icon".
 *
 * The connection records the glyph it was made with; the installed pack is what
 * actually runs. Falling back to the pack means a connector that gained an icon
 * in a later version stops drawing the generic MCP mark.
 */
export function useConnectionIconFor(
  connectionId: string | null | undefined
): SdkConnectorIcon | undefined {
  const connections = useConnections()
  if (!connectionId) return undefined
  return iconForConnection(connections.find((c) => c.id === connectionId))
}

/** The same resolution for callers that already hold the connection, such as a picker's options. */
export function iconForConnection(
  connection: SourceConnection | null | undefined
): SdkConnectorIcon | undefined {
  if (!connection) return undefined
  const stored = connectionIcon(connection)
  if (stored) return stored
  const connectorId = connectionConnectorId(connection)
  return packCache.find((pack) => pack.id === connectorId)?.icon
}

/** Re-read connections and packs now, for a caller that just made one. */
export async function refreshConnections(): Promise<void> {
  await refresh().catch(() => {})
}

/** Test hook — drop cached state so unit tests can start clean. */
export function __resetConnectionsCacheForTests(): void {
  cache = null
  packCache = []
  initPromise = null
  listeners.clear()
  if (unsubscribeConfigChange) {
    unsubscribeConfigChange()
    unsubscribeConfigChange = null
  }
}
