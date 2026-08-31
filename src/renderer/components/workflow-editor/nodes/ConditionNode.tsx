import { GitFork } from 'lucide-react'
import type {
  ConditionConfig,
  ConditionOperator,
  NodeExecutionStatus
} from '../../../../shared/types'
import { NODE_GLYPH, truncate } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

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
    <NodeShell
      icon={<GitFork size={18} strokeWidth={2} className={`${NODE_GLYPH} shrink-0`} />}
      label={label}
      subtitle={hasConfig ? OPERATOR_LABELS[config.operator] : 'Not configured'}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {preview && <NodeFooter mono>{truncate(preview, 60)}</NodeFooter>}
    </NodeShell>
  )
}
