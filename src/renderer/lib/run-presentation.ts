import { Zap, Clock, CheckSquare, Play, type LucideIcon, RotateCcw } from 'lucide-react'
import { failedStep, isSignInWait } from '@vornrun/shared/workflow-graph'
import type {
  NodeExecutionStatus,
  SdkConnectorIcon,
  TriggerConfig,
  WorkflowExecution,
  WorkflowNode
} from '../../shared/types'
import type { ConnectorLook } from './use-connections'
import type { RunBucket } from '../stores/types'
import { formatRunDuration } from './format-time'

/** What the editor toasts when a run it launched reaches a terminal state. */
export function runCompletionToast(
  execution: WorkflowExecution,
  nodes: WorkflowNode[]
): { kind: 'success' | 'error' | 'quiet'; message: string; failedNodeId?: string } {
  if (execution.status === 'success') {
    const steps = stepProgress(execution, nodes).done
    const duration = formatRunDuration(execution.startedAt, execution.completedAt)
    return {
      kind: 'success',
      message: `Run finished — ${steps} step${steps === 1 ? '' : 's'} in ${duration}`
    }
  }
  if (execution.status === 'error') {
    const failed = failedStep(execution)
    const failedNode = failed ? nodes.find((n) => n.id === failed.nodeId) : undefined
    return {
      kind: 'error',
      message: failedNode ? `Run failed at "${failedNode.label}"` : 'Run failed',
      failedNodeId: failedNode?.id
    }
  }
  return { kind: 'quiet', message: '' }
}

/**
 * Which filter bucket a run belongs to. A paused run is `waiting` rather than
 * `running` — it is not making progress, and that is the whole point of the
 * Needs review filter.
 */
export function bucketOf(execution: WorkflowExecution): RunBucket {
  if (execution.status === 'running') {
    return execution.nodeStates.some((n) => n.status === 'waiting') ? 'waiting' : 'running'
  }
  return execution.status === 'success' ? 'success' : 'error'
}

/** A step's name, or a short id once the workflow no longer has it. */
export function nodeLabel(node: WorkflowNode | undefined, nodeId: string): string {
  return node?.label || nodeId.slice(0, 8)
}

/** A run's state in words, naming the step it broke at, waits at or is working on. */
export function runStatusLine(execution: WorkflowExecution, nodes: WorkflowNode[]): string {
  const named = (nodeId: string): string =>
    nodeLabel(
      nodes.find((n) => n.id === nodeId),
      nodeId
    )
  const waiting = execution.nodeStates.find((ns) => ns.status === 'waiting')
  if (waiting) {
    return `${isSignInWait(waiting) ? 'Waiting for sign-in' : 'Waiting'} at ${named(waiting.nodeId)}`
  }
  if (execution.status === 'running') {
    const active = execution.nodeStates.find((ns) => ns.status === 'running')
    return active ? `Running ${named(active.nodeId)}` : 'Running'
  }
  if (execution.status === 'error') {
    const failed = failedStep(execution)
    return failed ? `Failed at ${named(failed.nodeId)}` : 'Failed'
  }
  return execution.status === 'cancelled' ? 'Stopped' : 'Completed'
}

/** How far a run got: the steps that succeeded out of all it reached, the trigger left out. */
export function stepProgress(
  execution: WorkflowExecution,
  nodes: WorkflowNode[]
): { done: number; total: number } {
  const triggers = new Set(nodes.filter((n) => n.type === 'trigger').map((n) => n.id))
  const steps = execution.nodeStates.filter((ns) => !triggers.has(ns.nodeId))
  return { done: steps.filter((ns) => ns.status === 'success').length, total: steps.length }
}

export type RunSource = 'manual' | 'schedule' | 'task' | 'connector' | 'restore'

/** The parts of a workflow definition a run row needs to render itself. */
export interface RunWorkflowRef {
  name?: string
  /** Key into the shared `ICON_MAP` — the workflow's own chosen glyph. */
  icon?: string
  iconColor?: string
  nodes: WorkflowNode[]
}

export interface RunPresentation {
  /** Headline for the run — what it acted on, not which workflow ran it. */
  title: string
  /** One-line description of the subject, or the workflow name as a fallback. */
  subtitle?: string
  source: RunSource
  /** Where the run came from, in a word (`manual`, `github`, `scheduled`…). */
  sourceLabel: string
  /** The workflow's own icon and colour, so a run carries the mark the sidebar shows. */
  iconName?: string
  iconColor?: string
  /** Set for connector-triggered runs so the row can draw the brand glyph. */
  connectorId?: string
  /** A packaged connector's own glyph, which the built-in lookup cannot supply. */
  connectorIcon?: SdkConnectorIcon
  /** From a packaged connector, so a missing glyph falls back to the plug. */
  connectorPackaged?: boolean
  /** Used only when the workflow is gone or never picked an icon. */
  fallbackIcon: LucideIcon
}

const SOURCE_ICONS: Record<RunSource, LucideIcon> = {
  manual: Zap,
  schedule: Clock,
  task: CheckSquare,
  connector: Play,
  restore: RotateCcw
}

function triggerNodeOf(nodes: WorkflowNode[]): WorkflowNode | undefined {
  return nodes.find((n) => n.type === 'trigger')
}

function triggerTypeOf(nodes: WorkflowNode[]): TriggerConfig['triggerType'] | undefined {
  const trigger = triggerNodeOf(nodes)
  return trigger ? (trigger.config as TriggerConfig).triggerType : undefined
}

function sourceOf(
  execution: WorkflowExecution,
  triggerType: TriggerConfig['triggerType'] | undefined
): RunSource {
  if (execution.connectorItem) return 'connector'
  if (execution.triggerTaskId) return 'task'
  if (triggerType === 'once' || triggerType === 'recurring') return 'schedule'
  if (triggerType === 'connectorPoll') return 'connector'
  if (triggerType === 'taskCreated' || triggerType === 'taskStatusChanged') return 'task'
  if (execution.triggerSession || triggerType === 'sessionRestored') return 'restore'
  return 'manual'
}

/**
 * GitHub numbers issues and pull requests in one sequence, and the connector
 * flattens both into the same item shape — the item URL is the only thing that
 * survives the poll to tell them apart (`/pull/<n>` vs `/issues/<n>`).
 *
 * Keyed on that URL and not on the connector id, because a packaged connector
 * does not have its own id here: those connections are stored as `mcp` with
 * the real id in `filters.sdkConnectorId`, so an `id === 'github'` test would
 * quietly stop matching the day GitHub ships as a package and every run would
 * read `mcp 123`.
 */
function connectorTitle(execution: WorkflowExecution, connectorId: string): string | undefined {
  const item = execution.connectorItem
  if (!item) return undefined
  const url = item.externalUrl
  if (url?.includes('/pull/')) return `PR #${item.externalId}`
  if (url?.includes('/issues/')) return `Issue #${item.externalId}`
  return item.externalId ? `${connectorId} ${item.externalId}` : undefined
}

export function describeRun(
  execution: WorkflowExecution,
  workflow?: RunWorkflowRef,
  /** Resolved from the run's connection, because the item only knows it as `mcp`. */
  look?: ConnectorLook
): RunPresentation {
  const nodes = workflow?.nodes ?? []
  const source = sourceOf(execution, triggerTypeOf(nodes))
  const item = execution.connectorItem
  const name = workflow?.name?.trim() || undefined
  const mark = { iconName: workflow?.icon, iconColor: workflow?.iconColor }

  if (item) {
    const connectorId = look?.connectorId ?? item.connectorId
    const title = connectorTitle(execution, connectorId) ?? item.title
    return {
      title,
      subtitle: item.title !== title ? item.title : name,
      source: 'connector',
      sourceLabel: connectorId,
      ...mark,
      connectorId,
      connectorIcon: look?.icon,
      connectorPackaged: look?.packaged,
      fallbackIcon: SOURCE_ICONS.connector
    }
  }

  if (execution.triggerSession) {
    const { label, restore } = execution.triggerSession
    return {
      title: name ?? label,
      subtitle: `restore · ${restore} · ${label}`,
      source: 'restore',
      sourceLabel: 'restore',
      ...mark,
      fallbackIcon: SOURCE_ICONS.restore
    }
  }

  if (execution.triggerTaskId) {
    return {
      title: name ?? `Task ${execution.triggerTaskId.slice(0, 6)}`,
      subtitle: `Task ${execution.triggerTaskId.slice(0, 6)}`,
      source: 'task',
      sourceLabel: 'task',
      ...mark,
      fallbackIcon: SOURCE_ICONS.task
    }
  }

  return {
    title: name ?? execution.workflowId.slice(0, 8),
    subtitle: undefined,
    source,
    sourceLabel: source === 'schedule' ? 'scheduled' : source === 'restore' ? 'restore' : 'manual',
    ...mark,
    fallbackIcon: SOURCE_ICONS[source]
  }
}

/**
 * Short fields an agent step may emit as its verdict. A typed step with an
 * `outputSchema` is the only place a run carries a human-meaningful conclusion,
 * so a finished run shows it beside its state.
 */
const VERDICT_KEYS = ['verdict', 'recommendation', 'decision', 'summary', 'result', 'status']
const MAX_VERDICT_LENGTH = 40

/** The conclusion the run's last typed step wrote, when it is short enough to be one. */
export function runVerdict(execution: WorkflowExecution): string | undefined {
  for (let i = execution.nodeStates.length - 1; i >= 0; i--) {
    const out = execution.nodeStates[i].structuredOutput
    if (!out) continue
    for (const key of VERDICT_KEYS) {
      const value = out[key]
      if (typeof value === 'string' && value.trim() && value.length <= MAX_VERDICT_LENGTH) {
        return value.trim()
      }
    }
  }
  return undefined
}

/**
 * What each node is doing across every run of a workflow that is live now.
 *
 * A node can be in several states at once when runs go in parallel, so the most
 * urgent wins: a gate waiting on the person outranks work still going, which
 * outranks a failure worth reading, which outranks a step that finished.
 *
 * Returns undefined when nothing is running, so a canvas showing a definition
 * rather than a run renders no status at all.
 */
const LIVE_STATUS_RANK: Partial<Record<NodeExecutionStatus, number>> = {
  waiting: 4,
  running: 3,
  error: 2,
  success: 1
}

export function liveNodeStatus(
  executions: Iterable<WorkflowExecution>,
  workflowId: string
): Record<string, NodeExecutionStatus> | undefined {
  const worst: Record<string, NodeExecutionStatus> = {}
  const ranks: Record<string, number> = {}
  let found = false
  for (const exec of executions) {
    // Finished runs stay in the store to back the history list, so without this
    // the canvas would keep every dot from the last run that ever completed and
    // could never go back to showing a plain definition.
    if (exec.status !== 'running' || exec.workflowId !== workflowId) continue
    for (const ns of exec.nodeStates ?? []) {
      const rank = LIVE_STATUS_RANK[ns.status]
      if (!rank || rank <= (ranks[ns.nodeId] ?? 0)) continue
      worst[ns.nodeId] = ns.status
      ranks[ns.nodeId] = rank
      found = true
    }
  }
  return found ? worst : undefined
}
