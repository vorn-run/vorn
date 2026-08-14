import { Hand } from 'lucide-react'
import type { ApprovalConfig, NodeExecutionStatus } from '../../../../shared/types'
import { NODE_GLYPH, truncate } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

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
    <NodeShell
      icon={<Hand size={14} className={`shrink-0 ${NODE_GLYPH}`} strokeWidth={2} />}
      label={label}
      subtitle={subtitle}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {config.message && <NodeFooter>{truncate(config.message, 60)}</NodeFooter>}
    </NodeShell>
  )
}
