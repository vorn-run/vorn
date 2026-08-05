import type { SdkConnectorIcon, SourceConnection } from '../../shared/types'
import { SDK_FILTER_KEYS } from '../../shared/types'

export { SDK_FILTER_KEYS, connectionConnectorId } from '../../shared/types'

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
