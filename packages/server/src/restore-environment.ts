import type { RestoreEnvironment, TerminalSession } from '@vornrun/shared/types'

export interface EnvironmentProbe {
  isDirectory(at: string): boolean
  branch(cwd: string): string | null | Promise<string | null>
  head(cwd: string): string | null | Promise<string | null>
}

// What is there now against what the record says, so Resume is offered knowingly.
export async function probeEnvironment(
  session: Pick<
    TerminalSession,
    'projectPath' | 'worktreePath' | 'branch' | 'headCommit' | 'remoteHostId'
  >,
  probe: EnvironmentProbe
): Promise<RestoreEnvironment | undefined> {
  if (session.remoteHostId) return undefined
  const cwd = session.worktreePath ?? session.projectPath
  const present = probe.isDirectory(cwd)
  const [branch, head] = present
    ? await Promise.all([probe.branch(cwd), probe.head(cwd)])
    : [null, null]
  return {
    worktree: present ? 'ok' : 'missing',
    branch: { recorded: session.branch ?? null, actual: branch },
    head: { recorded: session.headCommit ?? null, actual: head }
  }
}
