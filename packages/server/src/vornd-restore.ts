import type { HeadlessSession, TerminalSession } from '@vornrun/shared/types'
import log from './logger'
import { ptyManager } from './pty-manager'
import { headlessManager } from './headless-manager'
import { consumeAllRestored, consumeRestored } from './restored-sessions'
import { sessionManager } from './session-persistence'
import { vorndSessions, type HeldSession } from './vornd-sessions'

/**
 * The sessions vornd still holds from this server's previous run, taken on
 * again.
 *
 * Each terminal is taken on under the record that run saved rather than offered
 * to resume; one with no record is left where it is, and said so. While vornd
 * owns the records between runs (`vorndSessions.restoresSessions`) the record is
 * its copy's, which has it live again already (`adopted`), and a headless agent
 * it holds is followed again too; otherwise it is the one this server's database
 * kept (`restored-sessions`). Either way `announce` tells the windows.
 */
export function takeOnHeld(
  held: HeldSession[],
  announce: (session: TerminalSession) => void
): void {
  const fromVornd = vorndSessions.restoresSessions()
  for (const one of held) {
    if (one.kind === 'piped') {
      const record = fromVornd ? vorndSessions.mirror.headlessRecord(one.id) : undefined
      if (record) headlessManager.adoptHeld(ownRecord(record), one)
      continue
    }
    const session = fromVornd
      ? vorndSessions.mirror.terminal(one.id)
      : consumeRestored(one.id)?.session
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
 * vornd owns the session records now: what this server's database kept of its
 * last run is handed to it, once, for a vornd with nothing of its own to offer.
 * Either way nothing is offered from here any more.
 */
export async function carryToVornd(): Promise<void> {
  const records = consumeAllRestored().map((one) => one.session)
  if (records.length === 0) return
  const carried = await vorndSessions.carry(records)
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
