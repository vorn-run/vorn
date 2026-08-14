import { Globe } from 'lucide-react'
import { FileTypeIcon } from './file-icons'
import type { PromotedCard } from '../hooks/usePromotedCards'

/**
 * What a card looks like, in one place.
 *
 * A card shows up in five lists — its tab, its dock pill, its sidebar row, the
 * focus stage and the mobile list — and each had built this itself, so a new
 * card kind meant finding all five and the stroke weights had already drifted
 * between them without anyone noticing.
 */
export function CardSubjectIcon({
  card,
  size = 14
}: {
  card: Pick<PromotedCard, 'kind' | 'name'>
  size?: number
}) {
  return (
    <span
      className="shrink-0 flex items-center justify-center"
      style={{ width: size, height: size }}
    >
      {card.kind === 'browser' ? (
        <Globe size={size} strokeWidth={1.5} className="text-ink-faint" />
      ) : (
        <FileTypeIcon name={card.name} size={size} />
      )}
    </span>
  )
}
