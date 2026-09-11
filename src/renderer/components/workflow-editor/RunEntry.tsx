import { useState, useEffect, useMemo, useRef } from 'react'
import { GATE_APPROVE, GATE_REJECT } from '../../lib/gate-affordance'
import { ChevronDown, ChevronRight, Maximize2, Play, RotateCcw, Check, X } from 'lucide-react'
import {
  WorkflowExecution,
  WorkflowNode,
  NodeExecutionState,
  TaskConfig,
  AiAgentType,
  ApprovalConfig,
  supportsExactSessionResume
} from '../../../shared/types'

import { formatRelativeTime, formatRunDuration } from '../../lib/format-time'
import { WORKFLOW_STATUS_DOT_PULSE, WORKFLOW_STATUS_DOT } from '../../lib/workflow-status'
import { nodeLabel } from '../../lib/run-presentation'
import { IconButton } from '../IconButton'
import { failedStep, hasFailedStep, isSignInWait } from '@vornrun/shared/workflow-graph'
import { StopRunButton } from '../workflow-runs/StopRunButton'
import { connectorLookFor, useConnections } from '../../lib/use-connections'
import { SignInButton } from '../workflow-runs/SignInButton'
import {
  TASK_CHIP,
  nodeConnectionId,
  stepMeta,
  stepTimeline,
  stepOutputPreview,
  stepPreview
} from './node-visuals'

const STATUS_LABELS: Record<WorkflowExecution['status'] | NodeExecutionState['status'], string> = {
  success: 'Success',
  error: 'Error',
  running: 'Running',
  pending: 'Pending',
  skipped: 'Skipped',
  waiting: 'Waiting for approval',
  cancelled: 'Stopped'
}

export function StatusDot({
  status
}: {
  status: WorkflowExecution['status'] | NodeExecutionState['status']
}) {
  const label = STATUS_LABELS[status] ?? 'Unknown'
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`w-2 h-2 rounded-full shrink-0 ${WORKFLOW_STATUS_DOT_PULSE[status] ?? WORKFLOW_STATUS_DOT.pending}`}
    />
  )
}

export function NodeLabel({ nodeId, nodes }: { nodeId: string; nodes: WorkflowNode[] }) {
  return (
    <span>
      {nodeLabel(
        nodes.find((n) => n.id === nodeId),
        nodeId
      )}
    </span>
  )
}

interface RunStepsListProps {
  execution: WorkflowExecution
  nodes: WorkflowNode[]
  tasks?: TaskConfig[]
  /** Show the trigger node as the first stage. Run History hides it (the
   *  trigger is implied by the workflow); the Inbox trace shows it because
   *  "what fired this" is the first thing you read. */
  includeTrigger?: boolean
  onViewFullOutput?: (logs: string) => void
  onClickTask?: (taskId: string) => void
  /** Auto-expand whichever step is running and keep it scrolled into view. */
  followActive?: boolean
  onResumeSession?: (
    agentSessionId: string,
    agentType: AiAgentType,
    projectName: string,
    projectPath: string,
    branch?: string,
    useWorktree?: boolean
  ) => void
}

const MAX_INPUT_PREVIEW = 60

/** One-line preview of a run input. Object-valued inputs (a picked connector
 *  item) get JSON-serialized and clipped so a large payload can't push the
 *  steps off screen. */
function formatInputValue(value: unknown): string {
  const text =
    value == null ? '' : typeof value === 'object' ? JSON.stringify(value) : String(value)
  return text.length > MAX_INPUT_PREVIEW ? `${text.slice(0, MAX_INPUT_PREVIEW)}…` : text
}

/** Lines up what a step shows beneath it with its name, past the dot. */
const UNDER_LABEL = 'pl-[34px] pr-4 pb-2.5'

const EMPTY_LOG: Partial<Record<NodeExecutionState['status'], string>> = {
  running: 'No output captured yet…',
  pending: "Step hasn't started yet.",
  skipped: 'Step was skipped.'
}

export function RunStepsList({
  execution,
  nodes,
  tasks,
  includeTrigger = false,
  onViewFullOutput,
  onClickTask,
  followActive,
  onResumeSession
}: RunStepsListProps) {
  const activeNodeId = followActive
    ? (execution.nodeStates.find((ns) => ns.status === 'running')?.nodeId ?? null)
    : null
  // The step worth reading opens by itself: the running one when following, else where a failed run broke.
  const focus =
    activeNodeId ?? (execution.status === 'error' ? (failedStep(execution)?.nodeId ?? null) : null)
  const [expandedNodeId, setExpandedNodeId] = useState(focus)
  const [openedFor, setOpenedFor] = useState(focus)
  if (openedFor !== focus) {
    setOpenedFor(focus)
    if (focus) setExpandedNodeId(focus)
  }
  const expandedRowRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (followActive && expandedNodeId) {
      expandedRowRef.current?.scrollIntoView?.({ block: 'nearest' })
    }
  }, [followActive, expandedNodeId])
  const connections = useConnections()
  const nodesById = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes])

  const actionStates = includeTrigger
    ? execution.nodeStates
    : execution.nodeStates.filter((ns) => nodesById.get(ns.nodeId)?.type !== 'trigger')

  const triggerTask =
    execution.triggerTaskId && tasks
      ? tasks.find((t) => t.id === execution.triggerTaskId)
      : undefined
  const inputs = Object.entries(execution.inputs ?? {})

  return (
    <div className="border-t border-white/[0.06]">
      {inputs.length > 0 && (
        <div className="px-4 py-2 border-b border-white/[0.05] flex items-baseline gap-3 min-w-0">
          <span className="text-[10px] uppercase tracking-wider text-ink-faint shrink-0">
            Inputs
          </span>
          <div className="flex flex-wrap gap-x-3 gap-y-0.5 min-w-0 text-[11.5px] font-mono">
            {inputs.map(([key, value]) => (
              <span
                key={key}
                className="max-w-full truncate"
                title={`${key}=${formatInputValue(value)}`}
              >
                <span className="text-ink-faint">{key}</span>{' '}
                <span className="text-ink-secondary">{formatInputValue(value)}</span>
              </span>
            ))}
          </div>
        </div>
      )}
      {actionStates.map((ns) => {
        const nodeTask = ns.taskId && tasks ? tasks.find((t) => t.id === ns.taskId) : undefined
        const node = nodesById.get(ns.nodeId)
        const nodeConfig = node?.config as
          | {
              agentType?: AiAgentType | 'fromTask'
              projectName?: string
              projectPath?: string
              branch?: string
              useWorktree?: boolean
            }
          | undefined

        const configAgent =
          nodeConfig?.agentType && nodeConfig.agentType !== 'fromTask'
            ? nodeConfig.agentType
            : undefined
        const resumeAiAgentType: AiAgentType | undefined = ns.agentType ?? configAgent
        const resumeProjectName =
          ns.projectName ||
          nodeConfig?.projectName ||
          nodeTask?.projectName ||
          triggerTask?.projectName ||
          ''
        const resumeProjectPath = ns.projectPath || nodeConfig?.projectPath || ''
        const resumeBranch = nodeConfig?.branch ?? nodeTask?.branch ?? triggerTask?.branch
        const resumeUseWorktree =
          nodeConfig?.useWorktree ?? nodeTask?.useWorktree ?? triggerTask?.useWorktree
        const canResume =
          !!ns.agentSessionId &&
          !!onResumeSession &&
          !!resumeAiAgentType &&
          !!resumeProjectName &&
          supportsExactSessionResume(resumeAiAgentType)
        const handleResume = (): void =>
          onResumeSession!(
            ns.agentSessionId!,
            resumeAiAgentType!,
            resumeProjectName,
            resumeProjectPath,
            resumeBranch,
            resumeUseWorktree
          )

        const look = connectorLookFor(connections, nodeConnectionId(node))
        const meta = stepMeta(node, look?.connectorId)
        // What the step said beats what it was told to do, so a row is never blank.
        const preview = stepOutputPreview(ns) ?? stepPreview(node)
        // Only for the open row: stepTimeline slices a tail out of every step's logs.
        const isExpanded = expandedNodeId === ns.nodeId
        const timeline = isExpanded ? stepTimeline(ns.logs, ns.diagnostics) : []
        const canViewFull = !!onViewFullOutput && !!ns.logs && timeline.length > 0

        const isWaitingGate = ns.status === 'waiting' && node?.type === 'approval'
        const signInWait = isSignInWait(ns)
        const approvalMessage =
          node?.type === 'approval' ? (node.config as ApprovalConfig).message : undefined
        const faint = ns.status === 'pending' || ns.status === 'skipped'

        return (
          <div
            key={ns.nodeId}
            ref={isExpanded ? expandedRowRef : undefined}
            className="border-b border-white/[0.05] last:border-b-0"
          >
            <button
              type="button"
              aria-expanded={isExpanded}
              onClick={() => setExpandedNodeId(isExpanded ? null : ns.nodeId)}
              className="w-full grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 px-4 py-2 text-left hover:bg-white/[0.02] transition-colors"
            >
              <StatusDot status={ns.status} />
              <span
                className={`flex items-center gap-1.5 min-w-0 text-[12.5px] ${faint ? 'text-ink-faint' : 'text-ink'}`}
              >
                <span className="truncate">{nodeLabel(node, ns.nodeId)}</span>
                {meta && <span className="text-[11.5px] text-ink-faint truncate">{meta}</span>}
                {nodeTask && (
                  <span
                    className={`${TASK_CHIP} max-w-[80px]`}
                    onClick={(e) => {
                      e.stopPropagation()
                      onClickTask?.(nodeTask.id)
                    }}
                    title={nodeTask.title}
                  >
                    {nodeTask.title}
                  </span>
                )}
              </span>
              <span className="font-mono text-[12px] text-ink-secondary tabular-nums">
                {ns.startedAt && ns.completedAt
                  ? formatRunDuration(ns.startedAt, ns.completedAt)
                  : null}
              </span>
              {preview && !isExpanded && !isWaitingGate && (
                <span className="col-start-2 col-span-2 text-[12px] font-mono text-ink-faint truncate">
                  {preview}
                </span>
              )}
            </button>

            {isWaitingGate && (
              <div className={`${UNDER_LABEL} flex items-start gap-2`}>
                <div className="flex-1 min-w-0 text-[11.5px] text-bronzo">
                  {approvalMessage || 'Waiting for approval.'}
                </div>
                <button
                  onClick={() => {
                    void window.api.resolveWorkflowGate({
                      runId: execution.runId,
                      nodeId: ns.nodeId,
                      decision: 'approve'
                    })
                  }}
                  className={`flex items-center gap-1 px-2 py-1 text-[11px] shrink-0 ${GATE_APPROVE}`}
                >
                  <Check size={11} strokeWidth={2.5} />
                  Approve
                </button>
                <button
                  onClick={() => {
                    void window.api.resolveWorkflowGate({
                      runId: execution.runId,
                      nodeId: ns.nodeId,
                      decision: 'reject'
                    })
                  }}
                  className={`flex items-center gap-1 px-2 py-1 text-[11px] shrink-0 ${GATE_REJECT}`}
                >
                  <X size={11} strokeWidth={2.5} />
                  Reject
                </button>
              </div>
            )}

            {signInWait && (
              <div className={`${UNDER_LABEL} flex items-start gap-2`}>
                <div className="flex-1 min-w-0 text-[11.5px] text-bronzo">
                  {ns.error || 'Signed out. Sign in, and this step runs again.'}
                </div>
                <SignInButton connectionId={nodeConnectionId(node)} compact />
              </div>
            )}

            {/* Engine lines are dimmed under the agent's own words, and the log is the trace's only box. */}
            {isExpanded && (
              <div className={`${UNDER_LABEL} flex flex-col gap-1.5`}>
                {ns.error && <p className="text-[12px] text-danger">{ns.error}</p>}
                {timeline.length > 0 && (
                  <div className="bg-black/30 border border-white/[0.05] rounded overflow-auto max-h-[280px]">
                    {timeline.map((entry, ti) =>
                      entry.kind === 'agent' ? (
                        <pre
                          key={ti}
                          className="text-[12px] text-ink-secondary px-2.5 py-1.5
                                     font-mono whitespace-pre-wrap break-all leading-relaxed"
                        >
                          {entry.text}
                        </pre>
                      ) : (
                        <p
                          key={ti}
                          className="text-[12px] text-ink-faint font-mono px-2.5 py-0.5
                                     bg-white/[0.02] whitespace-pre-wrap break-all leading-relaxed"
                        >
                          {entry.text}
                        </p>
                      )
                    )}
                  </div>
                )}
                {timeline.length === 0 && !ns.error && (
                  <p className="text-[11.5px] text-ink-faint italic">
                    {EMPTY_LOG[ns.status] ?? 'No output recorded.'}
                  </p>
                )}
                {(canViewFull || canResume) && (
                  <div className="flex items-center gap-1">
                    {canViewFull && (
                      <IconButton
                        label="View full output"
                        onClick={() => onViewFullOutput!(ns.logs!)}
                      >
                        <Maximize2 size={12} strokeWidth={2} />
                      </IconButton>
                    )}
                    {canResume && (
                      <IconButton label="Resume session" onClick={handleResume}>
                        <RotateCcw size={12} strokeWidth={2} />
                      </IconButton>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

interface RunEntryProps {
  execution: WorkflowExecution
  nodes: WorkflowNode[]
  workflowName?: string
  tasks?: TaskConfig[]
  onViewFullOutput?: (logs: string) => void
  onClickTask?: (taskId: string) => void
  /** Start a fresh run with this run's launch context. */
  onRerunRun?: (execution: WorkflowExecution) => void
  /** Resume this failed run from its failed step, reusing completed outputs. */
  onRetryRun?: (execution: WorkflowExecution) => void
  /** Keep this run expanded and its active step in view while it streams. */
  follow?: boolean
  onResumeSession?: (
    agentSessionId: string,
    agentType: AiAgentType,
    projectName: string,
    projectPath: string,
    branch?: string,
    useWorktree?: boolean
  ) => void
}

export function RunEntry({
  execution,
  nodes,
  workflowName,
  tasks,
  onViewFullOutput,
  onClickTask,
  onRerunRun,
  onRetryRun,
  follow,
  onResumeSession
}: RunEntryProps) {
  const hasWaitingGate = execution.nodeStates.some((ns) => ns.status === 'waiting')
  const [expanded, setExpanded] = useState(hasWaitingGate || !!follow)

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (hasWaitingGate || follow) setExpanded(true)
  }, [hasWaitingGate, follow])

  const triggerTask =
    execution.triggerTaskId && tasks
      ? tasks.find((t) => t.id === execution.triggerTaskId)
      : undefined

  return (
    <div className="border border-white/[0.08] rounded overflow-hidden">
      {/* Run header — the toggle and the stop control are siblings so the
          stop button isn't nested inside the row's button. */}
      <div className="flex items-center hover:bg-white/[0.03] transition-colors">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex-1 min-w-0 flex items-center gap-2 px-3 py-2.5 text-left"
        >
          {expanded ? (
            <ChevronDown size={12} className="text-ink-faint" />
          ) : (
            <ChevronRight size={12} className="text-ink-faint" />
          )}
          <StatusDot status={execution.status} />
          <span className="text-[12px] text-ink flex-1 min-w-0 truncate">
            {workflowName && <span className="text-ink-faint mr-1.5">{workflowName}</span>}
            {formatRelativeTime(execution.startedAt)}
          </span>
          {triggerTask && (
            <span
              className={`${TASK_CHIP} max-w-[100px] shrink-0`}
              onClick={(e) => {
                e.stopPropagation()
                onClickTask?.(triggerTask.id)
              }}
              title={triggerTask.title}
            >
              {triggerTask.title}
            </span>
          )}
          {execution.partial && (
            <span className="text-[11px] text-ink-faint shrink-0">partial</span>
          )}
          <span className="font-mono text-[11px] text-ink-secondary tabular-nums shrink-0">
            {formatRunDuration(execution.startedAt, execution.completedAt)}
          </span>
        </button>
        {hasFailedStep(execution) && onRetryRun && (
          <IconButton
            label="Retry from failed step"
            position="top"
            onClick={() => onRetryRun(execution)}
          >
            <RotateCcw size={12} strokeWidth={2} />
          </IconButton>
        )}
        {execution.status !== 'running' && onRerunRun && (
          <IconButton label="Run again" position="top" onClick={() => onRerunRun(execution)}>
            <Play size={12} strokeWidth={2} />
          </IconButton>
        )}
        <span className="pr-2 shrink-0">
          <StopRunButton execution={execution} stopPropagation={false} />
        </span>
      </div>

      {expanded && (
        <RunStepsList
          execution={execution}
          nodes={nodes}
          tasks={tasks}
          onViewFullOutput={onViewFullOutput}
          onClickTask={onClickTask}
          onResumeSession={onResumeSession}
          followActive={follow}
        />
      )}
    </div>
  )
}
