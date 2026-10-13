import { TerminalSession } from '../../shared/types'

export function getDisplayName(session: TerminalSession): string {
  return session.displayName?.trim() || session.projectName
}

export function getBranchLabel(session: {
  branch?: string
  isWorktree?: boolean
  worktreeName?: string
}): string | undefined {
  if (session.isWorktree && session.worktreeName) return session.worktreeName
  return session.branch
}
