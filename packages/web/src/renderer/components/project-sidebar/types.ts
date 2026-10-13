import type { AgentStatus, AgentType } from '../../../shared/types'

export interface SidebarSessionInfo {
  id: string
  name: string
  status: AgentStatus
  agentType: AgentType
  branch?: string
  isWorktree?: boolean
  worktreePath?: string
}
