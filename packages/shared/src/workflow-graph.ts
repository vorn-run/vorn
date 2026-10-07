import {
  ApprovalConfig,
  GateFeedbackConfig,
  ConnectorItemContext,
  LaunchAgentConfig,
  LoopConfig,
  NodeExecutionState,
  WorkflowExecution,
  WorkflowEdge,
  WorkflowExecutionContext,
  WorkflowNode
} from './types'
import type { StepOutputs } from './template-vars'
import { isRecordList, toItemList } from './item-list'

/**
 * What the editor and the run views read of a workflow's graph and runs.
 *
 * Graph shape, loop bodies, gate rounds and what a run's steps produced. The
 * engine that walks runs is vornd's (`vorn-work` and `vorn-workflow`), which
 * decides the same things the same way; these are the answers the clients draw.
 */

/** Default ceiling for a headless step that never reports an exit. */
export const DEFAULT_STEP_TIMEOUT_MINUTES = 60

export function buildStepOutputsMap(
  execution: WorkflowExecution,
  nodeMap: Map<string, WorkflowNode>
): StepOutputs {
  const outputs: StepOutputs = {}
  for (const ns of execution.nodeStates) {
    const node = nodeMap.get(ns.nodeId)
    if (!node?.slug) continue
    const settled = ns.status === 'success' || ns.status === 'error'
    // A gate that asked is readable before it settles: the steps it sent back read its comment.
    const gate = node.type === 'approval' ? gateOutputs(ns) : undefined
    if (!settled && !gate) continue

    // Schema-typed connector outputs come first so a declared key like
    // `html_url` wins over the generic fallback — but the defaults
    // (output/status/error) always overlay so control-flow references keep
    // working regardless of whether the connector returned a typed payload.
    outputs[node.slug] = {
      ...(settled && {
        ...(ns.structuredOutput ?? {}),
        output: ns.output || ns.logs || '',
        status: ns.status,
        error: ns.error || '',
        // The directory the step worked in, so a later step can run there.
        worktreePath: ns.worktreePath ?? ''
      }),
      ...gate
    }
  }
  return outputs
}

/**
 * A gate's text, its latest comment, every comment, the comments pinned to
 * its review page in the latest round, and which time it asked;
 * undefined before it first asks. When the text is a list of records — what a
 * review gate draws as a table — `items` is that list as data, the rows the
 * reviewer kept, ready for a for-each loop.
 */
function gateOutputs(state: NodeExecutionState): Record<string, unknown> | undefined {
  if (state.round === undefined && !state.feedback?.length) return undefined
  const entries = state.feedback ?? []
  const text = state.editedText ?? state.editableText ?? ''
  const list = toItemList(text)
  return {
    ...('items' in list && isRecordList(list.items) && { items: list.items }),
    text,
    feedback: entries.length > 0 ? entries[entries.length - 1].comment : '',
    feedbackAll: entries.map((e) => `Round ${e.round}: ${e.comment}`).join('\n'),
    comments: (entries.at(-1)?.comments ?? []).map((c) => ({
      quote: c.quote ?? '',
      comment: c.comment,
      anchored: Boolean(c.quote)
    })),
    round: state.round ?? 1
  }
}

/** Ceiling on how many times a gate may ask, whatever a workflow says. */
export const MAX_GATE_ROUNDS = 10

export const DEFAULT_GATE_ROUNDS = 3

export function gateMaxRounds(feedback: Pick<GateFeedbackConfig, 'maxRounds'> | undefined): number {
  const rounds = Math.floor(Number(feedback?.maxRounds))
  return Number.isFinite(rounds)
    ? Math.min(Math.max(1, rounds), MAX_GATE_ROUNDS)
    : DEFAULT_GATE_ROUNDS
}

/** Whether the reviewer may still send the work back at the round the gate is on. */
export function canRequestChanges(
  config: ApprovalConfig | undefined,
  state: Pick<NodeExecutionState, 'round'> | undefined
): boolean {
  if (!config?.feedback?.from) return false
  return (state?.round ?? 1) < gateMaxRounds(config.feedback)
}

/** Steps on some path from `from` to `to`, both included; empty when `to` is not below `from`. */
export function nodesBetween(
  from: string,
  to: string,
  edges: readonly { source: string; target: string }[]
): Set<string> {
  if (from === to) return new Set()
  const { successors, predecessors } = buildGraph(edges)
  const reach = (start: string, next: Map<string, string[]>): Set<string> => {
    const seen = new Set([start])
    const queue = [start]
    while (queue.length > 0) {
      for (const id of next.get(queue.shift()!) ?? []) {
        if (!seen.has(id)) {
          seen.add(id)
          queue.push(id)
        }
      }
    }
    return seen
  }
  const below = reach(from, successors)
  if (!below.has(to)) return new Set()
  const above = reach(to, predecessors)
  return new Set([...below].filter((id) => above.has(id)))
}

/** How much of each step's output a loop keeps per pass, in {{steps.<loop>.results}}. */
export const LOOP_RESULT_OUTPUT_CHARS = 8000

/** Ceiling on `maxIterations`, whatever a workflow asks for. */
export const MAX_LOOP_ITERATIONS = 10

/**
 * Whether a node's failure ends the run.
 *
 * Absent means stop — see WorkflowNodeErrorPolicy for why that is the default
 * rather than the continue-everything behaviour this replaced.
 */
export function stopsRunOnError(node: Partial<Pick<WorkflowNode, 'onError'>>): boolean {
  return (node.onError ?? 'stop') === 'stop'
}

/** A step the engine wrote off rather than ran: the reconciler never saw it start. */
/** A step parked until its connection signs in again, not on an approval. */
export function isSignInWait(state: { status: string; waitingFor?: string }): boolean {
  return state.status === 'waiting' && state.waitingFor === 'signIn'
}

export const ABANDONED = 'Run abandoned (no session id recorded)'

/** A step that never ran, so no policy of its own has anything to say about it. */
export function neverRan(state: NodeExecutionState): boolean {
  return state.error?.startsWith('Skipped:') === true || state.error === ABANDONED
}

/** The step that failed on its own terms, which is what a retry has somewhere to start from. */
export function failedStep(execution: WorkflowExecution): NodeExecutionState | undefined {
  return execution.nodeStates.find((ns) => ns.status === 'error' && !neverRan(ns))
}

export function hasFailedStep(execution: WorkflowExecution): boolean {
  return failedStep(execution) !== undefined
}

export function buildGraph(edges: readonly { source: string; target: string }[]): {
  successors: Map<string, string[]>
  predecessors: Map<string, string[]>
} {
  const successors = new Map<string, string[]>()
  const predecessors = new Map<string, string[]>()
  for (const e of edges) {
    successors.set(e.source, [...(successors.get(e.source) || []), e.target])
    predecessors.set(e.target, [...(predecessors.get(e.target) || []), e.source])
  }
  return { successors, predecessors }
}

/**
 * Which loop owns each body step.
 *
 * Ids a loop lists that no longer exist are left out, so a stale
 * `bodyNodeIds` entry (a step deleted in an older build) is not a member of
 * anything. The first loop to claim a step keeps it.
 */
export function loopBodyOwners(nodes: readonly WorkflowNode[]): Map<string, string> {
  const exists = new Set(nodes.map((n) => n.id))
  const owners = new Map<string, string>()
  for (const n of nodes) {
    if (n.type !== 'loop') continue
    for (const id of (n.config as LoopConfig).bodyNodeIds ?? []) {
      if (exists.has(id) && id !== n.id && !owners.has(id)) owners.set(id, n.id)
    }
  }
  return owners
}

/**
 * The run graph as the main scheduler sees it: each loop stands in for its body.
 *
 * A loop drives its body itself, so the scheduler must never reach a body step
 * on its own — otherwise a step left pending by a failed pass becomes ready the
 * moment its predecessor settles, and runs outside the loop. Edges into and
 * inside a body are dropped; an edge leaving a body is redrawn from the loop,
 * which is also how the canvas draws it. A branch label on such an edge is
 * dropped, since the loop, not the condition inside it, is what the run waits on.
 */
export function collapseLoopBodies(
  nodes: readonly WorkflowNode[],
  edges: readonly WorkflowEdge[]
): WorkflowEdge[] {
  const owners = loopBodyOwners(nodes)
  const seen = new Set<string>()
  const collapsed: WorkflowEdge[] = []
  for (const edge of edges) {
    if (owners.has(edge.target)) continue
    const owner = owners.get(edge.source)
    const next: WorkflowEdge = owner
      ? { id: `${edge.id}:via-loop`, source: owner, target: edge.target }
      : edge
    if (next.source === next.target) continue
    const key = `${next.source}->${next.target}:${next.conditionBranch ?? ''}`
    if (seen.has(key)) continue
    seen.add(key)
    collapsed.push(next)
  }
  return collapsed
}

/**
 * A loop's body as a graph of its own: members, the edges between them, and
 * the members nothing inside the body feeds (where each pass starts).
 *
 * A body without edges between its members is chained in `bodyNodeIds` order,
 * which is how loops ran before bodies could branch.
 */
export function loopBodyGraph(
  nodes: readonly WorkflowNode[],
  edges: readonly WorkflowEdge[],
  loop: WorkflowNode
): { members: WorkflowNode[]; edges: WorkflowEdge[]; entries: string[] } {
  const owners = loopBodyOwners(nodes)
  const byId = new Map(nodes.map((n) => [n.id, n]))
  const members = ((loop.config as LoopConfig).bodyNodeIds ?? [])
    .filter((id) => owners.get(id) === loop.id)
    .map((id) => byId.get(id)!)
  const ids = new Set(members.map((m) => m.id))
  let inner = edges.filter((e) => ids.has(e.source) && ids.has(e.target))
  if (inner.length === 0 && members.length > 1) {
    inner = members.slice(1).map((m, i) => ({
      id: `${members[i].id}->${m.id}:chain`,
      source: members[i].id,
      target: m.id
    }))
  }
  const fed = new Set(inner.map((e) => e.target))
  return { members, edges: inner, entries: members.filter((m) => !fed.has(m.id)).map((m) => m.id) }
}

/**
 * Why a loop's body cannot run, or undefined when it can.
 *
 * Checked by the engine before the first pass, and by the editor and the MCP
 * tools before a workflow is saved, so a shape the engine refuses is never
 * stored in the first place.
 */
export function loopStructureError(
  nodes: readonly WorkflowNode[],
  edges: readonly WorkflowEdge[],
  loop: WorkflowNode
): string | undefined {
  const { members, edges: inner } = loopBodyGraph(nodes, edges, loop)
  if (members.length === 0) return 'Loop has no body steps. Add at least one step for it to repeat.'
  const ids = new Set(members.map((m) => m.id))
  for (const m of members) {
    // A gate inside a loop would park the run mid-pass, and resuming means
    // re-entering the loop at the pass it stopped on: state a loop does not keep.
    if (m.type === 'approval') {
      return `Loop body contains an approval gate ("${m.label}"), which is not supported.`
    }
    if (m.type === 'loop')
      return `Loop body contains another loop ("${m.label}"), which is not supported.`
    if (m.type === 'trigger') return `Loop body contains a trigger ("${m.label}").`
  }
  for (const e of edges) {
    if (ids.has(e.target) && !ids.has(e.source) && e.source !== loop.id) {
      const from = nodes.find((n) => n.id === e.source)?.label ?? e.source
      return `"${from}" feeds a step inside the loop from outside it. Only the loop starts its steps.`
    }
    if (ids.has(e.source) && !ids.has(e.target) && e.conditionBranch) {
      return 'A condition inside the loop branches to a step outside it. Keep both branches inside the loop.'
    }
  }
  // Kahn's algorithm: anything left over sits on a cycle.
  const indegree = new Map(members.map((m) => [m.id, 0]))
  for (const e of inner) indegree.set(e.target, (indegree.get(e.target) ?? 0) + 1)
  const queue = [...indegree].filter(([, d]) => d === 0).map(([id]) => id)
  let visited = 0
  while (queue.length > 0) {
    const id = queue.shift()!
    visited++
    for (const e of inner) {
      if (e.source !== id) continue
      const d = (indegree.get(e.target) ?? 0) - 1
      indegree.set(e.target, d)
      if (d === 0) queue.push(e.target)
    }
  }
  if (visited < members.length) return 'The steps inside the loop form a cycle.'
  return undefined
}

/** Whether a run can still be stopped — drives the Stop control's visibility. */
export function isRunStoppable(execution: WorkflowExecution): boolean {
  return execution.status === 'running'
}

export type WorktreeMode = 'none' | 'new' | 'fromStep' | 'existing' | 'fromContext'

export function getWorktreeMode(cfg: LaunchAgentConfig): WorktreeMode {
  if (cfg.useWorktree === 'fromContext') return 'fromContext'
  return cfg.worktreeMode ?? (cfg.useWorktree === true ? 'new' : 'none')
}

/** The {{trigger.*}} namespace of a webhook run, rebuilt from the event's stored payload. */
export function webhookTriggerFromItem(
  connectorItem: ConnectorItemContext | undefined
): WorkflowExecutionContext['trigger'] | undefined {
  if (connectorItem?.connectorId !== 'webhook') return undefined
  const raw = connectorItem.raw as {
    body?: unknown
    headers?: Record<string, string>
    query?: Record<string, string>
    method?: string
  }
  return {
    type: 'webhook' as const,
    body: raw.body,
    headers: raw.headers,
    query: raw.query,
    method: raw.method
  }
}

/**
 * The run context for a scheduler-delivered event. A webhook event rides the
 * connector pipe for durability, so its payload is lifted into the trigger
 * namespace here while the connectorItem keeps the lease machinery working.
 */
export function schedulerExecutionContext(
  connectorItem: ConnectorItemContext | undefined,
  inputs: Record<string, unknown> | undefined
): WorkflowExecutionContext | undefined {
  if (!connectorItem && !inputs) return undefined
  const trigger = webhookTriggerFromItem(connectorItem)
  return { connectorItem, inputs, ...(trigger && { trigger }) }
}
