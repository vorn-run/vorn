import { X } from 'lucide-react'
import { Tooltip } from '../Tooltip'

interface Props {
  version: number
  /** How many sent comments this version was published in answer to. */
  answered: number
  /** The version those comments were written on, when there is one to compare against. */
  answeredOn?: number
  latest: number
  comparing: boolean
  onCompare: () => void
  onOpenLatest: () => void
  onDismiss: () => void
  btn: string
}

/** What a version is: newer than the one on screen, or the answer to comments you sent. */
export function ArtifactBanner({
  version,
  answered,
  answeredOn,
  latest,
  comparing,
  onCompare,
  onOpenLatest,
  onDismiss,
  btn
}: Props): React.JSX.Element | null {
  const behind = latest > version
  if (!behind && answered === 0) return null
  return (
    <div
      className="flex items-center gap-2 px-2.5 h-8 shrink-0 border-t border-white/[0.04]
                 bg-surface-sunken text-[12px] text-ink-secondary"
    >
      <span className="truncate min-w-0">
        {behind
          ? `v${latest} is out; you are reading v${version}.`
          : `v${version} answers your ${answered} comment${answered === 1 ? '' : 's'}${
              answeredOn ? ` on v${answeredOn}` : ''
            }.`}
      </span>
      <span className="flex-1" />
      {behind ? (
        <button
          type="button"
          onClick={onOpenLatest}
          className="h-6 px-2 rounded text-ink hover:bg-white/[0.06] shrink-0"
        >
          Open v{latest}
        </button>
      ) : (
        answeredOn && (
          <button
            type="button"
            onClick={onCompare}
            aria-pressed={comparing}
            className={`h-6 px-2 rounded text-ink hover:bg-white/[0.06] shrink-0 ${
              comparing ? 'bg-white/[0.08]' : ''
            }`}
          >
            {comparing ? 'Close compare' : `Compare with v${answeredOn}`}
          </button>
        )
      )}
      <Tooltip label="Dismiss" position="bottom">
        <button type="button" onClick={onDismiss} aria-label="Dismiss" className={btn}>
          <X size={12} strokeWidth={2} />
        </button>
      </Tooltip>
    </div>
  )
}
