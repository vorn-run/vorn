/** Neither side of a split may collapse to nothing — a 0-extent pane is unrecoverable. */
export const MIN_SPLIT_RATIO = 0.15
export const MAX_SPLIT_RATIO = 0.85
export const DEFAULT_SPLIT_RATIO = 0.5

/**
 * Opening split for a card showing a device pane.
 *
 * A phone screen is about 0.46 as wide as it is tall, so it fills a pane by
 * height and leaves the surplus width as empty background — an even split buys
 * the device nothing and costs the terminal half a card. Two thirds to the
 * terminal keeps the phone at full height while giving the text the width it
 * actually uses.
 */
export const DEVICE_SPLIT_RATIO = 0.66

/**
 * Confine a split ratio to the visible range.
 *
 * Non-finite input falls back to an even split rather than propagating NaN into
 * a flex basis, which would render a card no drag could recover.
 */
export function clampSplitRatio(ratio: number): number {
  if (!Number.isFinite(ratio)) return DEFAULT_SPLIT_RATIO
  return Math.max(MIN_SPLIT_RATIO, Math.min(MAX_SPLIT_RATIO, ratio))
}

/**
 * Sanitize a stored pane-weight list.
 *
 * Weights are column-relative shares summing to 1, NOT two-sided split ratios:
 * with three panes open a legitimate weight sits well below MIN_SPLIT_RATIO, so
 * clamping them would bump a squeezed pane back up on release and again on
 * reload. Only non-finite or non-positive entries are rejected.
 */
export function sanitizePaneWeights(panes: unknown): number[] {
  if (!Array.isArray(panes)) return []
  return panes.map((n) => Number(n)).filter((n) => Number.isFinite(n) && n > 0)
}

/**
 * The share of a pane column each of its `count` panes should take.
 *
 * A stored list can be the wrong length — opening a third pane must not require
 * migrating what two panes had saved — so it is truncated or padded with even
 * shares and renormalized to sum to 1.
 */
export function splitPaneWeights(stored: number[] | undefined, count: number): number[] {
  if (count <= 0) return []
  const even = 1 / count
  const raw = Array.from({ length: count }, (_, i) => {
    const n = stored?.[i]
    return typeof n === 'number' && Number.isFinite(n) && n > 0 ? n : even
  })
  const total = raw.reduce((a, b) => a + b, 0)
  return total > 0 ? raw.map((n) => n / total) : raw.map(() => even)
}

/**
 * Move the divider below pane `index` to `ratio` — a fraction of the whole
 * column, since that is what a pointer position measures against.
 *
 * Only the two panes the divider touches change; the rest keep their share, so
 * dragging one divider never shuffles the whole stack. Neither of the pair may
 * collapse, hence the clamp against their combined span.
 */
export function resizePaneWeights(weights: number[], index: number, ratio: number): number[] {
  if (index < 0 || index + 1 >= weights.length) return weights
  const before = weights.slice(0, index).reduce((a, b) => a + b, 0)
  const pair = weights[index] + weights[index + 1]
  if (pair <= 0) return weights
  const target = Number.isFinite(ratio) ? ratio - before : pair / 2
  const first = Math.max(MIN_SPLIT_RATIO * pair, Math.min(MAX_SPLIT_RATIO * pair, target))
  const next = [...weights]
  next[index] = first
  next[index + 1] = pair - first
  return next
}
