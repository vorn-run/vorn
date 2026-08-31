/**
 * A transcript is spoken for while its process starts, before it reports its own id.
 *
 * codex and opencode cannot be told an id at launch, so `agentSessionId` only
 * arrives from the capture five seconds later. Until then a live session does not
 * know which conversation it is writing, and two resumes would both resolve to it.
 * Claims lapse rather than lock, so a spawn that dies never wedges the transcript.
 */

const SPAWN_WINDOW_MS = 15_000

interface Spawning {
  sessionId: string
  claimedAt: number
}

const spawning = new Map<string, Spawning>()

function evictLapsed(now: number): void {
  for (const [transcriptId, held] of spawning) {
    if (now - held.claimedAt >= SPAWN_WINDOW_MS) spawning.delete(transcriptId)
  }
}

/** The session already starting on this transcript, or undefined when the claim is taken. */
export function claimSpawningTranscript(
  transcriptId: string,
  sessionId: string
): string | undefined {
  const now = Date.now()
  evictLapsed(now)
  const held = spawning.get(transcriptId)
  if (held) return held.sessionId
  spawning.set(transcriptId, { sessionId, claimedAt: now })
  return undefined
}

export function releaseSpawningTranscript(transcriptId: string, sessionId: string): void {
  if (spawning.get(transcriptId)?.sessionId === sessionId) spawning.delete(transcriptId)
}

export function spawningTranscripts(): Set<string> {
  evictLapsed(Date.now())
  return new Set(spawning.keys())
}

export function resetTranscriptClaims(): void {
  spawning.clear()
}
