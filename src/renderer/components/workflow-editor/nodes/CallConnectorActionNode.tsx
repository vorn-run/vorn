import { Zap } from 'lucide-react'
import type { CallConnectorActionConfig, NodeExecutionStatus } from '../../../../shared/types'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { NODE_GLYPH } from '../node-visuals'
import { NodeShell } from './NodeShell'

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
      subtitle={config.action || 'Select action'}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    />
  )
}
