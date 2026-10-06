/**
 * Notifications the window has been shown, by the id of the effect that raised
 * them.
 *
 * vornd tells a notification again after it or the server restarts, and while
 * vornd runs native work a client can hear the same one from more than one
 * place. Each carries `effectId`, the same however often it is told, so this
 * shows it once. A payload without one is shown as it comes.
 */

/** As long as the server keeps its own receipt of a notification (NOTICE_RECEIPT_MS). */
export const NOTICE_RECEIPT_MS = 24 * 60 * 60 * 1000

/** Receipts kept: far more notifications than a day brings, and a bound on memory. */
export const NOTICE_RECEIPTS_KEPT = 2048

export class NoticeReceipts {
  /** When each id was first shown, least recently heard first. */
  private readonly seen = new Map<string, number>()

  constructor(
    private readonly kept: number = NOTICE_RECEIPTS_KEPT,
    private readonly keepMs: number = NOTICE_RECEIPT_MS,
    private readonly now: () => number = Date.now
  ) {}

  /** Whether a notification with this effect id is new; one heard within the window is not. */
  first(effectId: string): boolean {
    const now = this.now()
    const at = this.seen.get(effectId)
    this.seen.delete(effectId)
    if (at !== undefined && now - at < this.keepMs) {
      // Heard again: kept longest among those to drop, from when it was shown.
      this.seen.set(effectId, at)
      return false
    }
    this.seen.set(effectId, now)
    while (this.seen.size > this.kept) {
      const oldest = this.seen.keys().next().value
      if (oldest === undefined) break
      this.seen.delete(oldest)
    }
    return true
  }

  /** Whether to show a `terminal:notify` payload: unless its effect was shown already. */
  shows(payload: unknown): boolean {
    const effectId = (payload as { effectId?: unknown } | null)?.effectId
    return typeof effectId === 'string' && effectId !== '' ? this.first(effectId) : true
  }
}
