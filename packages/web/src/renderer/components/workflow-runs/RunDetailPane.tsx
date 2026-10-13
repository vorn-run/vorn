import { useEffect, useState } from 'react'
import { useAppStore } from '../../stores'
import { toast } from '../Toast'
import {
  Check,
  X,
  Inbox,
  Play,
  RotateCcw,
  ExternalLink,
  Eye,
  MessageSquare,
  Pencil
} from 'lucide-react'
import { formatRelativeTime, formatRunDuration } from '../../lib/format-time'
import {
  describeRun,
  runDotStatus,
  runStatusLine,
  runVerdict,
  type RunWorkflowRef
} from '../../lib/run-presentation'
import { canRequestChanges, hasFailedStep, isSignInWait } from '@vornrun/shared/workflow-graph'
import { RunStepsList, StatusDot } from '../workflow-editor/RunEntry'
import { useConnectorLook } from '../../lib/use-connections'
import { nodeConnectionId } from '../workflow-editor/node-visuals'
import { SignInButton } from './SignInButton'
import { StopRunButton } from './StopRunButton'
import { IconButton } from '../IconButton'
import { RunIcon } from './RunIcon'
import { workflowRunId, type ApprovalConfig, type TaskConfig } from '../../../shared/types'
import type { RunListEntry } from '../../hooks/useAllWorkflowRuns'
import { GATE_APPROVE, GATE_NEUTRAL, GATE_REJECT } from '../../lib/gate-affordance'
import { GateAsk, GateComposer, GateEditor } from './GateActions'
import type { GateDraft } from './GateJsonEditor'
import { GateReviewModal } from './GateReviewModal'

export function RunDetailEmptyState() {
  return (
    <div className="h-full flex flex-col items-center justify-center gap-2 text-ink-faint">
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
  // Retry and re-run need the full definition, not the render-only ref.
  const fullWorkflow = useAppStore((s) => s.config?.workflows?.find((w) => w.id === run.workflowId))
  const onLaunchError = (err: unknown): void => {
    toast.error(err instanceof Error ? err.message : String(err))
  }
  const look = useConnectorLook(run.connectorItem?.connectionId)
  const presentation = describeRun(run, workflow, look)
  const waitingStep = run.nodeStates.find((ns) => ns.status === 'waiting')
  const signInWait = waitingStep !== undefined && isSignInWait(waitingStep)
  const gate =
    waitingStep && !signInWait
      ? nodes.find((n) => n.id === waitingStep.nodeId && n.type === 'approval')
      : undefined
  const gateConfig = gate?.config as ApprovalConfig | undefined
  // Keyed to the gate, so a comment or review left open does not follow the pane to another run.
  const openGate = `${run.runId}:${waitingStep?.nodeId ?? ''}`
  const [composingFor, setComposingFor] = useState<string | null>(null)
  const [reviewingFor, setReviewingFor] = useState<string | null>(null)
  const [editingFor, setEditingFor] = useState<string | null>(null)
  const [rewrite, setRewrite] = useState<{ gate: string; text: string } | null>(null)
  const [draft, setDraft] = useState<(GateDraft & { gate: string }) | null>(null)
  const composing = composingFor === openGate
  const reviewing = reviewingFor === openGate
  const editing = editingFor === openGate
  const saved = rewrite?.gate === openGate ? rewrite.text : undefined
  // Approve stays in view under the editor, so while it is open Approve answers
  // with what the reviewer is looking at, and waits while that cannot be sent.
  const live = editing && draft?.gate === openGate ? draft : undefined
  const edited = live ? live.edited : saved
  const blocked = live?.invalid ?? null
  const setComposing = (on: boolean): void => setComposingFor(on ? openGate : null)
  const setReviewing = (on: boolean): void => setReviewingFor(on ? openGate : null)
  const verdict = run.status === 'success' ? runVerdict(run) : undefined
  const meta = [
    workflowName !== presentation.title && workflowName,
    `Run ${workflowRunId(run).slice(0, 8)}`,
    presentation.sourceLabel,
    formatRelativeTime(run.startedAt),
    formatRunDuration(run.startedAt, run.completedAt)
  ]
    .filter(Boolean)
    .join(' · ')

  // Keyboard approval mirrors the two visible actions, and only while a gate is
  // actually open — otherwise a stray "r" in the app would resolve nothing.
  // `shortcutsEnabled` lets the view mute them behind a modal, and `repeat` is
  // ignored so holding a key can't reject the run that auto-selects next.
  useEffect(() => {
    if (!waitingStep || signInWait || !shortcutsEnabled || composing || reviewing || editing) {
      return undefined
    }
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
          decision: 'approve',
          ...(edited && { edited })
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
  }, [run, waitingStep, signInWait, shortcutsEnabled, composing, reviewing, editing, edited])

  return (
    <div className="h-full flex flex-col min-h-0 overflow-y-auto">
      <div className="px-5 pt-4 pb-4 shrink-0 flex flex-col gap-2">
        <div className="flex items-center gap-1 min-w-0">
          <span className="flex shrink-0 mr-1">
            <RunIcon presentation={presentation} size={15} className="text-ink-secondary" />
          </span>
          <h2 className="flex-1 min-w-0 truncate text-[15px] font-medium text-ink">
            {presentation.title}
          </h2>
          {hasFailedStep(run) && fullWorkflow && (
            <IconButton
              label="Retry from failed step"
              position="bottom"
              onClick={() => window.api.retryWorkflowRun(run.runId).catch(onLaunchError)}
            >
              <RotateCcw size={14} strokeWidth={1.75} />
            </IconButton>
          )}
          {run.status !== 'running' && fullWorkflow && (
            <IconButton
              label="Run again"
              position="bottom"
              onClick={() => window.api.rerunWorkflowRun(run.runId).catch(onLaunchError)}
            >
              <Play size={14} strokeWidth={1.75} />
            </IconButton>
          )}
          <IconButton
            label="Open workflow"
            position="bottom"
            title={workflowDeleted ? 'Workflow no longer exists' : undefined}
            disabled={workflowDeleted}
            onClick={onOpenWorkflow}
          >
            <ExternalLink size={14} strokeWidth={1.75} />
          </IconButton>
          <StopRunButton execution={run} stopPropagation={false} />
        </div>
        <p className="text-[12px] text-ink-faint truncate">{meta}</p>
        {presentation.subtitle && (
          <p className="text-[13px] text-ink-secondary">{presentation.subtitle}</p>
        )}
        <div className="flex items-center gap-2 text-[13px] text-ink">
          <StatusDot status={waitingStep ? 'waiting' : runDotStatus(run)} />
          <span>{runStatusLine(run, nodes)}</span>
          {verdict && (
            <>
              <span className="text-ink-faint">·</span>
              <span className="text-ink-secondary">{verdict}</span>
            </>
          )}
        </div>
      </div>

      {waitingStep && (
        <div className="px-5 pb-4 shrink-0 flex flex-col gap-1.5">
          {gate && waitingStep && (
            <>
              <GateAsk state={waitingStep} config={gateConfig} />
              {editing ? (
                <GateEditor
                  state={{ ...waitingStep, ...(saved !== undefined && { editedText: saved }) }}
                  onSave={(next) => {
                    setRewrite(next === undefined ? null : { gate: openGate, text: next })
                    setEditingFor(null)
                    setDraft(null)
                  }}
                  onCancel={() => {
                    setEditingFor(null)
                    setDraft(null)
                  }}
                  onDraftChange={(next) => setDraft({ ...next, gate: openGate })}
                  large
                />
              ) : composing ? (
                <GateComposer
                  runId={run.runId}
                  state={waitingStep}
                  config={gateConfig}
                  nodes={nodes}
                  kind="changes"
                  edited={edited}
                  onDone={() => setComposing(false)}
                  large
                />
              ) : (
                (waitingStep.viewToken ||
                  waitingStep.editableText !== undefined ||
                  canRequestChanges(gateConfig, waitingStep)) && (
                  <div className="flex gap-1.5">
                    {waitingStep.editableText !== undefined && (
                      <button
                        type="button"
                        onClick={() => setEditingFor(openGate)}
                        className={`flex-1 flex items-center justify-center gap-2 px-4 py-2 text-[12.5px] ${GATE_NEUTRAL}`}
                      >
                        <Pencil size={13} strokeWidth={1.75} />
                        {saved ? 'Edited' : 'Edit'}
                      </button>
                    )}
                    {waitingStep.viewToken && (
                      <button
                        type="button"
                        onClick={() => setReviewing(true)}
                        className={`flex-1 flex items-center justify-center gap-2 px-4 py-2 text-[12.5px] ${GATE_NEUTRAL}`}
                      >
                        <Eye size={13} strokeWidth={1.75} />
                        Open review
                      </button>
                    )}
                    {canRequestChanges(gateConfig, waitingStep) && (
                      <button
                        type="button"
                        onClick={() => setComposing(true)}
                        className={`flex-1 flex items-center justify-center gap-2 px-4 py-2 text-[12.5px] ${GATE_NEUTRAL}`}
                      >
                        <MessageSquare size={13} strokeWidth={1.75} />
                        Request changes
                      </button>
                    )}
                  </div>
                )
              )}
              {reviewing && (
                <GateReviewModal
                  runId={run.runId}
                  workflowName={workflowName}
                  state={waitingStep}
                  node={gate}
                  nodes={nodes}
                  onClose={() => setReviewing(false)}
                />
              )}
            </>
          )}
          {signInWait ? (
            <SignInButton
              connectionId={nodeConnectionId(nodes.find((n) => n.id === waitingStep.nodeId))}
            />
          ) : (
            <button
              type="button"
              disabled={blocked !== null}
              title={blocked ? `Cannot approve yet. ${blocked}` : undefined}
              onClick={() =>
                void window.api.resolveWorkflowGate({
                  runId: run.runId,
                  nodeId: waitingStep.nodeId,
                  decision: 'approve',
                  ...(edited && { edited })
                })
              }
              className={`flex items-center gap-2 px-4 py-2.5 text-[13px] ${GATE_APPROVE} disabled:opacity-40 disabled:cursor-not-allowed`}
            >
              <Check size={14} strokeWidth={2} />
              Approve &amp; continue
              <span className="flex-1" />
              <kbd className="text-[10px] font-mono text-ink-faint border border-white/[0.08] rounded px-1 py-0.5">
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
              <kbd className="text-[10px] font-mono text-ink-faint border border-white/[0.08] rounded px-1 py-0.5">
                R
              </kbd>
            )}
          </button>
        </div>
      )}

      <div className="px-5 pb-6">
        <RunStepsList
          execution={run}
          nodes={nodes}
          tasks={tasks}
          includeTrigger
          onViewFullOutput={onViewFullOutput}
        />
      </div>
    </div>
  )
}
