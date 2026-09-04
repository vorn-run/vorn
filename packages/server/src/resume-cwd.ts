import type { TerminalSession } from '@vornrun/shared/types'

/**
 * Where a session resumes into, or null when there is nowhere left.
 *
 * A resume is the one moment a remembered path turns into where a process
 * starts, so every candidate is checked before it is spawned into. The shell
 * branch of `sessions:resume` already asked this question; the agent branch
 * handed its worktree straight to the spawn, so a worktree removed while the
 * machine was off -- or simply cleaned up after its branch merged -- was spawned
 * into as though it were there. One function for both, so they cannot drift
 * apart again.
 *
 * Order is the most specific place first: the shell's own reported directory,
 * then the worktree, then the project. Falling back is deliberate -- a session
 * whose worktree is gone can still do useful work in the project -- but it is
 * reported, because the person deserves to know they are not where they were.
 *
 * `isDirectory` is a parameter so this can be decided without a filesystem.
 */
export interface ResumeCwd {
  cwd: string
  /** Set when the resume did not land where the session was. */
  fellBackFrom?: string
}

export function resumeCwdFor(
  previous: Pick<TerminalSession, 'projectPath' | 'worktreePath' | 'shellCwd'>,
  isDirectory: (at: string) => boolean
): ResumeCwd | null {
  const preferred = previous.shellCwd ?? previous.worktreePath
  if (preferred !== undefined && isDirectory(preferred)) return { cwd: preferred }
  if (isDirectory(previous.projectPath)) {
    return preferred === undefined
      ? { cwd: previous.projectPath }
      : { cwd: previous.projectPath, fellBackFrom: preferred }
  }
  return null
}
