import { AgentStatus } from '../../shared/types'
import { STATUS_LABEL } from '../lib/status-colors'
import { RunningGlyph } from './RunningGlyph'
import { Tooltip } from './Tooltip'

interface Props {
  status: AgentStatus
  size?: number
}

// Minimal vocabulary: only the running state renders a visual indicator.
// Idle / waiting / error intentionally render nothing — the row background,
// the terminal content and hover tooltips carry those signals already.
export function StatusBadge({ status, size = 18 }: Props) {
  if (status !== 'running') return null

  return (
    <Tooltip label={STATUS_LABEL.running} position="top">
      <span
        className="inline-flex items-center justify-center"
        style={{ width: size, height: size }}
        aria-label={STATUS_LABEL.running}
        role="img"
        data-status="running"
      >
        <RunningGlyph size={size} className="text-green-400" />
      </span>
    </Tooltip>
  )
}
