import { Fragment, memo } from 'react'
import { formatRelativeTime, formatRunDuration } from '../../lib/format-time'
import {
  describeRun,
  runStatusLine,
  stepProgress,
  type RunWorkflowRef
} from '../../lib/run-presentation'
import { WORKFLOW_STATUS_DOT_PULSE, WORKFLOW_STATUS_DOT } from '../../lib/workflow-status'
import { useConnectorLook } from '../../lib/use-connections'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'

interface Props {
  run: RunListEntry
  workflow?: RunWorkflowRef
  workflowDeleted: boolean
  selected: boolean
  onSelect: () => void
  onOpenWorkflow: () => void
}

function RunListRowImpl({
  run,
  workflow,
  workflowDeleted,
  selected,
  onSelect,
  onOpenWorkflow
}: Props) {
  const nodes = workflow?.nodes ?? []
  const look = useConnectorLook(run.connectorItem?.connectionId)
  const presentation = describeRun(run, workflow, look)
  const { done, total } = stepProgress(run, nodes)
  const dotStatus = run.nodeStates.some((n) => n.status === 'waiting') ? 'waiting' : run.status
  // The dot is the row's only colour; everything else is one quiet line of words.
  const details = [
    runStatusLine(run, nodes),
    presentation.subtitle ?? presentation.sourceLabel,
    run.status !== 'success' && done > 0 ? `${done} of ${total} steps` : undefined,
    run.partial ? 'partial' : undefined,
    workflowDeleted ? 'deleted' : undefined,
    formatRelativeTime(run.startedAt)
  ].filter((part): part is string => !!part)

  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      onDoubleClick={workflowDeleted ? undefined : onOpenWorkflow}
      className={`relative w-full text-left px-4 py-2.5 border-b border-white/[0.04] grid grid-cols-[6px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 transition-colors ${
        selected ? 'bg-white/[0.04]' : 'hover:bg-white/[0.02]'
      }`}
    >
      {selected && <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />}
      <span
        role="img"
        aria-label={dotStatus}
        className={`w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT_PULSE[dotStatus] ?? WORKFLOW_STATUS_DOT.pending}`}
      />
      <span className="text-[13px] text-ink truncate">{presentation.title}</span>
      <span className="font-mono text-[12px] text-ink-secondary tabular-nums">
        {formatRunDuration(run.startedAt, run.completedAt)}
      </span>
      <span className="col-start-2 col-span-2 text-[12px] text-ink-faint truncate">
        {details.map((part, i) => (
          <Fragment key={i}>
            {i > 0 && ' · '}
            <span>{part}</span>
          </Fragment>
        ))}
      </span>
    </button>
  )
}

export const RunListRow = memo(RunListRowImpl)
