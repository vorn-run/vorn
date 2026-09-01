import { Globe } from 'lucide-react'
import type { HttpRequestConfig, NodeExecutionStatus } from '../../../../shared/types'
import { NODE_GLYPH } from '../node-visuals'
import { NodeShell } from './NodeShell'

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
    />
  )
}
