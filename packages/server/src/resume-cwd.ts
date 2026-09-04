import type { TerminalSession } from '@vornrun/shared/types'

export interface ResumeCwd {
  cwd: string
  /** The more specific place the session wanted, when it is no longer there. */
  fellBackFrom?: string
}

// Most specific place first: the shell's own directory, the worktree, the project.
export function resumeCwdFor(
  previous: Pick<TerminalSession, 'projectPath' | 'worktreePath' | 'shellCwd'>,
  isDirectory: (at: string) => boolean
): ResumeCwd | null {
  const wanted = [previous.shellCwd, previous.worktreePath, previous.projectPath].filter(
    (at): at is string => at !== undefined
  )
  const cwd = wanted.find(isDirectory)
  if (cwd === undefined) return null
  return cwd === wanted[0] ? { cwd } : { cwd, fellBackFrom: wanted[0] }
}
