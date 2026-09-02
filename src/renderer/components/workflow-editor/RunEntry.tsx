import { useState, useEffect, useRef } from 'react'
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
import { Tooltip } from '../Tooltip'
import { approveWorkflowGate, rejectWorkflowGate } from '../../lib/workflow-execution'
import { StopRunButton } from '../workflow-runs/StopRunButton'
import { ConnectorIcon } from '../ConnectorIcon'
import { connectorLookFor, useConnections, type ConnectorLook } from '../../lib/use-connections'
import {
  NODE_TYPE_ICON,
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
  const node = nodes.find((n) => n.id === nodeId)
  return <span>{node?.label || nodeId.slice(0, 8)}</span>
}

/**
 * Glyph for one step of a run. A step bound to a connection shows that
 * connector's brand mark — "GitHub Trigger" reads as GitHub — and every other
 * step shows the icon and tint of its node type.
 */
function StepIcon({
  node,
  look
}: {
  node: WorkflowNode | undefined
  look: ConnectorLook | undefined
}) {
  if (look) {
    return (
      <ConnectorIcon
        connectorId={look.connectorId}
        icon={look.icon}
        size={12}
        className="text-gray-400 shrink-0"
      />
    )
  }
  const visual = node ? NODE_TYPE_ICON[node.type] : undefined
  if (!visual) return null
  const Icon = visual
  // Deliberately neutral: a trace is read for its status dots, and tinting
  // every step by node type turns the list into a rainbow that competes with
  // them. The glyph carries the type; the colour carries the outcome.
  return <Icon size={12} strokeWidth={1.5} className="text-gray-400 shrink-0" />
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
  const [expandedNodeId, setExpandedNodeId] = useState<string | null>(null)
  const activeNodeId = followActive
    ? (execution.nodeStates.find((ns) => ns.status === 'running')?.nodeId ?? null)
    : null
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (activeNodeId) setExpandedNodeId(activeNodeId)
  }, [activeNodeId])
  const expandedRowRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (followActive && expandedNodeId) {
      expandedRowRef.current?.scrollIntoView?.({ block: 'nearest' })
    }
  }, [followActive, expandedNodeId])
  const connections = useConnections()

  const actionStates = includeTrigger
    ? execution.nodeStates
    : execution.nodeStates.filter((ns) => {
        const node = nodes.find((n) => n.id === ns.nodeId)
        return node?.type !== 'trigger'
      })

  const triggerTask =
    execution.triggerTaskId && tasks
      ? tasks.find((t) => t.id === execution.triggerTaskId)
      : undefined

  return (
    <div className="border-t border-white/[0.06]">
      {execution.inputs && Object.keys(execution.inputs).length > 0 && (
        <div className="px-4 py-2 border-b border-white/[0.04] flex items-baseline gap-2">
          <span className="text-[10px] uppercase tracking-wider text-gray-600 shrink-0">
            Inputs
          </span>
          <div className="flex flex-wrap gap-1.5 min-w-0">
            {Object.entries(execution.inputs).map(([key, value]) => (
              <span
                key={key}
                className="inline-flex items-baseline gap-1 max-w-full px-1.5 py-0.5 rounded bg-white/[0.04] border border-white/[0.06] text-[11px] font-mono"
                title={`${key}=${formatInputValue(value)}`}
              >
                <span className="text-gray-500 shrink-0">{key}</span>
                <span className="text-gray-300 truncate">{formatInputValue(value)}</span>
              </span>
            ))}
          </div>
        </div>
      )}
      <div className="p-3 space-y-0">
        {actionStates.map((ns, i) => {
          const nodeTask = ns.taskId && tasks ? tasks.find((t) => t.id === ns.taskId) : undefined
          const node = nodes.find((n) => n.id === ns.nodeId)
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
          // What the step said beats what it was told to do. A step with no
          // output yet (a trigger, a pending step) still shows its configured
          // body, so a card is never blank.
          const preview = stepOutputPreview(ns) ?? stepPreview(node)
          // Only for the open card: stepTimeline slices a tail out of every
          // step's logs, and a run with a dozen noisy steps would pay for all
          // of them on every render to show one.
          const isExpanded = expandedNodeId === ns.nodeId
          const timeline = isExpanded ? stepTimeline(ns.logs, ns.diagnostics) : []

          const isWaitingGate = ns.status === 'waiting' && node?.type === 'approval'
          const approvalMessage =
            node?.type === 'approval' ? (node.config as ApprovalConfig).message : undefined

          return (
            <div key={ns.nodeId} ref={isExpanded ? expandedRowRef : undefined}>
              {/* Line linking the previous step to this one so the cards read
                  as one continuous flow. Neutral on purpose: the status dots
                  carry the colour, and tinting the connectors too turns the
                  trace into a rainbow that competes with them. It read
                  gray-700, which is blue-tinted rather than neutral. */}
              {i > 0 && (
                <div aria-hidden className="flex justify-center text-ink-ghost">
                  <div className="flex flex-col items-center">
                    <div className="w-px h-3.5 bg-current" />
                    <ChevronDown size={10} strokeWidth={2} className="-mt-[3px]" />
                  </div>
                </div>
              )}
              <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] overflow-hidden">
                <button
                  onClick={() => setExpandedNodeId(isExpanded ? null : ns.nodeId)}
                  className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-white/[0.03] transition-colors"
                >
                  <StatusDot status={ns.status} />
                  <StepIcon node={node} look={look} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-gray-500 font-mono">#{i + 1}</span>
                      <span className="text-[12px] text-gray-300 truncate">
                        <NodeLabel nodeId={ns.nodeId} nodes={nodes} />
                      </span>
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
                    </div>
                    {meta && (
                      <div className="text-[11px] text-gray-600 font-mono truncate mt-0.5">
                        {meta}
                      </div>
                    )}
                  </div>
                  {ns.startedAt && ns.completedAt && (
                    <span className="text-[11px] text-gray-500 font-mono shrink-0">
                      {formatRunDuration(ns.startedAt, ns.completedAt)}
                    </span>
                  )}
                  <ChevronDown
                    size={12}
                    className={`text-gray-600 shrink-0 transition-transform ${
                      isExpanded ? '' : '-rotate-90'
                    }`}
                  />
                </button>

                {/* The last line the step produced, so a trace can be read
                    top-to-bottom. Hidden once expanded, where the full log
                    replaces it, and while a gate is waiting, where the
                    approval block is the thing to read. */}
                {preview && !isWaitingGate && expandedNodeId !== ns.nodeId && (
                  <button
                    onClick={() => setExpandedNodeId(ns.nodeId)}
                    aria-label={`Show full output of step ${i + 1}`}
                    className="w-full flex items-center gap-2 px-3 py-1.5 text-left border-t
                               border-white/[0.05] bg-black/20 hover:bg-black/30 transition-colors"
                  >
                    <p className="flex-1 min-w-0 text-[12px] text-gray-500 font-mono truncate">
                      {preview}
                    </p>
                    <ChevronDown size={11} className="text-gray-600 shrink-0" />
                  </button>
                )}

                {isWaitingGate && (
                  <div className="px-3 pb-3 -mt-0.5 flex items-start gap-2">
                    <div className="flex-1 min-w-0 text-[11px] text-bronzo">
                      {approvalMessage || 'Waiting for approval.'}
                    </div>
                    <button
                      onClick={() => {
                        void approveWorkflowGate(execution, ns.nodeId)
                      }}
                      className={`flex items-center gap-1 px-2 py-1 text-[11px] shrink-0 ${GATE_APPROVE}`}
                    >
                      <Check size={11} strokeWidth={2.5} />
                      Approve
                    </button>
                    <button
                      onClick={() => {
                        void rejectWorkflowGate(execution, ns.nodeId)
                      }}
                      className={`flex items-center gap-1 px-2 py-1 text-[11px] shrink-0 ${GATE_REJECT}`}
                    >
                      <X size={11} strokeWidth={2.5} />
                      Reject
                    </button>
                  </div>
                )}

                {/* One ordered account of the step: what the engine did to get
                    the agent running, what the agent said, then how it ended.
                    Engine lines are dimmed and marked so the agent's own words
                    stay the foreground, without splitting them across panels. */}
                {isExpanded && timeline.length > 0 && (
                  <div className="px-3 pb-2">
                    <div className="bg-black/30 rounded-md overflow-auto max-h-[280px]">
                      {timeline.map((entry, ti) =>
                        entry.kind === 'agent' ? (
                          <pre
                            key={ti}
                            className="text-[12px] text-gray-300 px-2 py-1.5
                                       font-mono whitespace-pre-wrap break-all leading-relaxed"
                          >
                            {entry.text}
                          </pre>
                        ) : (
                          <p
                            key={ti}
                            className="text-[12px] text-gray-500 font-mono px-2 py-0.5
                                       bg-white/[0.02] whitespace-pre-wrap break-all leading-relaxed"
                          >
                            {entry.text}
                          </p>
                        )
                      )}
                    </div>
                    <div className="flex items-center gap-1 mt-1.5">
                      {onViewFullOutput && ns.logs && (
                        <Tooltip label="View full output">
                          <button
                            onClick={() => onViewFullOutput(ns.logs!)}
                            aria-label="View full output"
                            className="p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
                          >
                            <Maximize2 size={12} strokeWidth={2} />
                          </button>
                        </Tooltip>
                      )}
                      {canResume && (
                        <Tooltip label="Resume session">
                          <button
                            onClick={handleResume}
                            aria-label="Resume session"
                            className="p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
                          >
                            <RotateCcw size={12} strokeWidth={2} />
                          </button>
                        </Tooltip>
                      )}
                    </div>
                    {ns.error && <p className="text-[11px] text-danger mt-1">{ns.error}</p>}
                  </div>
                )}

                {isExpanded && timeline.length === 0 && (
                  <div className="px-3 pb-2">
                    {ns.error ? (
                      <>
                        <p className="text-[11px] text-danger">{ns.error}</p>
                        {canResume && (
                          <div className="mt-1.5">
                            <Tooltip label="Resume session">
                              <button
                                onClick={handleResume}
                                aria-label="Resume session"
                                className="p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
                              >
                                <RotateCcw size={12} strokeWidth={2} />
                              </button>
                            </Tooltip>
                          </div>
                        )}
                      </>
                    ) : (
                      <p className="text-[11px] text-gray-600 italic">
                        {ns.status === 'running'
                          ? 'No output captured yet…'
                          : ns.status === 'pending'
                            ? "Step hasn't started yet."
                            : ns.status === 'skipped'
                              ? 'Step was skipped.'
                              : 'No output recorded.'}
                      </p>
                    )}
                  </div>
                )}
              </div>
            </div>
          )
        })}
      </div>
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
    <div className="border border-white/[0.08] rounded-md overflow-hidden">
      {/* Run header — the toggle and the stop control are siblings so the
          stop button isn't nested inside the row's button. */}
      <div className="flex items-center hover:bg-white/[0.04] transition-colors">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex-1 min-w-0 flex items-center gap-2 px-3 py-2.5 text-left"
        >
          {expanded ? (
            <ChevronDown size={12} className="text-gray-500" />
          ) : (
            <ChevronRight size={12} className="text-gray-500" />
          )}
          <StatusDot status={execution.status} />
          <span className="text-[12px] text-gray-300 flex-1 min-w-0 truncate">
            {workflowName && <span className="text-gray-500 mr-1.5">{workflowName}</span>}
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
            <span className="text-[9px] font-mono uppercase tracking-wider text-gray-500 border border-white/[0.08] rounded px-1 shrink-0">
              partial
            </span>
          )}
          <span className="text-[11px] text-gray-500 shrink-0">
            {formatRunDuration(execution.startedAt, execution.completedAt)}
          </span>
        </button>
        {execution.status === 'error' && onRetryRun && (
          <span className="shrink-0">
            <Tooltip label="Retry from failed step" position="top">
              <button
                aria-label="Retry from failed step"
                onClick={() => onRetryRun(execution)}
                className="p-1 rounded text-gray-500 hover:text-white transition-colors"
              >
                <RotateCcw size={12} strokeWidth={2} />
              </button>
            </Tooltip>
          </span>
        )}
        {execution.status !== 'running' && onRerunRun && (
          <span className="shrink-0">
            <Tooltip label="Run again" position="top">
              <button
                aria-label="Run again"
                onClick={() => onRerunRun(execution)}
                className="p-1 rounded text-gray-500 hover:text-white transition-colors"
              >
                <Play size={12} strokeWidth={2} />
              </button>
            </Tooltip>
          </span>
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
