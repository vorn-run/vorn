import { useState, useEffect } from 'react'
import { ChevronDown, ChevronRight, Maximize2, RotateCcw, Check, X } from 'lucide-react'
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
import { STATUS_DOT_CLASSES as SHARED_STATUS_DOTS } from './statusDot'
import { Tooltip } from '../Tooltip'
import { approveWorkflowGate, rejectWorkflowGate } from '../../lib/workflow-execution'
import { StopRunButton } from '../workflow-runs/StopRunButton'
import { ConnectorIcon } from '../ConnectorIcon'
import { useConnections } from '../../lib/use-connections'
import {
  NODE_TYPE_VISUAL,
  nodeConnectionId,
  stepMeta,
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
      className={`w-2 h-2 rounded-full shrink-0 ${SHARED_STATUS_DOTS[status] ?? 'bg-gray-600'}`}
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
  connectorId
}: {
  node: WorkflowNode | undefined
  connectorId: string | undefined
}) {
  if (connectorId) {
    return <ConnectorIcon connectorId={connectorId} size={12} className="text-gray-400 shrink-0" />
  }
  const visual = node ? NODE_TYPE_VISUAL[node.type] : undefined
  if (!visual) return null
  const Icon = visual.icon
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
  onResumeSession
}: RunStepsListProps) {
  const [expandedNodeId, setExpandedNodeId] = useState<string | null>(null)
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

          const connectionId = nodeConnectionId(node)
          const connectorId = connectionId
            ? connections.find((c) => c.id === connectionId)?.connectorId
            : undefined
          const meta = stepMeta(node, connectorId)
          // What the step said beats what it was told to do. A step with no
          // output yet (a trigger, a pending step) still shows its configured
          // body, so a card is never blank.
          const preview = stepOutputPreview(ns) ?? stepPreview(node)

          const isWaitingGate = ns.status === 'waiting' && node?.type === 'approval'
          const approvalMessage =
            node?.type === 'approval' ? (node.config as ApprovalConfig).message : undefined

          return (
            <div key={ns.nodeId}>
              {/* Line linking the previous step to this one so the cards read
                  as one continuous flow. Deliberately neutral: the status dots
                  carry the colour, and tinting the connectors too turns the
                  trace into a rainbow that competes with them. */}
              {i > 0 && (
                <div aria-hidden className="flex justify-center text-gray-700">
                  <div className="flex flex-col items-center">
                    <div className="w-px h-3.5 bg-current" />
                    <ChevronDown size={10} strokeWidth={2} className="-mt-[3px]" />
                  </div>
                </div>
              )}
              <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] overflow-hidden">
                <button
                  onClick={() => setExpandedNodeId(expandedNodeId === ns.nodeId ? null : ns.nodeId)}
                  className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-white/[0.03] transition-colors"
                >
                  <StatusDot status={ns.status} />
                  <StepIcon node={node} connectorId={connectorId} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-gray-500 font-mono">#{i + 1}</span>
                      <span className="text-[12px] text-gray-300 truncate">
                        <NodeLabel nodeId={ns.nodeId} nodes={nodes} />
                      </span>
                      {nodeTask && (
                        <span
                          className="text-[10px] px-1.5 py-0.5 bg-blue-500/10 border border-blue-500/20 rounded text-blue-400 truncate max-w-[80px] cursor-pointer hover:bg-blue-500/20 transition-colors"
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
                      expandedNodeId === ns.nodeId ? '' : '-rotate-90'
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
                    <p className="flex-1 min-w-0 text-[11px] text-gray-500 font-mono truncate">
                      {preview}
                    </p>
                    <ChevronDown size={11} className="text-gray-600 shrink-0" />
                  </button>
                )}

                {isWaitingGate && (
                  <div className="px-3 pb-3 -mt-0.5 flex items-start gap-2">
                    <div className="flex-1 min-w-0 text-[11px] text-amber-300/90">
                      {approvalMessage || 'Waiting for approval.'}
                    </div>
                    <button
                      onClick={() => {
                        void approveWorkflowGate(execution, ns.nodeId)
                      }}
                      className="flex items-center gap-1 px-2 py-1 rounded text-[11px]
                             text-green-300 hover:text-green-200 hover:bg-green-500/10
                             border border-green-500/30 transition-colors shrink-0"
                    >
                      <Check size={11} strokeWidth={2.5} />
                      Approve
                    </button>
                    <button
                      onClick={() => {
                        void rejectWorkflowGate(execution, ns.nodeId)
                      }}
                      className="flex items-center gap-1 px-2 py-1 rounded text-[11px]
                             text-red-300 hover:text-red-200 hover:bg-red-500/10
                             border border-red-500/30 transition-colors shrink-0"
                    >
                      <X size={11} strokeWidth={2.5} />
                      Reject
                    </button>
                  </div>
                )}

                {expandedNodeId === ns.nodeId && ns.logs && (
                  <div className="px-3 pb-2">
                    <pre
                      className="text-[11px] text-gray-400 bg-black/30 rounded-md p-2 max-h-[200px] overflow-auto
                                font-mono whitespace-pre-wrap break-all leading-relaxed"
                    >
                      {ns.logs.length > 2000 ? ns.logs.slice(0, 2000) + '\n...' : ns.logs}
                    </pre>
                    <div className="flex items-center gap-1 mt-1.5">
                      {onViewFullOutput && (
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
                    {ns.error && <p className="text-[11px] text-red-400 mt-1">{ns.error}</p>}
                  </div>
                )}

                {expandedNodeId === ns.nodeId && !ns.logs && ns.error && (
                  <div className="px-3 pb-2">
                    <p className="text-[11px] text-red-400">{ns.error}</p>
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
                  </div>
                )}

                {expandedNodeId === ns.nodeId && !ns.logs && !ns.error && !ns.diagnostics && (
                  <div className="px-3 pb-2">
                    <p className="text-[11px] text-gray-600 italic">
                      {ns.status === 'running'
                        ? 'No output captured yet…'
                        : ns.status === 'pending'
                          ? "Step hasn't started yet."
                          : ns.status === 'skipped'
                            ? 'Step was skipped.'
                            : 'No output recorded.'}
                    </p>
                  </div>
                )}

                {/* What the engine did, as distinct from what the agent said. This
                is the only thing left to read when a step produced no output,
                so it renders even — especially — when the log is empty. */}
                {expandedNodeId === ns.nodeId && ns.diagnostics && (
                  <div className="px-3 pb-2">
                    <p className="text-[10px] uppercase tracking-wider text-gray-600 mb-1">
                      Diagnostics
                    </p>
                    <pre
                      className="text-[11px] text-gray-500 bg-black/20 rounded p-2 max-h-[160px] overflow-auto
                             font-mono whitespace-pre-wrap break-all leading-relaxed"
                    >
                      {ns.diagnostics}
                    </pre>
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
  onResumeSession
}: RunEntryProps) {
  const hasWaitingGate = execution.nodeStates.some((ns) => ns.status === 'waiting')
  const [expanded, setExpanded] = useState(hasWaitingGate)

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (hasWaitingGate) setExpanded(true)
  }, [hasWaitingGate])

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
              className="text-[10px] px-1.5 py-0.5 bg-violet-500/10 border border-violet-500/20 rounded text-violet-400 truncate max-w-[100px] shrink-0 cursor-pointer hover:bg-violet-500/20 transition-colors"
              onClick={(e) => {
                e.stopPropagation()
                onClickTask?.(triggerTask.id)
              }}
              title={triggerTask.title}
            >
              {triggerTask.title}
            </span>
          )}
          <span className="text-[11px] text-gray-500 shrink-0">
            {formatRunDuration(execution.startedAt, execution.completedAt)}
          </span>
        </button>
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
        />
      )}
    </div>
  )
}
