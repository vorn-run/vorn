import crypto from 'node:crypto'
import log from './logger'

/**
 * Which app instance may run a given workflow trigger.
 *
 * Workflow steps execute in the renderer, but every instance connects to this
 * one core, and core broadcasts a scheduler tick to *all* of them. Two open
 * windows therefore each see the same tick and would each launch the same
 * agents. The renderer asks here first: exactly one claim is granted per
 * (workflow, trigger parameters) inside the dedupe window, and the losers stand
 * down.
 *
 * Identity deliberately includes the trigger parameters. A connector poll fans
 * out one run per new item — same workflow, different item — and those are
 * genuinely different work that should proceed in parallel. Only a true
 * double-fire, the same workflow with the same parameters at the same moment,
 * collapses into one run.
 *
 * Claims lapse on their own, so an instance that disappears mid-run never wedges
 * the workflow the way a lock held until release would.
 */

/** Long enough to swallow one tick seen by several instances, short enough that
 *  a deliberate re-run moments later still goes through. */
export const DEFAULT_DEDUPE_WINDOW_MS = 10_000

export interface RunClaimRequest {
  workflowId: string
  /** Trigger parameters fingerprint — connector item id, task id, or 'manual'. */
  params?: string
  windowMs?: number
}

export interface RunClaimResult {
  /** False when another instance already owns this exact trigger. */
  granted: boolean
  /** The winning run's id — this instance's when granted, the holder's when not. */
  runId: string
}

interface HeldClaim {
  runId: string
  claimedAt: number
  windowMs: number
}

const claims = new Map<string, HeldClaim>()

function claimKey(workflowId: string, params: string): string {
  // Encoded rather than concatenated: a fingerprint carries arbitrary upstream
  // text, and any separator character it happened to contain would let two
  // distinct pairs collide on one key.
  return JSON.stringify([workflowId, params])
}

/** Drop lapsed entries so a long-lived core doesn't accumulate dead claims. */
function evictLapsed(now: number): void {
  for (const [key, held] of claims) {
    if (now - held.claimedAt >= held.windowMs) claims.delete(key)
  }
}

export function claimWorkflowRun(req: RunClaimRequest): RunClaimResult {
  const params = req.params || 'manual'
  const windowMs = req.windowMs ?? DEFAULT_DEDUPE_WINDOW_MS
  const now = Date.now()
  const key = claimKey(req.workflowId, params)

  evictLapsed(now)

  const held = claims.get(key)
  if (held) {
    log.info(
      `[workflow-claims] duplicate trigger for ${req.workflowId} (params=${params}) — deferring to run ${held.runId}`
    )
    return { granted: false, runId: held.runId }
  }

  const runId = crypto.randomUUID()
  claims.set(key, { runId, claimedAt: now, windowMs })
  return { granted: true, runId }
}

/**
 * Give up a claim early, so re-running the same trigger doesn't have to wait out
 * the window. Only the holder may release it — a late release from a superseded
 * run must not clear the claim its successor now owns.
 */
export function releaseWorkflowRun(
  workflowId: string,
  params: string | undefined,
  runId: string
): void {
  const key = claimKey(workflowId, params || 'manual')
  if (claims.get(key)?.runId === runId) claims.delete(key)
}

/** Test seam — the registry is process-global otherwise. */
export function resetWorkflowRunClaims(): void {
  claims.clear()
}
