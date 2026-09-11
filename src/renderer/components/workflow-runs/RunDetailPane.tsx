import { useEffect } from 'react'
import { useAppStore } from '../../stores'
import { toast } from '../Toast'
import { Check, X, Inbox, LogIn, Play, RotateCcw } from 'lucide-react'
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
import { hasFailedStep } from '@vornrun/shared/workflow-graph'
import { RunStepsList, StatusDot } from '../workflow-editor/RunEntry'
import { RunIcon } from './RunIcon'
import { useConnectorLook, useConnections } from '../../lib/use-connections'
import { nodeConnectionId } from '../workflow-editor/node-visuals'
import { StopRunButton } from './StopRunButton'
import { workflowRunId, type TaskConfig } from '../../../shared/types'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'
import { WORKFLOW_STATUS_TEXT } from '../../lib/workflow-status'
import { GATE_APPROVE, GATE_REJECT } from '../../lib/gate-affordance'

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

const WAIT_LABELS = {
  approval: { status: 'waiting for approval', paused: 'paused for review' },
  signIn: { status: 'waiting for sign-in', paused: 'paused until signed in' }
} as const

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
  // Retry and re-run need the full definition, not the render-only ref.
  const fullWorkflow = useAppStore((s) => s.config?.workflows?.find((w) => w.id === run.workflowId))
  const onLaunchError = (err: unknown): void => {
    toast.error(err instanceof Error ? err.message : String(err))
  }
  const look = useConnectorLook(run.connectorItem?.connectionId)
  const presentation = describeRun(run, workflow, look)
  const outcome = describeOutcome(run, nodes)
  const stages = runStages(run, nodes)
  const done = completedStageCount(stages)
  const summary = runSummaryText(run)
  const waitingStep = run.nodeStates.find((ns) => ns.status === 'waiting')
  const signInWait = waitingStep?.waitingFor === 'signIn'
  const wait = WAIT_LABELS[signInWait ? 'signIn' : 'approval']

  // Keyboard approval mirrors the two visible actions, and only while a gate is
  // actually open — otherwise a stray "r" in the app would resolve nothing.
  // `shortcutsEnabled` lets the view mute them behind a modal, and `repeat` is
  // ignored so holding a key can't reject the run that auto-selects next.
  useEffect(() => {
    if (!waitingStep || signInWait || !shortcutsEnabled) return undefined
    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.repeat) return
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target?.tagName ?? '')) {
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault()
        void window.api.resolveWorkflowGate({
          runId: run.runId,
          nodeId: waitingStep.nodeId,
          decision: 'approve'
        })
      } else if (!e.metaKey && !e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'r') {
        e.preventDefault()
        void window.api.resolveWorkflowGate({
          runId: run.runId,
          nodeId: waitingStep.nodeId,
          decision: 'reject'
        })
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [run, waitingStep, signInWait, shortcutsEnabled])

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
          {run.partial && (
            <span className="text-[9px] font-mono uppercase tracking-wider text-gray-500 border border-white/[0.08] rounded px-1 shrink-0">
              partial
            </span>
          )}
          <span className="flex-1" />
          {hasFailedStep(run) && fullWorkflow && (
            <button
              aria-label="Retry from failed step"
              title="Retry from failed step"
              onClick={() => window.api.retryWorkflowRun(run.runId).catch(onLaunchError)}
              className="p-1 rounded text-gray-500 hover:text-white transition-colors shrink-0"
            >
              <RotateCcw size={13} strokeWidth={2} />
            </button>
          )}
          {run.status !== 'running' && fullWorkflow && (
            <button
              aria-label="Run again"
              title="Run again"
              onClick={() => window.api.rerunWorkflowRun(run.runId).catch(onLaunchError)}
              className="p-1 rounded text-gray-500 hover:text-white transition-colors shrink-0"
            >
              <Play size={13} strokeWidth={2} />
            </button>
          )}
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
            <StatusDot status={waitingStep ? 'waiting' : run.status} />
            {(waitingStep || outcome.label) && (
              <span
                className={`text-[12.5px] ${waitingStep ? WORKFLOW_STATUS_TEXT.waiting : WORKFLOW_STATUS_TEXT[run.status]}`}
              >
                {waitingStep ? wait.status : outcome.label}
              </span>
            )}
          </div>
          <p className="mt-1 text-[11px] text-gray-500 font-mono">
            {done === stages.length && stages.length > 0
              ? 'ran end to end'
              : `${done} of ${stages.length} stages`}
            {' · '}
            {ranUninterrupted(run) ? 'never paused' : wait.paused}
            {' · '}
            {formatRelativeTime(run.startedAt)}
          </p>
          {summary && (
            <pre className="mt-2.5 text-[12px] text-gray-300 font-mono whitespace-pre-wrap break-words leading-relaxed max-h-[160px] overflow-auto">
              {summary}
            </pre>
          )}
        </div>
      </div>

      {waitingStep && (
        <div className="px-5 pb-4 shrink-0 flex flex-col gap-1.5">
          {signInWait ? (
            <SignInButton
              connectionId={nodeConnectionId(nodes.find((n) => n.id === waitingStep.nodeId))}
            />
          ) : (
            <button
              type="button"
              onClick={() =>
                void window.api.resolveWorkflowGate({
                  runId: run.runId,
                  nodeId: waitingStep.nodeId,
                  decision: 'approve'
                })
              }
              className={`flex items-center gap-2 px-4 py-2.5 text-[13px] ${GATE_APPROVE}`}
            >
              <Check size={14} strokeWidth={2} />
              Approve &amp; continue
              <span className="flex-1" />
              <kbd className="text-[10px] font-mono text-gray-500 border border-white/[0.08] rounded px-1 py-0.5">
                ⌘↵
              </kbd>
            </button>
          )}
          <button
            type="button"
            onClick={() =>
              void window.api.resolveWorkflowGate({
                runId: run.runId,
                nodeId: waitingStep.nodeId,
                decision: 'reject'
              })
            }
            className={`flex items-center gap-2 px-4 py-2.5 text-[13px] ${GATE_REJECT}`}
          >
            <X size={14} strokeWidth={2} />
            Reject run
            <span className="flex-1" />
            {!signInWait && (
              <kbd className="text-[10px] font-mono text-gray-500 border border-white/[0.08] rounded px-1 py-0.5">
                R
              </kbd>
            )}
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

/** Opens the sign-in window of the connection a parked step acts through. */
function SignInButton({ connectionId }: { connectionId: string | undefined }) {
  const connections = useConnections()
  const name = connections.find((c) => c.id === connectionId)?.name ?? 'the connection'
  return (
    <button
      type="button"
      disabled={!connectionId}
      onClick={() => connectionId && void window.api.signInConnection(connectionId)}
      className={`flex items-center gap-2 px-4 py-2.5 text-[13px] ${GATE_APPROVE} disabled:opacity-50`}
    >
      <LogIn size={14} strokeWidth={2} />
      Sign in to {name}
    </button>
  )
}
