import { claimEffectReceipt, pruneEffectReceipts } from './database'
import log from './logger'

/**
 * Receiver-side deduplication of vornd's at-least-once effects (Session
 * Recovery Contract §7).
 *
 * vornd numbers every effect by the record that caused it, and a replay after a
 * vornd restart produces the same ids for the same output. Persisted output
 * proves nothing about whether this server already acted on one, so the
 * receiver keeps what it acted on:
 *
 * - `notify`: a desktop notification. Kept 24 hours, the window the contract
 *   names; a replay never reaches further back than the newest checkpoint.
 * - `trigger`: an exit that starts the next workflow step. Kept a week, so a
 *   step is never run twice for one exit however late the repeat arrives.
 *
 * Persisted in `effect_receipts`, so a repeat after this server restarted is
 * still a repeat. If the database cannot be written the claim falls back to
 * this process's memory, which still stops repeats within one run.
 */

export type ReceiptKind = 'notify' | 'trigger'

const KEEP_MS: Record<ReceiptKind, number> = {
  notify: 24 * 60 * 60 * 1000,
  trigger: 7 * 24 * 60 * 60 * 1000
}

/** How often old receipts are pruned, checked on each claim. */
const PRUNE_EVERY_MS = 60 * 60 * 1000

const inMemory = new Map<string, number>()
let lastPrune = 0

/** True the first time `effectId` is claimed for `kind`; false for every repeat. */
export function claimEffect(
  effectId: string,
  kind: ReceiptKind,
  now: number = Date.now()
): boolean {
  prune(now)
  const key = `${kind}\0${effectId}`
  if (inMemory.has(key)) return false
  let first: boolean
  try {
    first = claimEffectReceipt(effectId, kind, now)
  } catch (err) {
    log.warn({ err, effectId }, '[effects] could not record an effect; deduplicating in memory')
    first = true
  }
  if (first) inMemory.set(key, now)
  return first
}

function prune(now: number): void {
  if (now - lastPrune < PRUNE_EVERY_MS) return
  lastPrune = now
  for (const kind of Object.keys(KEEP_MS) as ReceiptKind[]) {
    const before = now - KEEP_MS[kind]
    try {
      pruneEffectReceipts(kind, before)
    } catch (err) {
      log.warn({ err }, '[effects] could not prune old receipts')
    }
    for (const [key, at] of inMemory) {
      if (key.startsWith(`${kind}\0`) && at < before) inMemory.delete(key)
    }
  }
}

/** Test seam: forget what this process remembers, not what the database holds. */
export function resetEffectMemory(): void {
  inMemory.clear()
  lastPrune = 0
}
