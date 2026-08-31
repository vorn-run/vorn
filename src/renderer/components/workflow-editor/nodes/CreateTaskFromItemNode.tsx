import { ListPlus } from 'lucide-react'
import type { CreateTaskFromItemConfig, NodeExecutionStatus } from '../../../../shared/types'
import { NODE_GLYPH } from '../node-visuals'
import { NodeShell } from './NodeShell'

interface Props {
  label: string
  config: CreateTaskFromItemConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

export function CreateTaskFromItemNode({
  label,
  config,
  selected,
  executionStatus,
  onClick
}: Props) {
  const projectLabel =
    config.project === 'fromConnection' ? 'Project from connection' : config.project

  return (
    <NodeShell
      icon={<ListPlus size={18} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />}
      label={label}
      subtitle={`${projectLabel} · initial: ${config.initialStatus}`}
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    />
  )
}
