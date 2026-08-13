import { memo } from 'react'
import { formatRelativeTime } from '../../lib/format-time'
import {
  describeOutcome,
  describeRun,
  outcomeToneClass,
  runStages,
  type RunStage,
  type RunWorkflowRef
} from '../../lib/run-presentation'
import { WORKFLOW_STATUS_DOT_PULSE, WORKFLOW_STATUS_DOT } from '../../lib/workflow-status'
import { RunIcon } from './RunIcon'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'

/** Cap so a 40-node workflow doesn't render a hairline-thin bar. */
const MAX_SEGMENTS = 12

function StageBar({ stages }: { stages: RunStage[] }) {
  const shown = stages.slice(0, MAX_SEGMENTS)
  const overflow = stages.length - shown.length
  return (
    <span className="flex items-center gap-[3px] shrink-0" aria-hidden="true">
      {shown.map((stage) => (
        <span
          key={stage.nodeId}
          title={`${stage.label} · ${stage.status}`}
          className={`h-[3px] w-3 rounded-full ${
            stage.status === 'pending' ? 'bg-white/[0.10]' : stage.dotClass
          }`}
        />
      ))}
      {overflow > 0 && <span className="text-[10px] text-gray-600 ml-0.5">+{overflow}</span>}
    </span>
  )
}

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
  const presentation = describeRun(run, workflow)
  const stages = runStages(run, nodes)
  const outcome = describeOutcome(run, nodes)
  const dotStatus = run.nodeStates.some((n) => n.status === 'waiting') ? 'waiting' : run.status

  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      onDoubleClick={workflowDeleted ? undefined : onOpenWorkflow}
      className={`relative w-full text-left px-4 py-3 border-b border-white/[0.04] transition-colors ${
        selected ? 'bg-white/[0.05]' : 'hover:bg-white/[0.03]'
      }`}
    >
      {selected && <span className="absolute left-0 top-1 bottom-1 w-px bg-white rounded-full" />}

      <span className="flex items-center gap-2 min-w-0">
        <span
          role="img"
          aria-label={dotStatus}
          className={`w-1.5 h-1.5 rounded-full shrink-0 ${WORKFLOW_STATUS_DOT_PULSE[dotStatus] ?? WORKFLOW_STATUS_DOT.pending}`}
        />
        <RunIcon presentation={presentation} />
        <span
          className={`font-mono text-[12.5px] truncate min-w-0 ${selected ? 'text-white' : 'text-gray-200'}`}
        >
          {presentation.title}
        </span>
        {workflowDeleted && (
          <span className="text-[10px] uppercase tracking-wide text-gray-600 shrink-0">
            deleted
          </span>
        )}
        <span className="flex-1" />
        <span className="text-[11px] text-gray-500 tabular-nums shrink-0">
          {formatRelativeTime(run.startedAt)}
        </span>
      </span>

      {presentation.subtitle && (
        <span className="block mt-1.5 text-[12px] text-gray-400 truncate">
          {presentation.subtitle}
        </span>
      )}

      <span className="flex items-center gap-2 mt-2 min-w-0">
        <StageBar stages={stages} />
        <span className={`text-[11px] truncate min-w-0 ${outcomeToneClass(outcome.tone)}`}>
          {outcome.label}
        </span>
      </span>
    </button>
  )
}

export const RunListRow = memo(RunListRowImpl)
