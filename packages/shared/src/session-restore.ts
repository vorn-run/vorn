import type { CreateTerminalPayload, TerminalSession } from './types'

/**
 * Turning a saved session back into a request to start one.
 *
 * Shared rather than renderer-only because resuming is now something the server
 * does. A client asking for it would have to send a payload the server then
 * trusts, and the fields here -- a worktree path, a project directory -- are the
 * ones a session is launched into. The record is already on the server; it
 * should build the request from it.
 */
export function buildRestorePayload(
  s: TerminalSession,
  resumeSessionId?: string
): CreateTerminalPayload {
  if (s.agentType === 'shell') {
    throw new Error(
      'buildRestorePayload: shell sessions restore via createShellTerminal, not createTerminal'
    )
  }
  return {
    agentType: s.agentType,
    projectName: s.projectName,
    projectPath: s.projectPath,
    displayName: s.displayName,
    branch: s.isWorktree ? s.branch : undefined,
    existingWorktreePath: s.isWorktree ? s.worktreePath : undefined,
    worktreeName: s.worktreeName,
    useWorktree: (s.isWorktree && !s.worktreePath) || undefined,
    remoteHostId: s.remoteHostId,
    resumeSessionId
  }
}
