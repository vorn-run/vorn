import type { RestoredSession } from '../../shared/types'
import type { EndedSession } from '../stores/types'

// One reading of a record, so the board, the banner and the strip cannot disagree.
export function endedFromRestored(one: RestoredSession): EndedSession {
  return {
    reason: one.closedCleanly
      ? 'app-closed'
      : one.rebooted
        ? 'machine-restarted'
        : 'server-stopped',
    at: one.endedAt,
    replayed: one.replayable,
    partial: one.partial,
    ...(one.environment !== undefined && { environment: one.environment }),
    ...(one.session.shellCwd !== undefined && { cwd: one.session.shellCwd })
  }
}
