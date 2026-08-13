import { Terminal, Code2 } from 'lucide-react'
import type { ScriptConfig, NodeExecutionStatus } from '../../../../shared/types'
import { WORKFLOW_STATUS_DOT_PULSE } from '../../../lib/workflow-status'
import { NODE_SELECTED, NODE_UNSELECTED, NODE_GLYPH } from '../node-visuals'

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
        <Icon size={14} strokeWidth={2} className={`${NODE_GLYPH} shrink-0`} />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">
            {config.scriptType}
            {config.projectName && ` · ${config.projectName}`}
          </div>
        </div>
      </div>
      {preview && (
        <div className="mt-2 text-[11px] text-gray-600 truncate border-t border-white/[0.06] pt-2 font-mono">
          {preview.length > 50 ? preview.slice(0, 50) + '...' : preview}
        </div>
      )}
    </div>
  )
}
