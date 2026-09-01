import { Globe } from 'lucide-react'
import type { HttpRequestConfig, NodeExecutionStatus } from '../../../../shared/types'
import { NODE_GLYPH, truncate } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

interface Props {
  label: string
  config: HttpRequestConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

export function HttpRequestNode({ label, config, selected, executionStatus, onClick }: Props) {
  return (
    <NodeShell
      icon={<Globe size={18} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />}
      label={label}
      subtitle={config.url ? `${config.method} ${config.url}` : 'Set URL'}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {config.body?.trim() && <NodeFooter mono>{truncate(config.body.trim(), 50)}</NodeFooter>}
    </NodeShell>
  )
}
