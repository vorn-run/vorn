import type {
  AgentType,
  AiAgentType,
  HeadlessSession,
  RecentSession,
  TerminalSession
} from '@vornrun/shared/types'
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

/**
 * The conversations processes are writing right now.
 *
 * A workflow step runs in its own manager rather than the pty one, and it can be
 * launched naming a conversation. Left out, its transcript reads as free and a
 * resume beside it starts a second agent on the run in progress.
 */
export function heldTranscripts(
  live: TerminalSession[],
  headless: HeadlessSession[] = []
): Set<string> {
  const running = headless.filter((session) => session.status === 'running')
  const ids = [...live, ...running].map((session) => session.agentSessionId)
  return new Set(ids.filter((id): id is string => id !== undefined))
}

/** Only a pane can be handed back, so a headless run holds without being a holder. */
export function transcriptHolder(
  transcriptId: string,
  live: TerminalSession[]
): TerminalSession | undefined {
  return live.find((session) => session.agentSessionId === transcriptId)
}

/**
 * The conversation a fresh launch is really naming.
 *
 * Nothing for an agent that cannot be sent back to one: `agent-launch` drops the
 * id there, so binding on it would answer with an unrelated session and claiming
 * it would hold a transcript nobody is going to open.
 */
export function transcriptNamedOnCreate(
  agentType: AgentType,
  resumeSessionId: string | undefined
): string | undefined {
  if (!resumeSessionId || !supportsExactSessionResume(agentType)) return undefined
  return resumeSessionId
}

/** The session a fresh launch should show instead, when it names one already running. */
export function sessionToBindOnCreate(
  resumeSessionId: string | undefined,
  live: TerminalSession[]
): TerminalSession | undefined {
  return resumeSessionId ? transcriptHolder(resumeSessionId, live) : undefined
}

/** The conversation a resume should continue, claimed against everything in flight. */
export function claimTranscriptFor(
  session: TerminalSession,
  live: TerminalSession[],
  sessionId: string,
  headless: HeadlessSession[] = []
): string | undefined {
  const held = new Set([...heldTranscripts(live, headless), ...spawningTranscripts()])
  const transcriptId = resolveTranscriptId(session, held)
  if (!transcriptId) return undefined
  // Taken between resolving and claiming: let the agent choose rather than double up.
  const taken = claimSpawningTranscript(transcriptId, sessionId)
  return taken === undefined ? transcriptId : undefined
}
