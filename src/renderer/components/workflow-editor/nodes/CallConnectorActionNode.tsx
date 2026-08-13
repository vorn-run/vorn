import { Zap } from 'lucide-react'
import type { CallConnectorActionConfig, NodeExecutionStatus } from '../../../../shared/types'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../../lib/workflow-status'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import { NODE_SELECTED, NODE_UNSELECTED, NODE_GLYPH } from '../node-visuals'

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
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`relative px-3 py-2.5 rounded-sm border w-[280px] transition-all cursor-pointer
                  ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                  bg-surface-node hover:bg-white/[0.02]`}
    >
      {executionStatus && WORKFLOW_STATUS_DOT_PULSE[executionStatus] && (
        <span
          className={`absolute top-2 right-2 w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT_PULSE[executionStatus]}`}
        />
      )}
      <div className="flex items-center gap-2">
        {connectorId ? (
          <ConnectorIcon
            connectorId={connectorId}
            icon={icon}
            size={14}
            className={`${NODE_GLYPH} shrink-0`}
          />
        ) : (
          <Zap size={14} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
        )}
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">
            {config.action || 'Select action'}
          </div>
        </div>
      </div>
    </div>
  )
}
