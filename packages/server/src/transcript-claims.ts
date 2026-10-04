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

/**
 * Sessions whose workspace is still being prepared. Preparing waits on git (a
 * repository's turn, a worktree add of up to 30 s each try), which has no upper
 * bound the lapse window could cover, and is not the dead spawn the window is
 * for, so their claims do not lapse until it ends.
 */
const preparing = new Map<string, number>()

function evictLapsed(now: number): void {
  for (const [transcriptId, held] of spawning) {
    if (preparing.has(held.sessionId)) continue
    if (now - held.claimedAt >= SPAWN_WINDOW_MS) spawning.delete(transcriptId)
  }
}

/**
 * Keeps `sessionId`'s claims from lapsing until the returned function is
 * called, which restarts their window: from then on they wait on the agent's
 * report, as any claim does. Calling it twice is harmless.
 */
export function holdClaimsWhilePreparing(sessionId: string): () => void {
  preparing.set(sessionId, (preparing.get(sessionId) ?? 0) + 1)
  let done = false
  return () => {
    if (done) return
    done = true
    const left = (preparing.get(sessionId) ?? 1) - 1
    if (left > 0) {
      preparing.set(sessionId, left)
      return
    }
    preparing.delete(sessionId)
    const now = Date.now()
    for (const held of spawning.values()) {
      if (held.sessionId === sessionId) held.claimedAt = now
    }
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
  preparing.clear()
}
