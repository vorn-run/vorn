import { TerminalSession } from '@vornrun/shared/types'
import {
  getPreviousSessions as dbGetPreviousSessions,
  clearSessions as dbClearSessions
} from './database'
import log from './logger'

/** Debounce window so rapid session events don't thrash the DB. */
const DEBOUNCE_MS = 500

/**
 * What this server keeps of the session records, which is no longer the records
 * themselves: vornd owns and writes them. Left here are the debounced walk over
 * the live sessions that keeps each one's HEAD current, and what an older server
 * saved, read once to hand to vornd and then cleared.
 */
class SessionManager {
  private getActiveSessions: (() => TerminalSession[]) | null = null
  private debounceTimer: ReturnType<typeof setTimeout> | null = null

  /** Wire up a session source so the manager knows what to persist. */
  startAutoSave(getActiveSessions: () => TerminalSession[]): void {
    this.stopAutoSave()
    this.getActiveSessions = getActiveSessions
  }

  stopAutoSave(): void {
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer)
      this.debounceTimer = null
    }
    this.getActiveSessions = null
  }

  scheduleSave(): void {
    if (!this.getActiveSessions) return
    if (this.debounceTimer) clearTimeout(this.debounceTimer)
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null
      this.persistNow()
    }, DEBOUNCE_MS)
  }

  persistNow(): void {
    if (!this.getActiveSessions) return
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer)
      this.debounceTimer = null
    }
    this.getActiveSessions()
  }

  /**
   * What the last run left, with "there are none" told apart from "could not
   * read them".
   *
   * That distinction is the whole point of the null. Terminal history is keyed
   * by session id and swept when no session claims it, so one transient database
   * error read as "there are no sessions" is every terminal's history removed.
   *
   * There was a second reader that flattened null to `[]`, for an RPC no client
   * called after the app stopped asking the database what was open and started
   * asking the server what exists. Both are gone.
   */
  readPreviousSessions(): TerminalSession[] | null {
    try {
      const sessions = dbGetPreviousSessions()
      log.info(`[session-persistence] loaded ${sessions.length} previous session(s)`)
      return sessions
    } catch (err) {
      log.warn({ err }, '[session-persistence] getPreviousSessions failed:')
      return null
    }
  }

  /** Forgets what an older server saved, once vornd has been handed it. */
  clear(): void {
    try {
      dbClearSessions()
    } catch (err) {
      log.warn({ err }, '[session-persistence] clear failed:')
    }
  }
}

export const sessionManager = new SessionManager()
