import type { RestoreEnvironment, TerminalSession } from '@vornrun/shared/types'

export interface EnvironmentProbe {
  isDirectory(at: string): boolean
  branch(cwd: string): string | null
  head(cwd: string): string | null
}

// What is there now against what the record says, so Resume is offered knowingly.
export function probeEnvironment(
  session: Pick<
    TerminalSession,
    'projectPath' | 'worktreePath' | 'branch' | 'headCommit' | 'remoteHostId'
  >,
  probe: EnvironmentProbe
): RestoreEnvironment | undefined {
  if (session.remoteHostId) return undefined
  const cwd = session.worktreePath ?? session.projectPath
  const present = probe.isDirectory(cwd)
  return {
    worktree: present ? 'ok' : 'missing',
    branch: { recorded: session.branch ?? null, actual: present ? probe.branch(cwd) : null },
    head: { recorded: session.headCommit ?? null, actual: present ? probe.head(cwd) : null }
  }
}
