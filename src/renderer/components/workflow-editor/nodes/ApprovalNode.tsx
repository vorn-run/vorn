import { Hand } from 'lucide-react'
import type { ApprovalConfig, NodeExecutionStatus } from '../../../../shared/types'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../../lib/workflow-status'
import { NODE_SELECTED, NODE_UNSELECTED, NODE_GLYPH } from '../node-visuals'

interface Props {
  label: string
  config: ApprovalConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

export function ApprovalNode({ label, config, selected, executionStatus, onClick }: Props) {
  const subtitle = config.timeoutMs
    ? `Waits for approval · ${Math.round(config.timeoutMs / 1000)}s timeout`
    : 'Waits for approval'

  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`relative px-3 py-2.5 rounded-md border w-[280px] transition-all cursor-pointer
                  ${selected ? NODE_SELECTED : NODE_UNSELECTED}
                  bg-surface-node hover:bg-white/[0.02]`}
    >
      {executionStatus && WORKFLOW_STATUS_DOT_PULSE[executionStatus] && (
        <span
          className={`absolute top-2 right-2 w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT_PULSE[executionStatus]}`}
        />
      )}
      <div className="flex items-center gap-2">
        <Hand size={14} className={`shrink-0 ${NODE_GLYPH}`} strokeWidth={2} />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">{subtitle}</div>
        </div>
      </div>
      {config.message && (
        <div className="mt-2 text-[11px] text-gray-600 truncate border-t border-white/[0.06] pt-2">
          {config.message.length > 60 ? config.message.slice(0, 60) + '...' : config.message}
        </div>
      )}
    </div>
  )
}
