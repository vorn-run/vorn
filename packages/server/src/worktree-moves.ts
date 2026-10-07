/** The fields of a session that a worktree's branch rename or move changes. */
export interface WorktreeUpdates {
  branch?: string
  worktreePath?: string
  worktreeName?: string
}

/** Sets on `session` each field `updates` defines, as a rename or a move of its worktree does. */
export function applyWorktreeUpdates(session: WorktreeUpdates, updates: WorktreeUpdates): void {
  if (updates.branch !== undefined) session.branch = updates.branch
  if (updates.worktreeName !== undefined) session.worktreeName = updates.worktreeName
  if (updates.worktreePath !== undefined) session.worktreePath = updates.worktreePath
}
