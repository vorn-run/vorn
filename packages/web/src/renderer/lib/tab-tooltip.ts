import type { AgentStatus } from '../../shared/types'
import { STATUS_LABEL } from './status-colors'

export function buildTooltip(
  displayName: string,
  status: AgentStatus,
  branch?: string,
  isWorktree?: boolean,
  taskTitle?: string,
  shellCwd?: string,
  shellExitCode?: number,
  projectName?: string
): string {
  const heading = projectName ? `${projectName} / ${displayName}` : displayName
  const lines = [`${heading} — ${STATUS_LABEL[status]}`]
  if (branch) {
    lines.push(`Branch: ${branch}${isWorktree ? ' (worktree)' : ''}`)
  }
  if (taskTitle) {
    lines.push(`Task: ${taskTitle}`)
  }
  if (shellCwd) {
    lines.push(`Cwd: ${shellCwd}`)
  }
  if (shellExitCode !== undefined) {
    lines.push(`Exit: ${shellExitCode}`)
  }
  return lines.join('\n')
}
