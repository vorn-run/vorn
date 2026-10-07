import { AgentStatus, HookEvent, TerminalSession } from '@vornrun/shared/types'
import { ptyManager } from './pty-manager'
import log from './logger'

/** How a hook's conversation was matched to a terminal, for the log. */
type LinkedBy = 'terminal' | 'conversation'

class HookStatusMapper {
  // Confirmed session_id -> terminalId links, established exclusively on SessionStart
  private sessionMap = new Map<string, string>()

  /**
   * Returns the Vorn terminalId for a confirmed Claude session_id.
   * Returns undefined if the session has never fired a SessionStart that
   * matched a Vorn terminal.
   */
  getLinkedTerminal(sessionId: string): string | undefined {
    return this.sessionMap.get(sessionId)
  }

  /**
   * The terminal an event comes from: by an exact identity first (the terminal
   * id its launch gave it, then the conversation id it was started with), then
   * by a link already made, and by its folder only when nothing exact exists.
   *
   * The folder alone cannot tell two agents in one directory apart, so an exact
   * identity also overrules a link the folder guessed earlier.
   */
  resolveTerminal(event: HookEvent): string | undefined {
    const exact = this.findExact(event)
    if (exact) {
      this.linkExact(event.session_id, exact.session, exact.by)
      return exact.session.id
    }
    return this.sessionMap.get(event.session_id) ?? this.tryLink(event.session_id, event.cwd)
  }

  /** A live terminal the event names exactly, or undefined when it names none. */
  private findExact(event: HookEvent): { session: TerminalSession; by: LinkedBy } | undefined {
    const sessions = ptyManager.getActiveSessions()
    const terminalId = event.vorn_terminal_id
    if (terminalId) {
      const session = sessions.find((s) => s.id === terminalId)
      if (session) return { session, by: 'terminal' }
    }
    let newest: TerminalSession | undefined
    for (const s of sessions) {
      if (s.agentSessionId !== event.session_id) continue
      if (!newest || s.createdAt > newest.createdAt) newest = s
    }
    return newest && { session: newest, by: 'conversation' }
  }

  private linkExact(sessionId: string, session: TerminalSession, by: LinkedBy): void {
    if (this.sessionMap.get(sessionId) === session.id) return
    // Whatever else claimed this terminal was a guess, or a conversation it has since left.
    for (const [other, terminalId] of this.sessionMap) {
      if (terminalId === session.id) this.sessionMap.delete(other)
    }
    this.sessionMap.set(sessionId, session.id)
    log.info(`[hooks] linked session ${sessionId} -> terminal ${session.id} (by ${by})`)
    if (session.hookSessionId !== sessionId) ptyManager.linkHookSession(session.id, sessionId)
  }

  /**
   * Tries to link a Claude session_id to a Vorn terminal by cwd.
   * Only matches unlinked terminals to avoid stealing an already-claimed one.
   * The fallback for an event that names no terminal exactly; when several
   * terminals share the folder the most recently launched one is taken.
   */
  tryLink(sessionId: string, cwd: string): string | undefined {
    if (this.sessionMap.has(sessionId)) return this.sessionMap.get(sessionId)

    const linkedTerminalIds = new Set(this.sessionMap.values())
    const [session, ...others] = ptyManager.findUnlinkedSessionsByCwd(cwd, linkedTerminalIds)
    if (session) {
      if (others.length > 0) {
        log.warn(
          `[hooks] session ${sessionId} names no terminal and ${others.length + 1} share ` +
            `cwd ${cwd}; guessed the newest, ${session.id}, over ${others.map((s) => s.id).join(', ')}`
        )
      }
      log.info(`[hooks] linked session ${sessionId} -> terminal ${session.id} (cwd: ${cwd})`)
      this.sessionMap.set(sessionId, session.id)
      ptyManager.linkHookSession(session.id, sessionId)
      // Don't set statusSource here — promoteToHookStatus() handles it and
      // also re-arms the idle timer. Setting it directly would bypass that.
      return session.id
    }

    log.info(
      `[hooks] no unlinked terminal for session ${sessionId} cwd=${cwd} (active terminals: ${
        ptyManager
          .getActiveSessions()
          .map((s) => s.projectPath)
          .join(', ') || 'none'
      })`
    )
    return undefined
  }

  mapEventToStatus(event: HookEvent): { terminalId: string; status: AgentStatus } | null {
    // Resolved on every event, not only SessionStart: one can be missed (e.g.
    // Claude was already running when Vorn started).
    const terminalId = this.resolveTerminal(event)

    if (!terminalId) return null

    let status: AgentStatus

    switch (event.hook_event_name) {
      case 'SessionStart':
      case 'PreToolUse':
      case 'PostToolUse':
        status = 'running'
        break
      case 'PostToolUseFailure':
        status = 'error'
        break
      case 'Notification':
      case 'PermissionRequest':
        status = 'waiting'
        break
      case 'Stop':
        status = 'idle'
        break
      case 'SessionEnd':
        status = 'idle'
        this.sessionMap.delete(event.session_id)
        break
      default:
        return null
    }

    return { terminalId, status }
  }

  /** Pre-link a known session_id -> terminalId (used by Copilot where we generate the session ID ourselves) */
  forceLink(sessionId: string, terminalId: string): void {
    this.sessionMap.set(sessionId, terminalId)
  }

  removeSession(sessionId: string): void {
    this.sessionMap.delete(sessionId)
  }

  clear(): void {
    this.sessionMap.clear()
  }
}

export const hookStatusMapper = new HookStatusMapper()
