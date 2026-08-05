import type { SdkConnectorIcon, SourceConnection } from '../../shared/types'

/**
 * The `filters` keys a packaged connector writes about itself.
 *
 * A packaged connector is stored as an `mcp` connection, so everything that
 * distinguishes it from any other MCP server lives in `filters`. Naming the
 * keys once keeps the writer and the several readers from drifting apart,
 * where a mismatch shows up as a silently wrong icon or count rather than an
 * error.
 */
export const SDK_FILTER_KEYS = {
  connectorId: 'sdkConnectorId',
  version: 'sdkVersion',
  icon: 'sdkIcon'
} as const

/**
 * Which connector a connection belongs to.
 *
 * For a packaged connector this is not `connectorId` — that is `mcp` for all
 * of them, so counting by it would credit every packaged connector to every
 * other.
 */
export function connectionConnectorId(connection: {
  connectorId: string
  filters: SourceConnection['filters']
}): string {
  const packaged = connection.filters?.[SDK_FILTER_KEYS.connectorId]
  return typeof packaged === 'string' && packaged !== '' ? packaged : connection.connectorId
}

/**
 * The glyph a connection should show.
 *
 * A connector installed from a package is stored as an `mcp` connection, so
 * its own icon travels on the connection rather than being keyed by connector
 * id like the built-in ones.
 *
 * Re-validated on read: the value was written by a third-party manifest, and
 * a connection created by an older build could hold anything.
 */
export function connectionIcon(
  connection: { filters: SourceConnection['filters'] } | null | undefined
): SdkConnectorIcon | undefined {
  const raw = connection?.filters?.[SDK_FILTER_KEYS.icon]
  if (typeof raw !== 'string' || raw === '') return undefined
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return undefined
    const { viewBox, paths } = parsed as Partial<SdkConnectorIcon>
    if (!Array.isArray(paths) || paths.length === 0) return undefined
    if (!paths.every((d) => typeof d === 'string' && d !== '')) return undefined
    return { viewBox: typeof viewBox === 'string' ? viewBox : '0 0 24 24', paths }
  } catch {
    return undefined
  }
}
