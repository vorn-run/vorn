import { useEffect } from 'react'
import { Check, X, Inbox } from 'lucide-react'
import { formatRelativeTime, formatRunDuration } from '../../lib/format-time'
import {
  completedStageCount,
  describeOutcome,
  describeRun,
  ranUninterrupted,
  runStages,
  runSummaryText,
  type RunWorkflowRef
} from '../../lib/run-presentation'
import { approveWorkflowGate, rejectWorkflowGate } from '../../lib/workflow-execution'
import { RunStepsList, StatusDot } from '../workflow-editor/RunEntry'
import { RunIcon } from './RunIcon'
import { StopRunButton } from './StopRunButton'
import { workflowRunId, type TaskConfig } from '../../../shared/types'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'

const RUN_STATUS_TEXT = {
  running: 'text-blue-400',
  success: 'text-green-400',
  error: 'text-red-400',
  cancelled: 'text-gray-400'
} as const

export function RunDetailEmptyState() {
  return (
    <div className="h-full flex flex-col items-center justify-center gap-2 text-gray-600">
      <Inbox size={22} strokeWidth={1.5} />
      <p className="text-[12px]">Select a run to see its trace</p>
    </div>
  )
}

interface Props {
  run: RunListEntry
  workflow?: RunWorkflowRef
  workflowDeleted: boolean
  tasks?: TaskConfig[]
  /** False while something is layered over the pane (the log modal), so a
   *  keystroke meant for that surface can't resolve the gate underneath. */
  shortcutsEnabled?: boolean
  onOpenWorkflow: () => void
  onViewFullOutput?: (logs: string) => void
}

export function RunDetailPane({
  run,
  workflow,
  workflowDeleted,
  tasks,
  shortcutsEnabled = true,
  onOpenWorkflow,
  onViewFullOutput
}: Props) {
  const nodes = workflow?.nodes ?? []
  const workflowName = workflow?.name?.trim() || undefined
  const presentation = describeRun(run, workflow)
  const outcome = describeOutcome(run, nodes)
  const stages = runStages(run, nodes)
  const done = completedStageCount(stages)
  const summary = runSummaryText(run)
  const waitingGate = run.nodeStates.find((ns) => ns.status === 'waiting')

  // Keyboard approval mirrors the two visible actions, and only while a gate is
  // actually open — otherwise a stray "r" in the app would resolve nothing.
  // `shortcutsEnabled` lets the view mute them behind a modal, and `repeat` is
  // ignored so holding a key can't reject the run that auto-selects next.
  useEffect(() => {
    if (!waitingGate || !shortcutsEnabled) return undefined
    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.repeat) return
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target?.tagName ?? '')) {
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault()
        void approveWorkflowGate(run, waitingGate.nodeId)
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'r') {
        e.preventDefault()
        void rejectWorkflowGate(run, waitingGate.nodeId)
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [run, waitingGate, shortcutsEnabled])

  return (
    <div className="h-full flex flex-col min-h-0 overflow-y-auto">
      <div className="px-5 pt-4 pb-3 shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <RunIcon presentation={presentation} size={15} className="text-gray-300 shrink-0" />
          <h2 className="text-[15px] text-white font-mono truncate min-w-0">
            {presentation.title}
          </h2>
          <span className="text-[10px] px-1.5 py-0.5 rounded border border-white/[0.08] text-gray-400 shrink-0">
            {presentation.sourceLabel}
          </span>
          <span className="flex-1" />
          <StopRunButton execution={run} stopPropagation={false} />
        </div>
        <p className="mt-1.5 flex items-center gap-1.5 text-[11px] text-gray-500 font-mono truncate">
          {workflowName && workflowName !== presentation.title && (
            <>
              <span className="truncate">{workflowName}</span>
              <span>·</span>
            </>
          )}
          <span>run {workflowRunId(run).slice(0, 8)}</span>
          <span>·</span>
          <span>{formatRelativeTime(run.startedAt)}</span>
          <span>·</span>
          <span>{formatRunDuration(run.startedAt, run.completedAt)}</span>
        </p>
        {presentation.subtitle && (
          <p className="mt-2 text-[13px] text-gray-300">{presentation.subtitle}</p>
        )}
      </div>

      <div className="px-5 pb-4 shrink-0">
        <div className="rounded-md border border-white/[0.06] bg-white/[0.02] px-4 py-3">
          <div className="flex items-center gap-2">
            <StatusDot status={waitingGate ? 'waiting' : run.status} />
            <span
              className={`text-[12.5px] ${waitingGate ? 'text-amber-400' : RUN_STATUS_TEXT[run.status]}`}
            >
              {waitingGate ? 'waiting for approval' : outcome.label}
            </span>
          </div>
          <p className="mt-1 text-[11px] text-gray-500 font-mono">
            {done === stages.length && stages.length > 0
              ? 'ran end to end'
              : `${done} of ${stages.length} stages`}
            {' · '}
            {ranUninterrupted(run) ? 'never paused' : 'paused for review'}
            {' · '}
            {formatRelativeTime(run.startedAt)}
          </p>
          {summary && (
            <pre className="mt-2.5 text-[11.5px] text-gray-300 font-mono whitespace-pre-wrap break-words leading-relaxed max-h-[160px] overflow-auto">
              {summary}
            </pre>
          )}
        </div>
      </div>

      {waitingGate && (
        <div className="px-5 pb-4 shrink-0 flex flex-col gap-1.5">
          <button
            type="button"
            onClick={() => void approveWorkflowGate(run, waitingGate.nodeId)}
            className="flex items-center gap-2 px-4 py-2.5 rounded-md border border-blue-500/40 bg-blue-500/10
                       text-[13px] text-blue-300 hover:bg-blue-500/20 hover:text-blue-200 transition-colors"
          >
            <Check size={14} strokeWidth={2} />
            Approve &amp; continue
            <span className="flex-1" />
            <kbd className="text-[10px] font-mono text-gray-500 border border-white/[0.08] rounded px-1 py-0.5">
              ⌘↵
            </kbd>
          </button>
          <button
            type="button"
            onClick={() => void rejectWorkflowGate(run, waitingGate.nodeId)}
            className="flex items-center gap-2 px-4 py-2.5 rounded-md border border-white/[0.06]
                       text-[13px] text-gray-300 hover:bg-red-500/10 hover:text-red-300 hover:border-red-500/30 transition-colors"
          >
            <X size={14} strokeWidth={2} />
            Reject run
            <span className="flex-1" />
            <kbd className="text-[10px] font-mono text-gray-500 border border-white/[0.08] rounded px-1 py-0.5">
              R
            </kbd>
          </button>
        </div>
      )}

      <div className="border-t border-white/[0.04] px-5 py-3 flex items-center gap-2 shrink-0">
        <span className="text-[10px] uppercase tracking-wider text-gray-600">Run trace</span>
        <span className="text-[11px] text-gray-500 font-mono">
          {done} of {stages.length} stages complete
        </span>
        <span className="flex-1" />
        <button
          type="button"
          disabled={workflowDeleted}
          onClick={onOpenWorkflow}
          title={workflowDeleted ? 'Workflow no longer exists' : undefined}
          className="px-2 py-1 text-[11px] text-gray-400 border border-white/[0.08] rounded hover:bg-white/[0.04] hover:text-white
                     disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-gray-400"
        >
          Open workflow
        </button>
      </div>

      <div className="px-5 pb-6">
        <div className="rounded-md border border-white/[0.06] overflow-hidden">
          <RunStepsList
            execution={run}
            nodes={nodes}
            tasks={tasks}
            includeTrigger
            onViewFullOutput={onViewFullOutput}
          />
        </div>
      </div>
    </div>
  )
}
