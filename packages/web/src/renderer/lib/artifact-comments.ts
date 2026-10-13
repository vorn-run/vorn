import type { ArtifactComment, ArtifactMark } from '../../shared/types'

export const POPOVER_WIDTH = 260

/** Keep the popover inside the page area, just below the selection, or above it near the bottom. */
export function placePopover(
  rect: { x: number; y: number; width: number; height: number },
  area: { width: number; height: number }
): { x: number; y: number } {
  const x = Math.max(8, Math.min(rect.x, area.width - POPOVER_WIDTH - 8))
  const below = rect.y + rect.height + 8
  const y = below + 150 > area.height ? Math.max(8, rect.y - 158) : below
  return { x, y }
}

/** The most recently sent batch, which the rail keeps showing until the next one goes. */
export function latestSentBatch(comments: ArtifactComment[]): ArtifactComment[] {
  let latest: ArtifactComment | undefined
  for (const c of comments) {
    if (c.state === 'sent' && c.batchId && (!latest || (c.sentAt ?? '') > (latest.sentAt ?? '')))
      latest = c
  }
  return latest ? comments.filter((c) => c.batchId === latest!.batchId) : []
}

/** What to highlight: every draft, the last batch sent, and the comment in focus above both. */
export function marksFor(
  drafts: ArtifactComment[],
  sent: ArtifactComment[],
  focusId: string | null
): ArtifactMark[] {
  const marks: ArtifactMark[] = []
  for (const c of [...sent, ...drafts]) {
    if (c.anchor?.kind !== 'quote') continue
    const { quote, prefix, suffix } = c.anchor
    marks.push({
      id: c.id,
      quote,
      prefix,
      suffix,
      state: c.id === focusId ? 'focus' : c.state === 'draft' ? 'draft' : 'sent'
    })
  }
  return marks
}
