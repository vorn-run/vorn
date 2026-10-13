import { Zap } from 'lucide-react'
import type { CallConnectorActionConfig, NodeExecutionStatus } from '../../../../shared/types'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { connectorArgsPreview, NODE_GLYPH, truncate } from '../node-visuals'
import { NodeFooter, NodeShell } from './NodeShell'

interface Props {
  label: string
  config: CallConnectorActionConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

export function CallConnectorActionNode({
  label,
  config,
  selected,
  executionStatus,
  onClick
}: Props) {
  // Uses the shared connections cache — no IPC call per node render.
  const connectorId = useConnectorIdFor(config.connectionId)
  const icon = useConnectionIconFor(config.connectionId)
  // Drawn exactly when the height estimate charges for it, or the edge below detaches.
  const preview = connectorArgsPreview(config.args)

  return (
    <NodeShell
      icon={
        connectorId ? (
          // A brand mark identifies the step; it gets full ink and a step up in size.
          <ConnectorIcon
            connectorId={connectorId}
            icon={icon}
            size={18}
            className="text-ink shrink-0"
          />
        ) : (
          <Zap size={18} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
        )
      }
      label={label}
      subtitle={config.actionLabel || config.action || 'Select action'}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {preview && <NodeFooter mono>{truncate(preview, 50)}</NodeFooter>}
    </NodeShell>
  )
}
