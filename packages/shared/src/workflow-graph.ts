import {
  ConditionConfig,
  ConnectorItemContext,
  LaunchAgentConfig,
  ConditionOperator,
  LoopConfig,
  NodeExecutionState,
  ScriptConfig,
  WorkflowDefinition,
  WorkflowExecution,
  WorkflowExecutionContext,
  WorkflowNode
} from './types'
import { resolveTemplateVars, type StepOutputs } from './template-vars'

/**
 * The parts of workflow execution that decide rather than do.
 *
 * Graph shape, retry seeding, loop exit, condition evaluation, fingerprints:
 * none of it opens a socket, spawns a process or reads a store, and all of it
 * is needed on both sides of the wire -- the engine walks a run with it, and
 * the editor reads the same answers to draw one. It moved here from the
 * renderer when execution moved to the server, so that the two cannot drift.
 */

/** Default ceiling for a headless step that never reports an exit. */
export const DEFAULT_STEP_TIMEOUT_MINUTES = 60

const LOG_BUFFER_MAX = 100_000

const LOG_BUFFER_KEEP = 80_000

/** Cap renderer-resident log buffers so a chatty agent can't exhaust memory. */
export function appendBoundedLog(buffer: string, chunk: string): string {
  const next = buffer + chunk
  return next.length > LOG_BUFFER_MAX ? next.slice(-LOG_BUFFER_KEEP) : next
}

/** Tag worktree provenance for cleanup. `undefined` when no worktree is in
 *  play; `'inherited'` when the contextual source supplied one (don't delete);
 *  `'created'` when this node spun one up itself. */
export function resolveWorktreeOrigin(
  worktreePath: string | undefined,
  inherited: boolean
): 'created' | 'inherited' | undefined {
  if (!worktreePath) return undefined
  return inherited ? 'inherited' : 'created'
}

/** Scoped to the run, not the workflow — parallel runs each have their own gate. */
export function gateKey(runId: string, nodeId: string): string {
  return `${runId}:${nodeId}`
}

/**
 * What this run was triggered *with*. Two runs of one workflow count as the
 * same trigger only when this matches, which is what lets a connector fan-out
 * run its items in parallel while a genuine double-fire collapses to one run.
 */
export function dedupeFingerprint(context?: WorkflowExecutionContext): string {
  // Two runs started with different parameters are different triggers, so the
  // inputs qualify every fingerprint rather than only the context-less one —
  // a workflow launched twice from the same card with different answers is
  // still two distinct runs.
  const inputs = fingerprintInputs(context?.inputs)
  const params = inputs ? `:inputs:${inputs}` : ''
  const item = context?.connectorItem
  if (item) return `item:${item.connectionId}:${item.externalId}${params}`
  if (context?.task) return `task:${context.task.id}${params}`
  if (context?.source) return `session:${context.source.id}${params}`
  return `manual${params}`
}

/** Stable serialization of run inputs — key-sorted so object ordering can't
 *  make two identical parameter sets look like different triggers. */
function fingerprintInputs(inputs: Record<string, unknown> | undefined): string {
  if (!inputs) return ''
  const keys = Object.keys(inputs).sort()
  if (keys.length === 0) return ''
  // No try/catch: values are already required to be JSON-serializable, since
  // saveWorkflowRun persists them the same way. Falling back to a key-only
  // digest would silently collapse two different parameter sets into one
  // fingerprint — exactly the dedupe bug this function exists to prevent.
  return JSON.stringify(keys.map((k) => [k, inputs[k]]))
}

/**
 * Records what the engine did on a step's behalf, so a step that produced no
 * output still accounts for itself. Times are relative to the step starting,
 * because "the agent was spawned but had written nothing 60 minutes later" is
 * the shape of the answer, not the wall-clock time it happened at.
 */
export class StepDiagnostics {
  private readonly lines: string[] = []
  private readonly startedAt = Date.now()

  note(message: string): void {
    const seconds = ((Date.now() - this.startedAt) / 1000).toFixed(1)
    this.lines.push(`[+${seconds}s] ${message}`)
  }

  toString(): string {
    return this.lines.join('\n')
  }
}

export function updateNodeState(
  execution: WorkflowExecution,
  nodeId: string,
  updates: Partial<NodeExecutionState>
): void {
  const state = execution.nodeStates.find((s) => s.nodeId === nodeId)
  if (state) {
    Object.assign(state, updates)
  }
}

// A script step with its templates filled in, directory included.
export function resolveScriptConfig(
  config: ScriptConfig,
  context?: WorkflowExecutionContext,
  stepOutputs?: StepOutputs
): ScriptConfig {
  const path = (value: string | undefined): string | undefined =>
    value === undefined ? undefined : resolveTemplateVars(value, context, stepOutputs) || undefined
  return {
    ...config,
    scriptContent: resolveTemplateVars(config.scriptContent, context, stepOutputs),
    cwd: path(config.cwd),
    projectPath: path(config.projectPath),
    // Untrusted text reaches the script as an argument, never spliced into its source.
    ...(config.args && {
      args: config.args.map((arg) => resolveTemplateVars(arg, context, stepOutputs))
    })
  }
}

export function buildStepOutputsMap(
  execution: WorkflowExecution,
  nodeMap: Map<string, WorkflowNode>
): StepOutputs {
  const outputs: StepOutputs = {}
  for (const ns of execution.nodeStates) {
    if (ns.status !== 'success' && ns.status !== 'error') continue
    const node = nodeMap.get(ns.nodeId)
    if (!node?.slug) continue

    // Schema-typed connector outputs come first so a declared key like
    // `html_url` wins over the generic fallback — but the defaults
    // (output/status/error) always overlay so control-flow references keep
    // working regardless of whether the connector returned a typed payload.
    outputs[node.slug] = {
      ...(ns.structuredOutput ?? {}),
      output: ns.output || ns.logs || '',
      status: ns.status,
      error: ns.error || '',
      // The directory the step worked in, so a later step can run there.
      worktreePath: ns.worktreePath ?? ''
    }
  }
  return outputs
}

export function evaluateCondition(
  operator: ConditionOperator,
  resolved: string,
  value: string
): boolean {
  switch (operator) {
    case 'equals':
      return resolved === value
    case 'notEquals':
      return resolved !== value
    case 'contains':
      return resolved.includes(value)
    case 'notContains':
      return !resolved.includes(value)
    case 'isEmpty':
      return resolved.trim() === ''
    case 'isNotEmpty':
      return resolved.trim() !== ''
    default:
      return false
  }
}

/** Ceiling on `maxIterations`, whatever a workflow asks for. */
export const MAX_LOOP_ITERATIONS = 10

/**
 * Decide whether a loop should stop after the pass that just finished.
 *
 * Split out because the interesting part is testable without a run: the stop
 * reason is what a reader needs afterwards, and "we hit the cap" reads very
 * differently from "the reviewer approved it".
 */
/** Operators that compare against a value; the other two test the variable alone. */
const VALUE_OPERATORS: ConditionOperator[] = ['equals', 'notEquals', 'contains', 'notContains']

export function loopShouldStop(
  until: ConditionConfig | undefined,
  resolvedVariable: string,
  resolvedValue: string
): boolean {
  if (!until) return false

  // A half-written condition means "not configured yet", not "stop now". Typing
  // one into the form leaves it briefly incomplete, and some incomplete forms
  // are degenerate rather than merely false: `contains ""` matches every
  // string, and `notEquals ""` matches everything non-empty. Either would end
  // the loop after one pass, which looks like the loop being broken.
  //
  // The cost is that `equals ""` cannot be expressed, and it does not need to
  // be: isEmpty says exactly that and needs no value.
  if (until.variable.trim() === '') return false
  if (VALUE_OPERATORS.includes(until.operator) && resolvedValue.trim() === '') return false

  return evaluateCondition(until.operator, resolvedVariable, resolvedValue)
}

/**
 * The state a body step is reset to at the start of a pass.
 *
 * Enumerates what SURVIVES rather than what gets cleared. A list of fields to
 * clear has to be updated every time NodeExecutionState gains one, and the
 * failure is silent: the new field leaks from the previous pass into a step
 * reported as pending. The first version of this listed eight fields and
 * missed taskId, the worktree trio, approvedAt and diagnostics — and its test
 * passed, because the test enumerated the same eight.
 */
const SURVIVES_A_PASS = new Set(['nodeId'])

export function blankPassState(
  state: NodeExecutionState,
  iteration: number
): Partial<NodeExecutionState> {
  const cleared: Record<string, unknown> = {}
  for (const key of Object.keys(state)) {
    if (!SURVIVES_A_PASS.has(key)) cleared[key] = undefined
  }
  return { ...cleared, status: 'pending', iteration } as Partial<NodeExecutionState>
}

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

/** A step that failed on its own terms, which is what a retry has somewhere to start from. */
export function hasFailedStep(execution: WorkflowExecution): boolean {
  return execution.nodeStates.some((ns) => ns.status === 'error' && !neverRan(ns))
}

// Whether the run failed, rather than merely holding a step that failed and said so survivably.
export function runEndedInError(
  execution: WorkflowExecution,
  nodes: Map<string, WorkflowNode>,
  skipped?: ReadonlySet<string>
): boolean {
  return execution.nodeStates.some((ns) => {
    if (ns.status !== 'error' || skipped?.has(ns.nodeId)) return false
    if (neverRan(ns)) return true
    const node = nodes.get(ns.nodeId)
    return !node || stopsRunOnError(node)
  })
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

/** Stops at join points whose other predecessors aren't already terminal/skipped. */
export function collectSkippedBranch(
  startNodeId: string,
  successors: Map<string, string[]>,
  predecessors: Map<string, string[]>,
  isTerminal: (nodeId: string) => boolean
): Set<string> {
  const skipped = new Set<string>()
  const queue = [startNodeId]
  while (queue.length > 0) {
    const id = queue.shift()!
    if (skipped.has(id) || isTerminal(id)) continue
    skipped.add(id)
    for (const s of successors.get(id) || []) {
      const otherPreds = (predecessors.get(s) || []).filter(
        (p) => p !== id && !skipped.has(p) && !isTerminal(p)
      )
      if (otherPreds.length === 0) queue.push(s)
    }
  }
  return skipped
}

/**
 * The entry points into the branch a failed node feeds.
 *
 * collectSkippedBranch applies a join guard to every hop it takes, but not to
 * the node it is handed — so the first hop has to be filtered here. Without it,
 * a join directly below the failure is skipped even when another predecessor is
 * still live and about to feed it, and that path dies silently.
 */
export function skipEntryPoints(
  failedNodeId: string,
  edges: { source: string; target: string }[],
  predecessors: Map<string, string[]>,
  isSettled: (nodeId: string) => boolean
): string[] {
  const entries: string[] = []
  for (const edge of edges) {
    if (edge.source !== failedNodeId) continue
    const otherPreds = (predecessors.get(edge.target) || []).filter(
      (p) => p !== failedNodeId && !isSettled(p)
    )
    if (otherPreds.length > 0) continue
    entries.push(edge.target)
  }
  return entries
}

/**
 * The node states a retry starts from: successes adopted, deliberate skips
 * (condition branches, a partial run's slice) preserved, everything else —
 * failures, gate rejections, loop bodies — reset to pending.
 */
export function seedRetryStates(
  workflow: WorkflowDefinition,
  failedRun: WorkflowExecution
): NodeExecutionState[] {
  const priorById = new Map(failedRun.nodeStates.map((ns) => [ns.nodeId, ns]))
  // Body steps chain by real edges but are driven only by their loop; adopting
  // one as completed would let the wave loop race its successor with the loop.
  const bodyIds = new Set<string>()
  for (const n of workflow.nodes) {
    if (n.type !== 'loop') continue
    for (const id of (n.config as LoopConfig).bodyNodeIds ?? []) bodyIds.add(id)
  }
  return workflow.nodes.map((n) => {
    const prior = priorById.get(n.id)
    if (n.type === 'trigger') return { nodeId: n.id, status: 'success' }
    if (bodyIds.has(n.id)) return { nodeId: n.id, status: 'pending' }
    if (prior?.status === 'success') return { ...prior }
    if (prior?.status === 'skipped' && prior.skipReason) return { ...prior }
    return { nodeId: n.id, status: 'pending' }
  })
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
