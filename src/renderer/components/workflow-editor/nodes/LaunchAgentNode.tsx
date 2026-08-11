import { AgentIcon } from '../../AgentIcon'
import type { LaunchAgentConfig, AiAgentType, NodeExecutionStatus } from '../../../../shared/types'
import { useAppStore } from '../../../stores'
import { ClipboardList, Server } from 'lucide-react'
import { STATUS_DOT_CLASSES } from '../statusDot'

interface Props {
  label: string
  config: LaunchAgentConfig
  selected?: boolean
  executionStatus?: NodeExecutionStatus
  onClick: () => void
}

export function LaunchAgentNode({ label, config, selected, executionStatus, onClick }: Props) {
  const isFromTask = config.agentType === 'fromTask'
  const remoteHosts = useAppStore((s) => s.config?.remoteHosts)
  const remoteHost = config.remoteHostId
    ? remoteHosts?.find((h) => h.id === config.remoteHostId)
    : undefined

  const promptPreview = config.prompt
    ? config.prompt.length > 60
      ? config.prompt.slice(0, 60) + '...'
      : config.prompt
    : config.taskFromQueue
      ? 'Next task from queue'
      : config.taskId
        ? 'From task'
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
        <span className="shrink-0">
          {isFromTask ? (
            <ClipboardList size={14} className="text-blue-400" />
          ) : (
            <AgentIcon agentType={config.agentType as AiAgentType} size={14} />
          )}
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium text-white truncate">{label}</div>
          <div className="text-[11px] text-gray-500 truncate">
            {config.projectName || 'No project'}
            {!remoteHost && config.branch && ` · ${config.branch}`}
          </div>
          {remoteHost && (
            <div className="flex items-center gap-1 mt-0.5">
              <Server size={9} className="text-blue-400" strokeWidth={1.5} />
              <span className="text-[10px] text-blue-400 truncate">{remoteHost.label}</span>
            </div>
          )}
        </div>
      </div>
      {promptPreview && (
        <div className="mt-2 text-[11px] text-gray-600 truncate border-t border-white/[0.06] pt-2">
          {promptPreview}
        </div>
      )}
    </div>
  )
}
