import { TerminalSession } from '@vornrun/shared/types'
import {
  saveSessions as dbSaveSessions,
  getPreviousSessions as dbGetPreviousSessions,
  clearSessions as dbClearSessions
} from './database'
import log from './logger'

/** Debounce window so rapid session events don't thrash the DB. */
const DEBOUNCE_MS = 500

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
    const sessions = this.getActiveSessions()
    this.saveSessions(sessions)
  }

  saveSessions(sessions: TerminalSession[]): void {
    try {
      dbSaveSessions(sessions)
      log.info(`[session-persistence] saved ${sessions.length} session(s)`)
    } catch (err) {
      log.warn({ err }, `[session-persistence] saveSessions failed (${sessions.length} sessions):`)
    }
  }

  getPreviousSessions(): TerminalSession[] {
    return this.readPreviousSessions() ?? []
  }

  /**
   * The same list, with "there are none" told apart from "could not read them".
   *
   * `getPreviousSessions` answers `[]` for both, which is right for its callers:
   * a client asking what was open gets an empty list either way and nothing is
   * lost. It is wrong for a caller that acts on absence. Terminal history is
   * keyed by session id and swept when no session claims it, so one transient
   * database error read as "no sessions" is every terminal's history removed.
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

  clear(): void {
    try {
      dbClearSessions()
    } catch (err) {
      log.warn({ err }, '[session-persistence] clear failed:')
    }
  }
}

export const sessionManager = new SessionManager()
