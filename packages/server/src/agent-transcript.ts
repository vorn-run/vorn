import type { AiAgentType, RecentSession, TerminalSession } from '@vornrun/shared/types'
import { supportsExactSessionResume } from '@vornrun/shared/types'
import { comparablePath, getRecentSessionsFor } from './agent-history'
import { claimSpawningTranscript, spawningTranscripts } from './transcript-claims'

function preferredPaths(session: TerminalSession): string[] {
  return [session.worktreePath, session.projectPath]
    .filter((value): value is string => value !== undefined)
    .map(comparablePath)
}

/** Which agent conversation a session continues, or undefined to let the agent choose. */
export function resolveTranscriptId(
  session: TerminalSession,
  held: ReadonlySet<string> = new Set()
): string | undefined {
  if (!supportsExactSessionResume(session.agentType)) return undefined

  // Not hookSessionId: that is Vorn's own routing id, which the agent has never seen.
  const pinned = session.agentSessionId
  if (pinned && !held.has(pinned)) return pinned

  const agentType = session.agentType as AiAgentType
  const available = (candidate: RecentSession): boolean =>
    candidate.agentType === agentType && candidate.canResumeExact && !held.has(candidate.sessionId)

  const wanted = preferredPaths(session)
  const atPreferredPath = (candidates: RecentSession[]): RecentSession | undefined => {
    for (const path of wanted) {
      const match = candidates.find(
        (candidate) => available(candidate) && comparablePath(candidate.projectPath) === path
      )
      if (match) return match
    }
    return undefined
  }

  const scoped = getRecentSessionsFor(agentType, session.projectPath)
  const scopedMatch = atPreferredPath(scoped) ?? scoped.find(available)
  if (scopedMatch) return scopedMatch.sessionId

  // Unscoped matches by path only: a loose match here would cross projects.
  return atPreferredPath(getRecentSessionsFor(agentType))?.sessionId
}

/** The conversations processes are writing right now. */
export function heldTranscripts(live: TerminalSession[]): Set<string> {
  const ids = live.map((session) => session.agentSessionId)
  return new Set(ids.filter((id): id is string => id !== undefined))
}

export function transcriptHolder(
  transcriptId: string,
  live: TerminalSession[]
): TerminalSession | undefined {
  return live.find((session) => session.agentSessionId === transcriptId)
}

/** The conversation a resume should continue, claimed against everything in flight. */
export function claimTranscriptFor(
  session: TerminalSession,
  live: TerminalSession[],
  sessionId: string
): string | undefined {
  const held = new Set([...heldTranscripts(live), ...spawningTranscripts()])
  const transcriptId = resolveTranscriptId(session, held)
  if (transcriptId) claimSpawningTranscript(transcriptId, sessionId)
  return transcriptId
}
