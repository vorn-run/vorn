// Held while a process starts, before it can report which conversation it took.

// Longer than the capture ladder that sets `agentSessionId`, so a claim outlives
// the reporting it waits for; still lapsing, so a spawn that dies never wedges a
// transcript.
const SPAWN_WINDOW_MS = 60_000

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

/** Everything a session was holding, for when it reports its conversation or exits. */
export function releaseSpawningTranscriptsFor(sessionId: string): void {
  for (const [transcriptId, held] of spawning) {
    if (held.sessionId === sessionId) spawning.delete(transcriptId)
  }
}

export function spawningTranscripts(): Set<string> {
  evictLapsed(Date.now())
  return new Set(spawning.keys())
}

export function resetTranscriptClaims(): void {
  spawning.clear()
}
