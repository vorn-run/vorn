import type { RestoredSession, TerminalSession } from '../../shared/types'
import { useAppStore } from '../stores'
import { coldSessions } from './session-utils'

/**
 * Make the board agree with what the server actually has.
 *
 * Two questions -- what is running, and what is left of what is not -- asked at
 * the two moments the answer can change out from under a pane: when the app
 * starts, and when the server behind it is replaced.
 *
 * That second moment is the one this exists for. A pane's content lives in the
 * renderer, so when a server dies nothing about the pane changes: the terminal
 * keeps showing what it was showing, the bridge quietly reconnects to a
 * replacement holding none of the old PTYs, and the pane goes on accepting input
 * for a process that is gone. It looks exactly like a terminal that has been
 * quiet for a moment, which is the one thing a pane must never look like when it
 * is a photograph.
 */
export async function syncBoard(options: { showCold: boolean }): Promise<void> {
  const answers = await Promise.all([
    window.api.listActiveSessions(),
    window.api.getRestoredSessions()
  ]).catch((err) => {
    console.error('[board] failed to read what the server has:', err)
    return null
  })
  // A board left as it is beats a board rebuilt from an answer nobody gave.
  if (!answers) return
  const [active, carried]: [TerminalSession[], RestoredSession[]] = answers

  const store = useAppStore.getState()
  const live = new Set(active.map((s) => s.id))
  const heldById = new Map(carried.map((one) => [one.session.id, one]))

  // Anything the server has that this board does not. A session started from a
  // phone, or by a workflow, or before this window existed.
  for (const session of active) {
    if (!store.terminals.has(session.id)) store.addTerminal(session)
  }

  // Panes this board is holding for sessions the server no longer runs. On a
  // first start there are none; after a replacement this is all of them.
  for (const [id, term] of useAppStore.getState().terminals) {
    if (live.has(id) || term.ended) continue
    const held = heldById.get(id)
    useAppStore.getState().markEnded(id, {
      reason: held?.closedCleanly ? 'app-closed' : 'server-stopped',
      // A held record knows when its run ended. Without one the best available
      // answer is when this pane last saw anything.
      at: held?.endedAt ?? term.lastOutputTimestamp,
      // Whether there is a rebuilt screen behind it. False means the pane is
      // showing what it happens to still have in its own buffer and nothing
      // more, which the strip says out loud.
      replayed: held?.replayable ?? false,
      ...(held?.partial !== undefined && { partial: held.partial }),
      ...(term.session.shellCwd !== undefined && { cwd: term.session.shellCwd })
    })
  }

  if (!options.showCold) return

  // And the ones with no pane at all yet: ended before this window opened.
  for (const one of coldSessions(active, carried)) {
    if (useAppStore.getState().terminals.has(one.session.id)) continue
    useAppStore.getState().addTerminal(one.session, {
      reason: one.closedCleanly ? 'app-closed' : 'server-stopped',
      at: one.endedAt,
      replayed: one.replayable,
      partial: one.partial,
      ...(one.session.shellCwd !== undefined && { cwd: one.session.shellCwd })
    })
  }
}
