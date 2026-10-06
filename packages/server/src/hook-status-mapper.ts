import { AgentStatus, HookEvent } from '@vornrun/shared/types'
import { ptyManager } from './pty-manager'
import log from './logger'

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
   * Tries to link a Claude session_id to a Vorn terminal by cwd.
   * Only matches unlinked terminals to avoid stealing an already-claimed one.
   * Called on SessionStart (preferred) and as a fallback on any event.
   */
  tryLink(sessionId: string, cwd: string): string | undefined {
    if (this.sessionMap.has(sessionId)) return this.sessionMap.get(sessionId)

    const linkedTerminalIds = new Set(this.sessionMap.values())
    const session = ptyManager.findUnlinkedSessionByCwd(cwd, linkedTerminalIds)
    if (session) {
      log.info(`[hooks] linked session ${sessionId} -> terminal ${session.id} (cwd: ${cwd})`)
      this.sessionMap.set(sessionId, session.id)
      session.hookSessionId = sessionId
      ptyManager.recordChanged(session.id)
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
    // On SessionStart, always try to link. On other events, check the cache
    // first but fall back to cwd linking in case SessionStart was missed
    // (e.g. Claude was already running when Vorn started).
    const terminalId =
      event.hook_event_name === 'SessionStart'
        ? this.tryLink(event.session_id, event.cwd)
        : (this.sessionMap.get(event.session_id) ?? this.tryLink(event.session_id, event.cwd))

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
