import { TONE_DOT, TONE_DOT_MOVING, TONE_TEXT } from '../../lib/status-tone'
import type { RowState } from '../../lib/use-row-action'

// The same line install writes, for the actions that answer in one step.
export function ActivityLine({
  phrase,
  error,
  className = ''
}: RowState & { className?: string }): React.ReactElement | null {
  if (!phrase && !error) return null
  return (
    <span
      className={`flex items-start gap-1.5 ${phrase ? TONE_TEXT.live : TONE_TEXT.broken} ${className}`}
    >
      <span
        className={`w-1.5 h-1.5 rounded-full shrink-0 mt-1.5 ${phrase ? TONE_DOT_MOVING.live : TONE_DOT.broken}`}
      />
      {phrase ?? error}
    </span>
  )
}
