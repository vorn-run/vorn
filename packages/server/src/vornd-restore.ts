import type { HeadlessSession, TerminalSession } from '@vornrun/shared/types'
import log from './logger'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { consumeAllRestored } from './restored-sessions'
import { sessionManager } from './session-persistence'
import { vorndSessions, type HeldSession } from './vornd-sessions'

/**
 * The sessions vornd still holds from this server's previous run, taken on
 * again under the record vornd's copy has, which has them live again already
 * (`adopted`); one with no record is left where it is, and said so. `announce`
 * tells the windows.
 */
export function takeOnHeld(
  held: HeldSession[],
  announce: (session: TerminalSession) => void
): void {
  for (const one of held) {
    if (one.kind === 'piped') {
      const record = vorndSessions.mirror.headlessRecord(one.id)
      if (record) headlessManager.adoptHeld(ownRecord(record), one)
      continue
    }
    const session = vorndSessions.mirror.terminal(one.id)
    if (!session) {
      log.info({ id: one.id }, '[vornd] vornd holds a terminal this server has no record of')
      continue
    }
    const own = ownRecord(session)
    ptyManager.adoptVornd(own, one)
    announce(own)
  }
  sessionManager.scheduleSave()
}

/** A record from vornd's copy as this server's own: without the copy's revision and stamps. */
function ownRecord<T extends TerminalSession | HeadlessSession>(record: T): T {
  const own = { ...record } as T & { rev?: unknown; statusAt?: unknown; exitAt?: unknown }
  delete own.rev
  delete own.statusAt
  delete own.exitAt
  return own
}

/**
 * vornd owns the session records: what an older server saved in its database is
 * handed to it, and forgotten once vornd has answered, so it is handed only once.
 */
export async function carryToVornd(): Promise<void> {
  const records = consumeAllRestored().map((one) => one.session)
  if (records.length === 0) return
  const carried = await vorndSessions.carry(records)
  if (carried === null) return
  sessionManager.clear()
  // Some of them may run in the holder still: taken on now that vornd has their records.
  if (carried > 0) vorndSessions.restock()
  log.info(
    { records: records.length, carried },
    "[restored] handed the last run's sessions to vornd"
  )
}

/** Follows what vornd holds and owns, from each subscription on. */
export function wireVorndRestore(announce: (session: TerminalSession) => void): void {
  vorndSessions.on('held', (held: HeldSession[]) => takeOnHeld(held, announce))
  vorndSessions.on('restores', () => void carryToVornd())
}
