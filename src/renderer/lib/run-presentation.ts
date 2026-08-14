import { Zap, Clock, CheckSquare, Play, type LucideIcon } from 'lucide-react'
import type {
  ApprovalConfig,
  NodeExecutionState,
  NodeExecutionStatus,
  TriggerConfig,
  WorkflowExecution,
  WorkflowNode
} from '../../shared/types'
import { WORKFLOW_STATUS_DOT, type WorkflowStatusKey, type RunOutcomeTone } from './workflow-status'
import type { RunBucket } from '../stores/types'

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

export type RunSource = 'manual' | 'schedule' | 'task' | 'connector'

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
  /** Short badge text next to the title (`manual`, `github`, `schedule`…). */
  sourceLabel: string
  /** The workflow's own icon key and colour, preferred over any fallback so a
   *  run is recognisable by the same mark the sidebar shows. */
  iconName?: string
  iconColor?: string
  /** Set for connector-triggered runs so the row can draw the brand glyph. */
  connectorId?: string
  /** Used only when the workflow is gone or never picked an icon. */
  fallbackIcon: LucideIcon
}

const SOURCE_ICONS: Record<RunSource, LucideIcon> = {
  manual: Zap,
  schedule: Clock,
  task: CheckSquare,
  connector: Play
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
function connectorTitle(execution: WorkflowExecution): string | undefined {
  const item = execution.connectorItem
  if (!item) return undefined
  const url = item.externalUrl
  if (url?.includes('/pull/')) return `PR #${item.externalId}`
  if (url?.includes('/issues/')) return `Issue #${item.externalId}`
  return item.externalId ? `${item.connectorId} ${item.externalId}` : undefined
}

export function describeRun(
  execution: WorkflowExecution,
  workflow?: RunWorkflowRef
): RunPresentation {
  const nodes = workflow?.nodes ?? []
  const triggerType = triggerTypeOf(nodes)
  const source = sourceOf(execution, triggerType)
  const item = execution.connectorItem
  const name = workflow?.name?.trim() || undefined
  const iconName = workflow?.icon
  const iconColor = workflow?.iconColor

  if (item) {
    const title = connectorTitle(execution) ?? item.title
    return {
      title,
      subtitle: item.title !== title ? item.title : name,
      source: 'connector',
      sourceLabel: item.connectorId,
      iconName,
      iconColor,
      connectorId: item.connectorId,
      fallbackIcon: SOURCE_ICONS.connector
    }
  }

  if (execution.triggerTaskId) {
    return {
      title: name ?? `Task ${execution.triggerTaskId.slice(0, 6)}`,
      subtitle: `Task ${execution.triggerTaskId.slice(0, 6)}`,
      source: 'task',
      sourceLabel: 'task',
      iconName,
      iconColor,
      fallbackIcon: SOURCE_ICONS.task
    }
  }

  return {
    title: name ?? execution.workflowId.slice(0, 8),
    subtitle: undefined,
    source,
    sourceLabel: source === 'schedule' ? 'scheduled' : 'manual',
    iconName,
    iconColor,
    fallbackIcon: SOURCE_ICONS[source]
  }
}

export interface RunStage {
  nodeId: string
  status: NodeExecutionState['status']
  label: string
  /** Tailwind background class for the segment / dot. */
  dotClass: string
}

/**
 * Every node state of a run, in definition order where the workflow is still
 * around, so the progress bar reads left-to-right as the graph does. Includes
 * the trigger — it is stage #1 of what actually happened.
 */
export function runStages(execution: WorkflowExecution, nodes: WorkflowNode[]): RunStage[] {
  const order = new Map(nodes.map((n, i) => [n.id, i]))
  const states = [...execution.nodeStates]
  if (order.size > 0) {
    states.sort((a, b) => (order.get(a.nodeId) ?? 999) - (order.get(b.nodeId) ?? 999))
  }
  return states.map((ns) => {
    const node = nodes.find((n) => n.id === ns.nodeId)
    return {
      nodeId: ns.nodeId,
      status: ns.status,
      label: node?.label || ns.nodeId.slice(0, 8),
      dotClass: WORKFLOW_STATUS_DOT[ns.status as WorkflowStatusKey] ?? WORKFLOW_STATUS_DOT.pending
    }
  })
}

const TERMINAL_STAGE_STATUSES = new Set<NodeExecutionState['status']>([
  'success',
  'error',
  'skipped'
])

export function completedStageCount(stages: RunStage[]): number {
  return stages.filter((s) => TERMINAL_STAGE_STATUSES.has(s.status)).length
}

/**
 * Short fields an agent step may emit as its verdict. A typed step with an
 * `outputSchema` is the only place a run carries a human-meaningful conclusion,
 * so the row label prefers it over a generic status word.
 */
const VERDICT_KEYS = ['verdict', 'recommendation', 'decision', 'summary', 'result', 'status']
const MAX_VERDICT_LENGTH = 40

function verdictOf(execution: WorkflowExecution): string | undefined {
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

export type { RunOutcomeTone } from './workflow-status'
export { outcomeToneClass } from './workflow-status'

export interface RunOutcome {
  label: string
  tone: RunOutcomeTone
}

/**
 * The one line that says how a run ended. A paused gate outranks everything —
 * it is the only state that needs the user — and a finished run prefers the
 * agent's own verdict over a generic "completed".
 */
export function describeOutcome(execution: WorkflowExecution, nodes: WorkflowNode[]): RunOutcome {
  const gate = execution.nodeStates.find((ns) => ns.status === 'waiting')
  if (gate) {
    const node = nodes.find((n) => n.id === gate.nodeId)
    const message = node?.type === 'approval' ? (node.config as ApprovalConfig).message : undefined
    return {
      label: message?.trim() || 'needs review',
      tone: 'waiting'
    }
  }
  if (execution.status === 'running') return { label: 'in progress', tone: 'running' }
  if (execution.status === 'error') return { label: 'run failed', tone: 'error' }
  if (execution.status === 'cancelled') return { label: 'stopped', tone: 'neutral' }
  return { label: verdictOf(execution) ?? 'completed', tone: 'success' }
}

/** True when no step ever paused for a human. */
export function ranUninterrupted(execution: WorkflowExecution): boolean {
  return !execution.nodeStates.some((ns) => ns.approvedAt || ns.status === 'waiting')
}

const MAX_LOG_TAIL = 600

/**
 * Live-ish summary for the detail card. There is no stored run summary, so the
 * most informative thing available is the tail of whatever step is currently
 * talking — falling back to the last step that produced anything.
 */
export function runSummaryText(execution: WorkflowExecution): string | undefined {
  const running = execution.nodeStates.find((ns) => ns.status === 'running' && ns.logs)
  const source =
    running ?? [...execution.nodeStates].reverse().find((ns) => ns.logs?.trim() || ns.error)
  const text = source?.logs?.trim() || source?.error?.trim()
  if (!text) return undefined
  return text.length > MAX_LOG_TAIL ? `…${text.slice(-MAX_LOG_TAIL)}` : text
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
const LIVE_STATUS_RANK: Record<string, number> = {
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
  for (const exec of executions) {
    if (exec.workflowId !== workflowId) continue
    for (const ns of exec.nodeStates ?? []) {
      const rank = LIVE_STATUS_RANK[ns.status] ?? 0
      if (rank === 0) continue
      const held = worst[ns.nodeId]
      if (!held || rank > (LIVE_STATUS_RANK[held] ?? 0)) worst[ns.nodeId] = ns.status
    }
  }
  return Object.keys(worst).length > 0 ? worst : undefined
}
