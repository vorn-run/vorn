import { useAppStore } from '../stores'
import { withProgressToast } from './progress-toast'

interface RemoveWorktreeOptions {
  projectPath: string
  worktreePath: string
  sessionIds?: string[]
  force: boolean
}

export async function removeWorktreeWithProgress({
  projectPath,
  worktreePath,
  sessionIds = [],
  force
}: RemoveWorktreeOptions): Promise<void> {
  await withProgressToast(
    {
      loading:
        sessionIds.length > 0 ? 'Closing sessions & removing worktree…' : 'Removing worktree…',
      success: 'Worktree removed'
    },
    async () => {
      if (sessionIds.length > 0) {
        await Promise.all(
          sessionIds.flatMap((sid) => [
            window.api.killTerminal(sid).catch(() => {}),
            window.api.killHeadlessSession(sid).catch(() => {})
          ])
        )
        // Brief delay for processes to release file locks
        await new Promise((r) => setTimeout(r, 500))
      }
      const removed = await window.api.removeWorktree(projectPath, worktreePath, force)
      if (!removed) throw new Error('Failed to remove worktree')
      useAppStore.getState().loadWorktrees(projectPath, true)
    }
  )
}
