import { GitBranch } from 'lucide-react'
import type {
  ConditionConfig,
  ConditionOperator,
  NodeExecutionStatus
} from '../../../../shared/types'
import { STATUS_DOT_CLASSES } from '../statusDot'

interface Props {
  label: string
  config: ConditionConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

const OPERATOR_LABELS: Record<ConditionOperator, string> = {
  equals: '=',
  notEquals: '!=',
  contains: 'contains',
  notContains: 'not contains',
  isEmpty: 'is empty',
  isNotEmpty: 'is not empty'
}

export function ConditionNode({ label, config, selected, executionStatus, onClick }: Props) {
  const hasConfig = config.variable && config.operator

  const preview = hasConfig
    ? `${config.variable} ${OPERATOR_LABELS[config.operator]}${
        config.operator !== 'isEmpty' && config.operator !== 'isNotEmpty' && config.value
          ? ` "${config.value}"`
          : ''
      }`
    : undefined

  return (
    <div
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={`relative px-3 py-2.5 rounded-md border w-[280px] transition-all cursor-pointer
                  ${selected ? 'border-blue-500/60 shadow-[0_0_0_3px_rgba(59,130,246,0.08)]' : 'border-white/[0.08]'}
                  bg-surface-node hover:bg-white/[0.02]`}
    >
      {executionStatus && STATUS_DOT_CLASSES[executionStatus] && (
        <span
          className={`absolute top-2 right-2 w-1.5 h-1.5 rounded-full ${STATUS_DOT_CLASSES[executionStatus]}`}
        />
      )}
      <div className="flex items-center gap-2">
        <GitBranch size={14} style={{ color: '#a855f7' }} strokeWidth={2} className="shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">
            {hasConfig ? OPERATOR_LABELS[config.operator] : 'Not configured'}
          </div>
        </div>
      </div>
      {preview && (
        <div className="mt-2 text-[11px] text-gray-600 truncate border-t border-white/[0.06] pt-2 font-mono">
          {preview.length > 60 ? preview.slice(0, 60) + '...' : preview}
        </div>
      )}
    </div>
  )
}
