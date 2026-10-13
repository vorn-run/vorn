import { Terminal, Code2 } from 'lucide-react'
import type { ScriptConfig, NodeExecutionStatus } from '../../../../shared/types'
import { NODE_GLYPH, truncate } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

interface Props {
  label: string
  config: ScriptConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

const SCRIPT_ICONS: Record<ScriptConfig['scriptType'], typeof Terminal> = {
  bash: Terminal,
  powershell: Terminal,
  python: Code2,
  node: Code2
}

export function ScriptNode({ label, config, selected, executionStatus, onClick }: Props) {
  const Icon = SCRIPT_ICONS[config.scriptType] || Terminal

  const preview = config.scriptContent
    ? config.scriptContent
        .split('\n')
        .find((l) => l.trim() && !l.startsWith('#'))
        ?.trim()
    : undefined

  return (
    <NodeShell
      icon={<Icon size={18} strokeWidth={2} className={`${NODE_GLYPH} shrink-0`} />}
      label={label}
      subtitle={
        <>
          {config.scriptType}
          {config.projectName && ` · ${config.projectName}`}
        </>
      }
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {preview && <NodeFooter mono>{truncate(preview, 50)}</NodeFooter>}
    </NodeShell>
  )
}
