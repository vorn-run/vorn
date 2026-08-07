import { ConnectorIcon } from '../ConnectorIcon'
import { ICON_MAP } from '../project-sidebar/icon-map'
import type { RunPresentation } from '../../lib/run-presentation'

const DEFAULT_ICON_COLOR = '#6b7280'

/**
 * Glyph for a run. A workflow's own icon and colour win — that is the mark the
 * sidebar shows, so a run stays recognisable as belonging to it. Connector runs
 * of a workflow with no icon fall back to the connector's brand mark, and a
 * deleted workflow falls back to a glyph for how the run was triggered.
 */
export function RunIcon({
  presentation,
  size = 13,
  className
}: {
  presentation: RunPresentation
  size?: number
  className?: string
}) {
  const WorkflowIcon = presentation.iconName ? ICON_MAP[presentation.iconName] : undefined
  if (WorkflowIcon) {
    return (
      <WorkflowIcon
        size={size}
        color={presentation.iconColor || DEFAULT_ICON_COLOR}
        strokeWidth={1.5}
      />
    )
  }

  if (presentation.connectorId) {
    return (
      <ConnectorIcon
        connectorId={presentation.connectorId}
        size={size}
        className={className ?? 'text-gray-400'}
      />
    )
  }

  const Fallback = presentation.fallbackIcon
  return <Fallback size={size} strokeWidth={1.5} className={className ?? 'text-gray-400'} />
}
