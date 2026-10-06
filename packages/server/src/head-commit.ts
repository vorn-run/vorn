import type { TerminalSession } from '@vornrun/shared/types'

/** Ten sessions on a board would otherwise be ten subprocesses on every save. */
export const HEAD_REFRESH_MS = 30_000

// Keeps each session's recorded HEAD following the tree, at a bounded cost.
export class HeadRefresh {
  private checkedAt = new Map<string, number>()
  /** The latest read dispatched per session; an older one that lands late is dropped. */
  private latest = new Map<string, number>()
  private reads = 0

  constructor(
    private readonly read: (cwd: string) => Promise<string | null>,
    private readonly every: number = HEAD_REFRESH_MS,
    /** Told after a read moved a session's recorded HEAD, which it does in place. */
    private readonly moved: (session: TerminalSession) => void = () => {}
  ) {}

  refresh(sessions: TerminalSession[], now: number = Date.now()): void {
    for (const s of sessions) {
      if (s.remoteHostId) continue
      const last = this.checkedAt.get(s.id)
      if (last !== undefined && now - last < this.every) continue
      this.checkedAt.set(s.id, now)
      // Not awaited: the save that asked carries on with what the session has,
      // and the next one writes the new HEAD. A failed read leaves the old one,
      // and so does one overtaken by a read dispatched after it.
      const read = ++this.reads
      this.latest.set(s.id, read)
      this.read(s.worktreePath ?? s.projectPath).then(
        (head) => {
          if (head && this.latest.get(s.id) === read && s.headCommit !== head) {
            s.headCommit = head
            this.moved(s)
          }
        },
        () => {}
      )
    }
  }

  /** The next refresh reads this one again, whatever the clock says. */
  invalidate(id: string): void {
    this.checkedAt.delete(id)
  }

  forget(id: string): void {
    this.checkedAt.delete(id)
    this.latest.delete(id)
  }
}
