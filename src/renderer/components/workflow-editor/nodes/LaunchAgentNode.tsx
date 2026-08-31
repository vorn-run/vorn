import { AgentIcon } from '../../AgentIcon'
import type { LaunchAgentConfig, AiAgentType, NodeExecutionStatus } from '../../../../shared/types'
import { useAppStore } from '../../../stores'
import { ClipboardList, Server } from 'lucide-react'
import { NODE_GLYPH, truncate } from '../node-visuals'
import { NodeShell, NodeFooter } from './NodeShell'

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
    ? truncate(config.prompt, 60)
    : config.taskFromQueue
      ? 'Next task from queue'
      : config.taskId
        ? 'From task'
        : undefined

  return (
    <NodeShell
      icon={
        <span className="shrink-0">
          {isFromTask ? (
            <ClipboardList size={18} className={NODE_GLYPH} />
          ) : (
            <AgentIcon agentType={config.agentType as AiAgentType} size={18} />
          )}
        </span>
      }
      label={label}
      subtitle={
        <>
          {config.projectName || 'No project'}
          {!remoteHost && config.branch && ` · ${config.branch}`}
        </>
      }
      meta={
        remoteHost && (
          <div className="flex items-center gap-1 mt-0.5">
            <Server size={9} className="text-ink-faint" strokeWidth={1.5} />
            <span className="text-[10px] text-ink-faint truncate">{remoteHost.label}</span>
          </div>
        )
      }
      selected={selected}
      executionStatus={executionStatus}
      onClick={onClick}
    >
      {promptPreview && <NodeFooter>{promptPreview}</NodeFooter>}
    </NodeShell>
  )
}
