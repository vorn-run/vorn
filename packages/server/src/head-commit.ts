import type { TerminalSession } from '@vornrun/shared/types'

/** Ten sessions on a board would otherwise be ten subprocesses on every save. */
export const HEAD_REFRESH_MS = 30_000

// Keeps each session's recorded HEAD following the tree, at a bounded cost.
export class HeadRefresh {
  private checkedAt = new Map<string, number>()

  constructor(
    private readonly read: (cwd: string) => string | null,
    private readonly every: number = HEAD_REFRESH_MS
  ) {}

  refresh(sessions: TerminalSession[], now: number = Date.now()): void {
    for (const s of sessions) {
      if (s.remoteHostId) continue
      const last = this.checkedAt.get(s.id)
      if (last !== undefined && now - last < this.every) continue
      this.checkedAt.set(s.id, now)
      const head = this.read(s.worktreePath ?? s.projectPath)
      if (head) s.headCommit = head
    }
  }

  /** The next refresh reads this one again, whatever the clock says. */
  invalidate(id: string): void {
    this.checkedAt.delete(id)
  }

  forget(id: string): void {
    this.checkedAt.delete(id)
  }
}
