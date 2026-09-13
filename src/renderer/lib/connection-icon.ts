import type { SdkConnectorIcon, SourceConnection } from '../../shared/types'
import { SDK_FILTER_KEYS } from '../../shared/types'

export { SDK_CONNECTOR_ID, SDK_FILTER_KEYS, connectionConnectorId } from '../../shared/types'

/** The glyph a package's connection carries, re-validated on read because a third-party manifest wrote it. */
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
