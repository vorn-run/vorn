import { useEffect, useRef } from 'react'
import { useAppStore } from '../stores'
import type { GitDiffStat } from '../../shared/types'

const POLL_INTERVAL = 30_000

export function useGitDiffPolling(): void {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const pollingRef = useRef(false)

  useEffect(() => {
    const poll = async (): Promise<void> => {
      if (document.hidden) return
      if (pollingRef.current) return // skip if previous poll still running
      pollingRef.current = true

      try {
        const { terminals, updateGitDiffStats, updateSessionBranch } = useAppStore.getState()

        const entries = Array.from(terminals)

        const batchStats = new Map<string, GitDiffStat>()
        await Promise.allSettled(
          entries.map(async ([id, t]) => {
            const cwd = t.session.worktreePath || t.session.projectPath
            try {
              const [stat, branch] = await Promise.all([
                window.api.getGitDiffStat(cwd),
                window.api.getGitBranch(cwd)
              ])
              if (stat) batchStats.set(id, stat)
              if (branch && branch !== t.session.branch) {
                updateSessionBranch(id, branch)
              }
            } catch {
              // not a git repo or error — skip
            }
          })
        )

        if (batchStats.size > 0) {
          updateGitDiffStats(batchStats)
        }
      } finally {
        pollingRef.current = false
      }
    }

    poll()
    intervalRef.current = setInterval(poll, POLL_INTERVAL)

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current)
    }
  }, [])
}
