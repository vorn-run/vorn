import type { AgentStatus, AgentType } from '../../shared/types'
import { AgentIcon } from './AgentIcon'
import { RunningGlyph } from './RunningGlyph'

interface Props {
  agentType: AgentType
  status: AgentStatus
  size?: number
}

export function AgentStatusIcon({ agentType, status, size = 14 }: Props) {
  if (agentType === 'shell' || status !== 'running') {
    return <AgentIcon agentType={agentType} size={size} />
  }
  return <RunningGlyph size={size} className="text-white/85" aria-label="Running" />
}
