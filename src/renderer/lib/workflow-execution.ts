import {
  AiAgentType,
  WorkflowDefinition,
  WorkflowNode,
  WorkflowExecution,
  WorkflowExecutionContext,
  NodeExecutionState,
  LaunchAgentConfig,
  ScriptConfig,
  ConditionConfig,
  ConditionOperator,
  ApprovalConfig,
  CreateTaskFromItemConfig,
  CallConnectorActionConfig,
  TaskConfig,
  getProjectRemoteHostId
} from '../../shared/types'
import { resolveContextField, resolveTemplateVars, StepOutputs } from './template-vars'
import { getWorktreeMode } from './workflow-helpers'
import { buildTaskPrompt, buildWorkflowPrompt } from '../../shared/prompt-builder'
import { extractStructuredOutput } from '../../shared/structured-output'
import { useAppStore } from '../stores'
import { sendWorkflowGateNotification } from './notifications'

/**
 * Runs currently executing in this window, keyed by run id.
 *
 * Keyed by *run*, not by workflow: one workflow can have several runs in flight
 * (a connector poll fans out a run per item), and they must not shoulder each
 * other out. Duplicate suppression is a separate concern handled by the core's
 * claim registry, which sees every instance rather than just this window.
 *
 * The handle carries what stopping a run needs — the sessions it launched, and
 * a signal the steps watch so a stop lands promptly rather than at the next
 * node boundary.
 */
interface ActiveRun {
  runId: string
  workflowId: string
  dedupeParams: string
  abort: AbortController
  /** Headless sessions this run launched, so Stop can kill them. */
  sessionIds: Set<string>
  /**
   * The very object the engine is driving. The store holds shallow copies, so
   * stopping via a copy would leave the running execution's own status stale.
   */
  execution: WorkflowExecution
}

const activeRuns = new Map<string, ActiveRun>()

/** Default ceiling for a headless step that never reports an exit. */
export const DEFAULT_STEP_TIMEOUT_MINUTES = 60

const LOG_BUFFER_MAX = 100_000
const LOG_BUFFER_KEEP = 80_000

/** Cap renderer-resident log buffers so a chatty agent can't exhaust memory. */
function appendBoundedLog(buffer: string, chunk: string): string {
  const next = buffer + chunk
  return next.length > LOG_BUFFER_MAX ? next.slice(-LOG_BUFFER_KEEP) : next
}

/** Tag worktree provenance for cleanup. `undefined` when no worktree is in
 *  play; `'inherited'` when the contextual source supplied one (don't delete);
 *  `'created'` when this node spun one up itself. */
function resolveWorktreeOrigin(
  worktreePath: string | undefined,
  inherited: boolean
): 'created' | 'inherited' | undefined {
  if (!worktreePath) return undefined
  return inherited ? 'inherited' : 'created'
}

const PERSIST_INTERVAL_MS = 3000

/** Cleared on approve/reject so a late timer can't reject an already-resolved gate. */
const gateTimers = new Map<string, ReturnType<typeof setTimeout>>()

/** Scoped to the run, not the workflow — parallel runs each have their own gate. */
function gateKey(runId: string, nodeId: string): string {
  return `${runId}:${nodeId}`
}

function scheduleGateTimeout(
  runId: string,
  nodeId: string,
  timeoutMs: number | undefined,
  execution: WorkflowExecution,
  elapsedMs = 0
): void {
  if (!timeoutMs || timeoutMs <= 0) return
  const key = gateKey(runId, nodeId)
  const prev = gateTimers.get(key)
  if (prev) clearTimeout(prev)
  const remaining = Math.max(0, timeoutMs - elapsedMs)
  const timer = setTimeout(() => {
    gateTimers.delete(key)
    void rejectWorkflowGate(execution, nodeId, `Approval timed out after ${timeoutMs}ms`)
  }, remaining)
  gateTimers.set(key, timer)
}

/**
 * Resolve workflow runs that were left `running` when the renderer last
 * unloaded. The main process keeps headless sessions alive past a renderer
 * reload, so a node that was `running` may already have an `exited` event in
 * `session_events` even though the in-renderer exit-promise was lost.
 *
 * For each `running` node with a session id we look up its lifecycle log; if
 * it exited we mark the node success/error and persist. We do NOT auto-resume
 * the rest of the DAG — quietly continuing a stale run hours later is
 * surprising. Instead, the run is closed as `error` with a clear message so
 * the user can re-run.
 */
export async function reconcileRunningExecutions(
  executions: Iterable<WorkflowExecution>
): Promise<void> {
  for (const execution of executions) {
    if (execution.completedAt && execution.status !== 'running') continue

    let dirty = false
    let anyStillRunning = false
    let anyResolvedHere = false

    const runningNodes = execution.nodeStates.filter((ns) => ns.status === 'running')
    const probes = await Promise.all(
      runningNodes.map(async (ns) => {
        if (!ns.sessionId) return { ns, kind: 'no-session' as const }
        try {
          const events = await window.api.listSessionEventsBySession(ns.sessionId, 50)
          const exitEvent = events.find((e) => e.eventType === 'exited')
          return exitEvent
            ? { ns, kind: 'exited' as const, exitEvent }
            : { ns, kind: 'still-running' as const }
        } catch (err) {
          console.warn(
            `[workflow] reconcile: failed to query session_events for ${ns.sessionId}`,
            err
          )
          return { ns, kind: 'error' as const }
        }
      })
    )

    for (const probe of probes) {
      const { ns } = probe
      if (probe.kind === 'no-session') {
        ns.status = 'error'
        ns.error = 'Run abandoned (no session id recorded)'
        ns.completedAt = new Date().toISOString()
        dirty = true
      } else if (probe.kind === 'exited') {
        const meta = (probe.exitEvent.metadata as { exitCode?: number } | undefined) ?? {}
        const exitCode = typeof meta.exitCode === 'number' ? meta.exitCode : 0
        ns.status = exitCode === 0 ? 'success' : 'error'
        ns.completedAt = probe.exitEvent.timestamp
        if (exitCode !== 0 && !ns.error) ns.error = `Exit code ${exitCode}`
        dirty = true
        anyResolvedHere = true
      } else {
        // 'still-running' or 'error' — leave the node untouched.
        anyStillRunning = true
      }
    }

    // If we resolved at least one node, the rest of the DAG never advanced
    // (the in-memory exit promise died with the previous renderer). Close the
    // run rather than auto-resuming a stale execution.
    if (anyResolvedHere && !anyStillRunning) {
      const hasPending = execution.nodeStates.some((ns) => ns.status === 'pending')
      if (hasPending) {
        for (const ns of execution.nodeStates) {
          if (ns.status === 'pending') {
            ns.status = 'skipped'
            ns.error = 'Renderer reload abandoned this run; re-run to continue'
          }
        }
        execution.status = 'error'
      } else {
        execution.status = execution.nodeStates.some((ns) => ns.status === 'error')
          ? 'error'
          : 'success'
      }
      execution.completedAt = new Date().toISOString()
      dirty = true
    }

    if (dirty) {
      useAppStore.getState().setWorkflowExecution(execution.runId, { ...execution })
      await window.api.saveWorkflowRun(execution)
    }
  }
}

/**
 * Re-arm timeout timers for any approval gates that were `waiting` before the
 * app restarted. Called once after startup hydration; timers elsewhere are set
 * as gates enter `waiting` for the first time.
 */
export function rescheduleWaitingGateTimers(
  executions: Iterable<WorkflowExecution>,
  workflows: WorkflowDefinition[]
): void {
  const now = Date.now()
  for (const execution of executions) {
    const workflow = workflows.find((w) => w.id === execution.workflowId)
    if (!workflow) continue
    for (const ns of execution.nodeStates) {
      if (ns.status !== 'waiting') continue
      const node = workflow.nodes.find((n) => n.id === ns.nodeId)
      if (node?.type !== 'approval') continue
      const timeoutMs = (node.config as ApprovalConfig).timeoutMs
      if (!timeoutMs || timeoutMs <= 0) continue
      const startedAt = ns.startedAt ? new Date(ns.startedAt).getTime() : now
      scheduleGateTimeout(execution.runId, ns.nodeId, timeoutMs, execution, now - startedAt)
    }
  }
}

export interface ExecuteWorkflowOptions {
  source?: 'scheduler' | 'manual'
}

/**
 * Resolve a launchAgent node's configured agent to a concrete AiAgentType,
 * honoring the `'fromTask'` sentinel. Exported so it can be unit-tested
 * without mounting the workflow engine.
 *
 * Precedence for `'fromTask'`:
 *   1. `context.task.assignedAgent` — set when a task-based trigger fired.
 *   2. `resolvedTask.assignedAgent` — set when the node pulled a task via
 *      static `taskId` or `taskFromQueue`.
 *   3. `defaults.defaultAgent` from user config.
 *   4. `'claude'` as a final fallback.
 */
export function resolveEffectiveAgent(
  config: LaunchAgentConfig,
  context: WorkflowExecutionContext | undefined,
  resolvedTask: TaskConfig | undefined
): AiAgentType {
  if (config.agentType !== 'fromTask') return config.agentType
  return (
    context?.task?.assignedAgent ??
    resolvedTask?.assignedAgent ??
    useAppStore.getState().config?.defaults.defaultAgent ??
    'claude'
  )
}

function resolveTaskContext(task: TaskConfig, fallbackBranch?: string, fallbackWorktree?: boolean) {
  const state = useAppStore.getState()
  const project = state.config?.projects.find((p) => p.name === task.projectName)
  let initialPrompt: string
  if (project) {
    const siblingTasks = (state.config?.tasks || []).filter(
      (t) => t.projectName === task.projectName
    )
    initialPrompt = buildTaskPrompt({ task, project, siblingTasks })
  } else {
    initialPrompt = task.description
  }
  return {
    initialPrompt,
    resolvedTaskId: task.id,
    branch: task.branch || fallbackBranch,
    useWorktree: task.useWorktree || fallbackWorktree
  }
}

function persistExecution(execution: WorkflowExecution): void {
  useAppStore.getState().setWorkflowExecution(execution.runId, { ...execution })
  window.api.saveWorkflowRun(execution)
}

/**
 * What this run was triggered *with*. Two runs of one workflow count as the
 * same trigger only when this matches, which is what lets a connector fan-out
 * run its items in parallel while a genuine double-fire collapses to one run.
 */
function dedupeFingerprint(context?: WorkflowExecutionContext): string {
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

/** Resolved step ceiling: the node's own value, else the configured default. 0 disables. */
function resolveStepTimeoutMs(config: LaunchAgentConfig): number {
  if (typeof config.timeoutMs === 'number') return config.timeoutMs
  const minutes =
    useAppStore.getState().config?.defaults?.headlessStepTimeoutMinutes ??
    DEFAULT_STEP_TIMEOUT_MINUTES
  return minutes > 0 ? minutes * 60_000 : 0
}

/**
 * Records what the engine did on a step's behalf, so a step that produced no
 * output still accounts for itself. Times are relative to the step starting,
 * because "the agent was spawned but had written nothing 60 minutes later" is
 * the shape of the answer, not the wall-clock time it happened at.
 */
class StepDiagnostics {
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

function updateNodeState(
  execution: WorkflowExecution,
  nodeId: string,
  updates: Partial<NodeExecutionState>
): void {
  const state = execution.nodeStates.find((s) => s.nodeId === nodeId)
  if (state) {
    Object.assign(state, updates)
  }
}

function buildStepOutputsMap(
  execution: WorkflowExecution,
  nodeMap: Map<string, WorkflowNode>
): StepOutputs {
  const outputs: StepOutputs = {}
  for (const ns of execution.nodeStates) {
    if (ns.status !== 'success' && ns.status !== 'error') continue
    const node = nodeMap.get(ns.nodeId)
    if (!node?.slug) continue

    // Schema-typed connector outputs come first so a declared key like
    // `html_url` wins over the generic fallback — but the three defaults
    // (output/status/error) always overlay so control-flow references keep
    // working regardless of whether the connector returned a typed payload.
    outputs[node.slug] = {
      ...(ns.structuredOutput ?? {}),
      output: ns.output || ns.logs || '',
      status: ns.status,
      error: ns.error || ''
    }
  }
  return outputs
}

function evaluateCondition(operator: ConditionOperator, resolved: string, value: string): boolean {
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

async function executeNode(
  node: WorkflowNode,
  workflow: WorkflowDefinition,
  execution: WorkflowExecution,
  context?: WorkflowExecutionContext,
  stepOutputs?: StepOutputs,
  active?: ActiveRun
): Promise<void> {
  if (node.type === 'approval') {
    const existing = execution.nodeStates.find((s) => s.nodeId === node.id)
    if (existing?.status === 'waiting') return

    const config = node.config as ApprovalConfig
    const timeoutSuffix = config.timeoutMs ? ` (timeout ${config.timeoutMs}ms)` : ''
    console.log(`[workflow] approval gate "${node.label}" waiting${timeoutSuffix}`)

    updateNodeState(execution, node.id, {
      status: 'waiting',
      startedAt: new Date().toISOString()
    })
    persistExecution(execution)

    sendWorkflowGateNotification(
      workflow,
      node.id,
      node.label,
      config.message,
      useAppStore.getState().config ?? null,
      () => {
        useAppStore.getState().setEditingWorkflowId(workflow.id)
        useAppStore.getState().setWorkflowEditorOpen(true)
      }
    )

    scheduleGateTimeout(execution.runId, node.id, config.timeoutMs, execution)
    return
  }

  updateNodeState(execution, node.id, {
    status: 'running',
    startedAt: new Date().toISOString()
  })
  persistExecution(execution)

  if (node.type === 'condition') {
    const config = node.config as ConditionConfig
    const resolved = resolveTemplateVars(config.variable || '', context, stepOutputs)
    const value = resolveTemplateVars(config.value || '', context, stepOutputs)
    const result = evaluateCondition(config.operator, resolved, value)

    console.log(
      `[workflow] condition "${node.label}": "${resolved}" ${config.operator} "${value}" → ${result}`
    )

    updateNodeState(execution, node.id, {
      status: 'success',
      completedAt: new Date().toISOString(),
      output: String(result)
    })
    persistExecution(execution)
    return
  }

  if (node.type === 'script') {
    const config = node.config as ScriptConfig
    console.log(`[workflow] executing script: ${config.scriptType}`)

    const runId = crypto.randomUUID()
    const resolvedConfig: ScriptConfig = {
      ...config,
      scriptContent: resolveTemplateVars(config.scriptContent, context, stepOutputs),
      runId
    }

    let streamedLogs = ''
    const removeScriptDataListener = window.api.onScriptData(
      ({ runId: id, data }: { runId: string; data: string }) => {
        if (id !== runId) return
        streamedLogs = appendBoundedLog(streamedLogs, data)
        updateNodeState(execution, node.id, { logs: streamedLogs })
        // Keyed by the run, like every other write. Under the workflow id this
        // both missed the real run — so streamed script logs never reached it —
        // and inserted a second entry that runsForWorkflow reported as a
        // duplicate run. Note `runId` in this scope is the script's, not the
        // run's.
        useAppStore.getState().setWorkflowExecution(execution.runId, { ...execution })
      }
    )

    try {
      const result = await window.api.executeScript(resolvedConfig)

      const finalLogs = streamedLogs || result.output
      // Streamed logs already include stderr, so only surface result.error
      // when we fell through the non-streaming path (finalLogs === result.output).
      const errorTrailer = result.error && !streamedLogs ? `\nError: ${result.error}` : ''
      updateNodeState(execution, node.id, {
        status: result.success ? 'success' : 'error',
        completedAt: new Date().toISOString(),
        output: result.output,
        logs: finalLogs + errorTrailer,
        error: result.error
      })
    } catch (err) {
      console.error(`[workflow] script execution error:`, err)
      updateNodeState(execution, node.id, {
        status: 'error',
        completedAt: new Date().toISOString(),
        error: err instanceof Error ? err.message : String(err)
      })
    } finally {
      removeScriptDataListener()
    }
    persistExecution(execution)
    return
  }

  if (node.type === 'callConnectorAction') {
    const cfg = node.config as CallConnectorActionConfig
    const resolvedArgs: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(cfg.args ?? {})) {
      resolvedArgs[k] = resolveTemplateVars(v, context, stepOutputs)
    }
    try {
      const result = await window.api.executeConnectorAction({
        connectionId: cfg.connectionId,
        action: cfg.action,
        args: resolvedArgs
      })
      // Only persist plain objects as structuredOutput. Arrays would land
      // here under `typeof === 'object'` but break `buildStepOutputsMap`
      // which spreads the value into a string-keyed map (the array
      // indices `0`, `1`, … would become bogus step keys).
      const isPlainObject =
        !!result.output && typeof result.output === 'object' && !Array.isArray(result.output)
      updateNodeState(execution, node.id, {
        status: result.success ? 'success' : 'error',
        completedAt: new Date().toISOString(),
        output: result.success ? `${cfg.action} succeeded` : `${cfg.action} failed`,
        logs: JSON.stringify(result, null, 2),
        ...(isPlainObject && { structuredOutput: result.output }),
        ...(result.error && { error: result.error })
      })
    } catch (err) {
      updateNodeState(execution, node.id, {
        status: 'error',
        completedAt: new Date().toISOString(),
        error: err instanceof Error ? err.message : String(err)
      })
    }
    persistExecution(execution)
    return
  }

  if (node.type === 'createTaskFromItem') {
    const config = node.config as CreateTaskFromItemConfig
    const item = context?.connectorItem
    if (!item) {
      updateNodeState(execution, node.id, {
        status: 'skipped',
        completedAt: new Date().toISOString(),
        error: 'No connector item in context — this node only runs from a connectorPoll trigger.'
      })
      persistExecution(execution)
      return
    }

    const project =
      config.project === 'fromConnection' || !config.project ? undefined : config.project

    try {
      const result = await window.api.upsertTaskFromItem({
        connectionId: item.connectionId,
        item,
        initialStatus: config.initialStatus,
        ...(project && { project })
      })
      const verb = result.created ? 'Imported' : 'Updated'
      const titleSnippet = item.title.length > 60 ? item.title.slice(0, 57) + '...' : item.title
      const summary = `${verb} #${item.externalId} "${titleSnippet}"`
      updateNodeState(execution, node.id, {
        status: 'success',
        completedAt: new Date().toISOString(),
        taskId: result.taskId,
        output: summary,
        logs: `${summary}\nSource: ${item.externalUrl ?? '(no url)'}\nTaskId: ${result.taskId}`
      })
    } catch (err) {
      updateNodeState(execution, node.id, {
        status: 'error',
        completedAt: new Date().toISOString(),
        error: err instanceof Error ? err.message : String(err)
      })
    }
    persistExecution(execution)
    return
  }

  const config = node.config as LaunchAgentConfig
  console.log(
    `[workflow] launch agent: ${node.label} headless=${config.headless} prompt="${(config.prompt || '').slice(0, 50)}"`
  )

  let initialPrompt = config.prompt
  let resolvedTaskId: string | undefined
  let branch = config.branch ? resolveTemplateVars(config.branch, context) || undefined : undefined
  // When 'fromContext' but no context is available (SourcePromptDialog
  // cancelled mid-flight), fall through with undefined so session creation
  // uses its own default.
  let useWorktree: boolean | undefined =
    config.useWorktree === 'fromContext'
      ? context
        ? ((resolveContextField('useWorktree', context) as boolean | undefined) ?? undefined)
        : undefined
      : config.useWorktree
  const inheritedWorktree = config.useWorktree === 'fromContext'
  let existingWorktreePath: string | undefined
  const currentState = useAppStore.getState()

  const worktreeMode = getWorktreeMode(config)
  if (worktreeMode === 'fromStep') {
    if (!config.worktreeFromStepSlug) {
      throw new Error('Worktree mode "fromStep" requires a source step slug')
    }
    const nodeMap = new Map(workflow.nodes.map((n) => [n.slug || n.id, n]))
    const sourceNode = nodeMap.get(config.worktreeFromStepSlug)
    if (!sourceNode) {
      throw new Error(`Worktree source step "${config.worktreeFromStepSlug}" not found`)
    }
    const sourceState = execution.nodeStates.find((s) => s.nodeId === sourceNode.id)
    if (!sourceState?.worktreePath) {
      throw new Error(`Source step "${config.worktreeFromStepSlug}" has no worktreePath`)
    }
    existingWorktreePath = sourceState.worktreePath
    useWorktree = undefined
  } else if (worktreeMode === 'existing') {
    if (!config.existingWorktreePath) {
      throw new Error('Worktree mode "existing" requires an existingWorktreePath')
    }
    existingWorktreePath = config.existingWorktreePath
    useWorktree = undefined
  } else if (worktreeMode === 'fromContext' && context) {
    // Without this, createTerminal spawns a fresh worktree off project root instead of reusing the source's.
    const ctxWorktree = resolveContextField('worktreePath', context) as string | undefined
    if (ctxWorktree) {
      existingWorktreePath = ctxWorktree
      useWorktree = undefined
    }
  }

  let resolvedTask: TaskConfig | undefined

  // Fall back to the trigger's task id when the node doesn't bind to one
  // statically. This lets the seeded default task workflow stay task-agnostic
  // in its static config while still pulling prompt/branch/worktree from
  // whichever task fired the trigger.
  const effectiveTaskId = config.taskId ?? context?.task?.id
  if (effectiveTaskId) {
    // Status check was previously locked to 'todo'. That's too tight for
    // trigger-driven runs: by the time `taskStatusChanged` fires, the task is
    // already in its target status (typically 'in_progress'). Accept any
    // non-terminal status so both the legacy static-taskId path and the new
    // trigger-driven path work.
    const task = (currentState.config?.tasks || []).find(
      (t) => t.id === effectiveTaskId && t.status !== 'done' && t.status !== 'cancelled'
    )
    if (task) {
      resolvedTask = task
      const ctx = resolveTaskContext(task, branch, useWorktree)
      initialPrompt = ctx.initialPrompt
      resolvedTaskId = ctx.resolvedTaskId
      // Don't let task context override worktree resolution from fromStep/existing
      if (!existingWorktreePath) {
        branch = ctx.branch
        useWorktree = ctx.useWorktree
      }
    }
  } else if (config.taskFromQueue) {
    const task = currentState.getNextTask(config.projectName)
    if (task) {
      resolvedTask = task
      const ctx = resolveTaskContext(task, branch, useWorktree)
      initialPrompt = ctx.initialPrompt
      resolvedTaskId = ctx.resolvedTaskId
      if (!existingWorktreePath) {
        branch = ctx.branch
        useWorktree = ctx.useWorktree
      }
    }
  }

  const effectiveAgent = resolveEffectiveAgent(config, context, resolvedTask)

  // Resolve project name/path from the triggering task when the node config
  // leaves them blank. The seeded default task workflow does exactly this:
  // it's project-agnostic in its static config and relies on the task's
  // project at run time. Without this fallback, createHeadlessSession /
  // createTerminal would receive `cwd: ''` and silently spawn in an
  // undefined directory.
  let effectiveProjectName = resolveTemplateVars(config.projectName ?? '', context) || ''
  let effectiveProjectPath = resolveTemplateVars(config.projectPath ?? '', context) || ''
  if (!effectiveProjectName || !effectiveProjectPath) {
    const taskForProject = context?.task ?? resolvedTask
    if (taskForProject) {
      const proj = currentState.config?.projects.find((p) => p.name === taskForProject.projectName)
      if (proj) {
        effectiveProjectName = effectiveProjectName || proj.name
        effectiveProjectPath = effectiveProjectPath || proj.path
      }
    }
  }
  // If we resolved projectName from context but couldn't pull a path
  // (template-only case), still walk the projects store one more time so
  // contextual workflows launched without a `source` object still work.
  if (effectiveProjectName && !effectiveProjectPath) {
    const proj = currentState.config?.projects.find((p) => p.name === effectiveProjectName)
    if (proj) effectiveProjectPath = proj.path
  }

  if (initialPrompt) {
    initialPrompt = resolveTemplateVars(initialPrompt, context, stepOutputs)
  }

  // Typed output only makes sense headless: the engine parses the finished
  // run's logs, which an interactive terminal session never produces on exit.
  const outputSchema = config.headless ? config.outputSchema : undefined

  if (initialPrompt) {
    initialPrompt = buildWorkflowPrompt({
      workflow,
      stepName: config.displayName || node.label,
      userPrompt: initialPrompt,
      outputSchema
    })
  }

  if (config.headless) {
    console.log(
      `[workflow] creating headless session for "${node.label}" prompt="${(initialPrompt || '').slice(0, 80)}"`
    )

    let sessionId: string | null = null
    let logs = ''
    let bytesFromAgent = 0

    const diag = new StepDiagnostics()
    diag.note(
      `Launching ${effectiveAgent} in ${existingWorktreePath || effectiveProjectPath || '(no path)'}` +
        (initialPrompt
          ? ` with a ${initialPrompt.split('\n').length}-line prompt`
          : ' with no prompt')
    )
    /** Publishes the timeline immediately, so it is readable while the step is still running. */
    const publishDiagnostics = (): void => {
      updateNodeState(execution, node.id, { diagnostics: diag.toString() })
      useAppStore.getState().setWorkflowExecution(execution.runId, { ...execution })
    }
    publishDiagnostics()

    // Logs only live in renderer memory until the node finishes — if the
    // window reloads (HMR, devtools refresh, crash) mid-run they vanish even
    // though the headless agent in the main process keeps going. Throttle a
    // background save to disk so accumulated output survives a reload.
    let persistTimer: ReturnType<typeof setTimeout> | null = null
    let lastPersistedBytes = 0
    const schedulePersistLogs = () => {
      if (persistTimer) return
      persistTimer = setTimeout(() => {
        persistTimer = null
        if (logs.length === lastPersistedBytes) return
        lastPersistedBytes = logs.length
        // Only persist; the in-memory store was already updated by the
        // listener so the editor UI is up to date already.
        void window.api.saveWorkflowRun(execution)
      }, PERSIST_INTERVAL_MS)
    }

    const removeDataListener = window.api.onHeadlessData(
      ({ id, data }: { id: string; data: string }) => {
        if (sessionId && id === sessionId) {
          if (bytesFromAgent === 0) {
            // Whether the agent ever spoke at all is the single most useful
            // fact when a step stalls, so mark the first byte specifically.
            diag.note(`First output from the agent (${data.length} bytes)`)
            updateNodeState(execution, node.id, { diagnostics: diag.toString() })
          }
          bytesFromAgent += data.length
          logs = appendBoundedLog(logs, data)
          updateNodeState(execution, node.id, { logs })
          useAppStore.getState().setWorkflowExecution(execution.runId, { ...execution })
          schedulePersistLogs()
        }
      }
    )

    /**
     * How the step ends: the agent's exit code, or why we stopped waiting for
     * one. Waiting on the exit event alone is what wedged runs — an agent that
     * never exits, or an exit that arrived before we knew our own session id,
     * left the step pending forever and its run open with it.
     */
    type StepOutcome =
      | { kind: 'exit'; code: number }
      | { kind: 'timeout'; afterMs: number }
      | { kind: 'stopped' }

    // Definitely assigned: the Promise executor runs synchronously.
    let settle!: (outcome: StepOutcome) => void
    const outcomePromise = new Promise<StepOutcome>((resolve) => {
      settle = resolve
    })

    /**
     * Exits seen before `createHeadlessSession` returned. The agent can be dead
     * before we learn its id — a Windows shim that fails immediately exits in
     * milliseconds — and an exit dropped there used to be unrecoverable.
     */
    const exitsBeforeIdKnown = new Map<string, number>()

    const removeExitListener = window.api.onHeadlessExit(
      ({ id, exitCode: code }: { id: string; exitCode: number }) => {
        if (!sessionId) {
          exitsBeforeIdKnown.set(id, code)
          return
        }
        if (id === sessionId) settle({ kind: 'exit', code })
      }
    )

    const timeoutMs = resolveStepTimeoutMs(config)
    const timeoutTimer =
      timeoutMs > 0
        ? setTimeout(() => settle({ kind: 'timeout', afterMs: timeoutMs }), timeoutMs)
        : null

    const onAbort = (): void => settle({ kind: 'stopped' })
    active?.abort.signal.addEventListener('abort', onAbort, { once: true })

    try {
      const headlessSession = await window.api.createHeadlessSession({
        agentType: effectiveAgent,
        projectName: effectiveProjectName,
        projectPath: effectiveProjectPath,
        displayName: config.displayName || node.label,
        branch,
        useWorktree,
        existingWorktreePath,
        initialPrompt,
        promptDelayMs: config.promptDelayMs,
        headless: true,
        workflowId: workflow.id,
        workflowName: workflow.name,
        args: config.args
      })

      sessionId = headlessSession.id
      active?.sessionIds.add(headlessSession.id)
      useAppStore.getState().addHeadlessSession(headlessSession)

      diag.note(
        `Session ${headlessSession.id} started (pid ${headlessSession.pid || 'unknown'})` +
          (headlessSession.launchCommand ? `: ${headlessSession.launchCommand}` : '')
      )
      if (timeoutMs > 0) {
        diag.note(`Will give up after ${Math.round(timeoutMs / 60_000)} min without an exit`)
      }

      // Claim any exit that landed while we were still learning our id.
      const raced = exitsBeforeIdKnown.get(headlessSession.id)
      if (raced !== undefined) {
        diag.note(`Agent had already exited (code ${raced}) before its id reached us`)
        settle({ kind: 'exit', code: raced })
      }
      exitsBeforeIdKnown.clear()

      updateNodeState(execution, node.id, {
        sessionId: headlessSession.id,
        taskId: resolvedTaskId,
        worktreePath: headlessSession.worktreePath,
        worktreeName: headlessSession.worktreeName,
        worktreeOrigin: resolveWorktreeOrigin(headlessSession.worktreePath, inheritedWorktree),
        agentType: effectiveAgent,
        projectName: effectiveProjectName,
        projectPath: effectiveProjectPath,
        ...(headlessSession.agentSessionId
          ? { agentSessionId: headlessSession.agentSessionId }
          : {})
      })
      persistExecution(execution)

      if (resolvedTaskId) {
        useAppStore.getState().startTask(resolvedTaskId, headlessSession.id, effectiveAgent)
      }

      const outcome = await outcomePromise

      // Stopped: stopWorkflowRun owns the node's terminal state and has already
      // killed the agent. Writing a status here would just fight it.
      if (outcome.kind === 'stopped') {
        diag.note('Stopped by user')
        updateNodeState(execution, node.id, { diagnostics: diag.toString() })
        return
      }

      if (outcome.kind === 'timeout') {
        const minutes = Math.round(outcome.afterMs / 60_000)
        // Silence and slowness are different failures with different fixes, and
        // the log alone can't tell them apart when it's empty either way.
        const reason =
          bytesFromAgent === 0
            ? `Step timed out after ${minutes} minute${minutes === 1 ? '' : 's'}. The agent was started but never produced any output, which usually means it never really ran or is waiting on input it will never get.`
            : `Step timed out after ${minutes} minute${minutes === 1 ? '' : 's'} without the agent exiting, after ${bytesFromAgent} bytes of output.`
        console.warn(
          `[workflow] "${node.label}": ${reason} — killing session ${headlessSession.id}`
        )
        diag.note(reason)
        try {
          await window.api.killHeadlessSession(headlessSession.id)
          diag.note('Agent killed')
        } catch (err) {
          console.warn(`[workflow] failed to kill timed-out session ${headlessSession.id}`, err)
          diag.note(`Could not kill the agent: ${err instanceof Error ? err.message : String(err)}`)
        }
        updateNodeState(execution, node.id, {
          status: 'error',
          completedAt: new Date().toISOString(),
          output: logs,
          logs,
          error: reason,
          diagnostics: diag.toString()
        })
        persistExecution(execution)
        if (resolvedTaskId) useAppStore.getState().reopenTask(resolvedTaskId)
        return
      }

      const exitCode = outcome.code
      diag.note(
        `Agent exited with code ${exitCode} after ${bytesFromAgent} bytes of output` +
          (bytesFromAgent === 0 ? ' — it produced nothing at all' : '')
      )

      if (exitCode !== 0) {
        logs += `\nProcess exited with code ${exitCode}`
      }

      // Typed output: a clean exit still fails the node if the agent didn't
      // return a parseable object matching the declared schema, so downstream
      // branches never gate on garbage. On success the parsed object becomes the
      // node's structuredOutput → `{{steps.<slug>.<field>}}`.
      let structured: Record<string, unknown> | undefined
      let schemaError: string | undefined
      if (exitCode === 0 && outputSchema) {
        const result = extractStructuredOutput(logs, outputSchema)
        if (result.output) structured = result.output
        else schemaError = result.error || 'Agent output did not match the declared schema.'
      }

      if (schemaError) diag.note(`Output did not match the declared schema: ${schemaError}`)

      const failed = exitCode !== 0 || !!schemaError
      updateNodeState(execution, node.id, {
        diagnostics: diag.toString(),
        status: failed ? 'error' : 'success',
        completedAt: new Date().toISOString(),
        output: logs,
        logs,
        ...(structured && { structuredOutput: structured }),
        ...(exitCode !== 0 && { error: `Exit code ${exitCode}` }),
        ...(schemaError && exitCode === 0 && { error: schemaError })
      })
      persistExecution(execution)

      // Reset task back to todo on failure so it can be retried
      if (failed && resolvedTaskId) {
        useAppStore.getState().reopenTask(resolvedTaskId)
      }
    } catch (err) {
      // The step never got as far as running — most often the worktree or the
      // agent binary. Without this the timeline stops at "Launching…" and the
      // reason lives only in the error string.
      diag.note(`Could not start: ${err instanceof Error ? err.message : String(err)}`)
      throw err
    } finally {
      // In the finally so it survives the throw path too; the caller's error
      // handler assigns over the node state and leaves this field intact.
      updateNodeState(execution, node.id, { diagnostics: diag.toString() })
      removeDataListener()
      removeExitListener()
      active?.abort.signal.removeEventListener('abort', onAbort)
      if (timeoutTimer) clearTimeout(timeoutTimer)
      if (persistTimer) {
        clearTimeout(persistTimer)
        persistTimer = null
      }
    }
  } else {
    const cfg = useAppStore.getState().config
    const proj = cfg?.projects.find((p) => p.name === effectiveProjectName)
    const remoteHostId = proj ? getProjectRemoteHostId(proj) : undefined
    const session = await window.api.createTerminal({
      agentType: effectiveAgent,
      projectName: effectiveProjectName,
      projectPath: effectiveProjectPath,
      displayName: config.displayName || node.label,
      branch,
      useWorktree,
      existingWorktreePath,
      initialPrompt,
      promptDelayMs: config.promptDelayMs,
      args: config.args,
      remoteHostId
    })
    useAppStore.getState().addTerminal(session)

    if (resolvedTaskId) {
      useAppStore.getState().startTask(resolvedTaskId, session.id, effectiveAgent)
    }

    updateNodeState(execution, node.id, {
      status: 'success',
      completedAt: new Date().toISOString(),
      sessionId: session.id,
      logs: `Terminal session created: ${session.id}`,
      taskId: resolvedTaskId,
      worktreePath: session.worktreePath,
      worktreeName: session.worktreeName,
      worktreeOrigin: resolveWorktreeOrigin(session.worktreePath, inheritedWorktree),
      agentType: effectiveAgent,
      projectName: effectiveProjectName,
      projectPath: effectiveProjectPath
    })
    persistExecution(execution)
  }
}

function buildGraph(edges: readonly { source: string; target: string }[]): {
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
function collectSkippedBranch(
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

export async function executeWorkflow(
  workflow: WorkflowDefinition,
  context?: WorkflowExecutionContext,
  options?: ExecuteWorkflowOptions
): Promise<WorkflowExecution> {
  // connectorPoll workflows cannot be run directly from the renderer — the
  // scheduler owns the poll + fan-out. Route user-initiated "Run" clicks
  // through workflow:runManual. Scheduler-originated runs already carry a
  // connectorItem (per-item fan-out) so they don't need this reroute.
  const triggerNode = workflow.nodes.find((n) => n.type === 'trigger')
  const triggerCfg = triggerNode?.config as { triggerType?: string } | undefined
  if (
    triggerCfg?.triggerType === 'connectorPoll' &&
    !context?.connectorItem &&
    options?.source !== 'scheduler'
  ) {
    await window.api.runWorkflowManual(workflow.id, context?.inputs)
    const existing = latestRunForWorkflow(workflow.id)
    if (existing) return existing
    // Return a minimal synthetic execution so callers don't break. The real
    // executions will land via onSchedulerExecute as the scheduler fans out.
    return {
      runId: `pending:${workflow.id}`,
      workflowId: workflow.id,
      startedAt: new Date().toISOString(),
      status: 'running',
      nodeStates: workflow.nodes.map((n) => ({
        nodeId: n.id,
        status: n.type === 'trigger' ? 'success' : 'pending'
      }))
    }
  }

  // A run parked on an approval gate still owns the workflow's queue position —
  // starting another now would race two runs through the same gate.
  const waiting = runsForWorkflow(workflow.id).find((e) =>
    e.nodeStates.some((ns) => ns.status === 'waiting')
  )
  if (waiting) {
    console.warn(
      `[workflow] skipping execution of "${workflow.name}" — existing run is waiting for approval`
    )
    return waiting
  }

  // Ask the core, not this window, whether we own this trigger. Every instance
  // hears the same scheduler tick; only the one granted the claim runs it.
  const dedupeParams = dedupeFingerprint(context)
  const claim = await window.api.claimWorkflowRun({ workflowId: workflow.id, params: dedupeParams })
  if (!claim.granted) {
    console.warn(
      `[workflow] skipping execution of "${workflow.name}" — trigger already claimed (params=${dedupeParams})`
    )
    const existing = useAppStore.getState().workflowExecutions.get(claim.runId)
    if (existing) return existing
    throw new Error(`Workflow "${workflow.name}" is already running for this trigger`)
  }

  const execution: WorkflowExecution = {
    runId: claim.runId,
    workflowId: workflow.id,
    startedAt: new Date().toISOString(),
    status: 'running',
    nodeStates: workflow.nodes.map((n) => ({
      nodeId: n.id,
      status: n.type === 'trigger' ? 'success' : 'pending'
    })),
    triggerTaskId: context?.task?.id,
    dedupeParams,
    inputs: context?.inputs
  }

  const actionNodeCount = workflow.nodes.filter((n) => n.type !== 'trigger').length
  console.log(
    `[workflow] executeWorkflow "${workflow.name}" — ${actionNodeCount} action nodes, run=${execution.runId}, triggerTaskId=${context?.task?.id}`
  )

  persistExecution(execution)

  return runExecution(workflow, execution, context, options)
}

/** Live runs of one workflow, newest first. */
function runsForWorkflow(workflowId: string): WorkflowExecution[] {
  return Array.from(useAppStore.getState().workflowExecutions.values())
    .filter((e) => e.workflowId === workflowId)
    .sort((a, b) => b.startedAt.localeCompare(a.startedAt))
}

function latestRunForWorkflow(workflowId: string): WorkflowExecution | undefined {
  return runsForWorkflow(workflowId)[0]
}

/**
 * Stop a run: kill the agents it launched, then close it as `cancelled`.
 *
 * Worktrees are deliberately left on disk. A stopped run has usually done
 * partial work, and discarding it silently is not something the user can undo.
 */
export async function stopWorkflowRun(runId: string): Promise<void> {
  const handle = activeRuns.get(runId)
  // Prefer the live object over the store's copy so the run the engine is
  // driving sees the cancellation too, not just the snapshot the UI renders.
  const execution = handle?.execution ?? useAppStore.getState().workflowExecutions.get(runId)
  if (!execution && !handle) {
    console.warn(`[workflow] stopWorkflowRun: no run ${runId}`)
    return
  }

  handle?.abort.abort()

  // Kill from the node states as well as the handle: a run rehydrated after a
  // reload has sessions recorded but no in-memory handle.
  const sessionIds = new Set<string>(handle?.sessionIds ?? [])
  for (const ns of execution?.nodeStates ?? []) {
    if (ns.sessionId && (ns.status === 'running' || ns.status === 'waiting')) {
      sessionIds.add(ns.sessionId)
    }
  }
  await Promise.allSettled(
    Array.from(sessionIds).map((id) =>
      Promise.resolve(window.api.killHeadlessSession(id)).catch((err) =>
        console.warn(`[workflow] stop: failed to kill session ${id}`, err)
      )
    )
  )

  if (!execution) return

  const now = new Date().toISOString()
  for (const ns of execution.nodeStates) {
    if (ns.status === 'running' || ns.status === 'pending' || ns.status === 'waiting') {
      ns.status = ns.status === 'pending' ? 'skipped' : 'error'
      ns.completedAt = now
      ns.error = 'Stopped by user'
    }
    // Drop any armed approval timer for this run so it can't fire post-stop.
    const timer = gateTimers.get(gateKey(runId, ns.nodeId))
    if (timer) {
      clearTimeout(timer)
      gateTimers.delete(gateKey(runId, ns.nodeId))
    }
  }
  execution.status = 'cancelled'
  execution.completedAt = now
  persistExecution(execution)

  // Release immediately rather than at the dedupe window's expiry, so stopping
  // a run and starting it again is not blocked by the run just stopped.
  const workflow = (useAppStore.getState().config?.workflows || []).find(
    (w) => w.id === execution.workflowId
  )
  await Promise.allSettled([
    window.api.releaseWorkflowRun({
      workflowId: execution.workflowId,
      params: execution.dedupeParams,
      runId
    }),
    window.api.reportWorkflowComplete({
      workflowId: execution.workflowId,
      workflowName: workflow?.name ?? execution.workflowId,
      completedAt: now,
      status: 'cancelled',
      sessionsLaunched: sessionIds.size
    })
  ])
  console.log(`[workflow] run ${runId} stopped by user`)
}

/** Whether a run can still be stopped — drives the Stop control's visibility. */
export function isRunStoppable(execution: WorkflowExecution): boolean {
  return execution.status === 'running'
}

async function runExecution(
  workflow: WorkflowDefinition,
  execution: WorkflowExecution,
  context: WorkflowExecutionContext | undefined,
  options?: ExecuteWorkflowOptions
): Promise<WorkflowExecution> {
  // Guards re-entry into *this* run (a gate approved twice, say). Other runs of
  // the same workflow are free to proceed alongside it.
  if (activeRuns.has(execution.runId)) {
    console.warn(`[workflow] runExecution: run ${execution.runId} already active, skipping`)
    return execution
  }
  const active: ActiveRun = {
    runId: execution.runId,
    workflowId: workflow.id,
    dedupeParams: execution.dedupeParams ?? 'manual',
    abort: new AbortController(),
    sessionIds: new Set(),
    execution
  }
  activeRuns.set(execution.runId, active)

  const nodeMap = new Map(workflow.nodes.map((n) => [n.id, n]))
  const { successors: successorsMap, predecessors: predecessorsMap } = buildGraph(workflow.edges)

  // Rebuilt at the start of every wave so external mutations (e.g. a sibling
  // gate approved mid-loop by another re-entry) are picked up.
  const completed = new Set<string>()
  const skippedByCondition = new Set<string>()
  function rebuildCompletionSets(): void {
    completed.clear()
    skippedByCondition.clear()
    for (const ns of execution.nodeStates) {
      if (ns.status === 'success' || ns.status === 'error') completed.add(ns.nodeId)
      else if (ns.status === 'skipped') skippedByCondition.add(ns.nodeId)
    }
  }

  const running = new Set<string>()

  function markSkippedBranch(startNodeId: string): void {
    const branch = collectSkippedBranch(
      startNodeId,
      successorsMap,
      predecessorsMap,
      (id) => completed.has(id) || skippedByCondition.has(id)
    )
    for (const id of branch) skippedByCondition.add(id)
  }

  function getReadyNodes(): WorkflowNode[] {
    const ready: WorkflowNode[] = []
    for (const node of workflow.nodes) {
      if (node.type === 'trigger') continue
      if (completed.has(node.id) || running.has(node.id)) continue
      if (skippedByCondition.has(node.id)) continue
      const ns = execution.nodeStates.find((s) => s.nodeId === node.id)
      if (ns?.status === 'waiting') continue

      const preds = predecessorsMap.get(node.id) || []
      const allPredsReady = preds.every((p) => completed.has(p) || skippedByCondition.has(p))
      if (allPredsReady && preds.some((p) => completed.has(p))) {
        ready.push(node)
      }
    }
    return ready
  }

  const actionNodeCount = workflow.nodes.filter((n) => n.type !== 'trigger').length

  // A run parked on a gate is still live, so its claim stays held; only a run
  // that reaches a terminal state gives the trigger back.
  let parkedOnGate = false

  try {
    let wave = 0
    while (true) {
      if (active.abort.signal.aborted) break
      rebuildCompletionSets()
      const ready = getReadyNodes()
      if (ready.length === 0) break

      wave++
      console.log(
        `[workflow] wave ${wave}: executing ${ready.length} node(s) in parallel: ${ready.map((n) => n.label).join(', ')}`
      )

      if (wave > 1 && workflow.staggerDelayMs) {
        await new Promise((r) => setTimeout(r, workflow.staggerDelayMs))
      }

      const stepOutputs = buildStepOutputsMap(execution, nodeMap)

      const promises = ready.map(async (node) => {
        running.add(node.id)
        try {
          await executeNode(node, workflow, execution, context, stepOutputs, active)
        } catch (err) {
          console.error(`[workflow] node "${node.label}" error:`, err)
          updateNodeState(execution, node.id, {
            status: 'error',
            completedAt: new Date().toISOString(),
            error: err instanceof Error ? err.message : String(err)
          })
          persistExecution(execution)
        }
        running.delete(node.id)

        const postState = execution.nodeStates.find((s) => s.nodeId === node.id)
        if (postState?.status === 'waiting') return

        completed.add(node.id)

        // After a condition node completes, skip the non-matching branch
        if (node.type === 'condition') {
          const condState = execution.nodeStates.find((s) => s.nodeId === node.id)
          const result = condState?.output // "true" or "false"
          const skipBranch = result === 'true' ? 'false' : 'true'

          for (const edge of workflow.edges) {
            if (edge.source === node.id && edge.conditionBranch === skipBranch) {
              markSkippedBranch(edge.target)
              // Mark skipped nodes in execution state
              for (const skippedId of skippedByCondition) {
                updateNodeState(execution, skippedId, {
                  status: 'skipped',
                  completedAt: new Date().toISOString()
                })
              }
              persistExecution(execution)
            }
          }
        }
      })

      await Promise.all(promises)
    }

    // Stopped mid-flight: stopWorkflowRun already wrote the terminal state, so
    // leave it alone rather than recomputing it from the half-finished DAG.
    if (active.abort.signal.aborted) {
      return execution
    }

    const hasWaiting = execution.nodeStates.some((ns) => ns.status === 'waiting')
    if (hasWaiting) {
      parkedOnGate = true
      persistExecution(execution)
      return execution
    }

    // Mark any nodes still pending as skipped (unreachable due to missing edges or cycles)
    const pendingNodes = execution.nodeStates.filter((ns) => ns.status === 'pending')
    if (pendingNodes.length > 0) {
      for (const ns of pendingNodes) {
        ns.status = 'error'
        ns.completedAt = new Date().toISOString()
        ns.error = 'Skipped: predecessor nodes did not complete'
      }
      persistExecution(execution)
    }

    const hasErrors = execution.nodeStates.some(
      (ns) => ns.status === 'error' && !skippedByCondition.has(ns.nodeId)
    )
    execution.status = hasErrors ? 'error' : 'success'
    execution.completedAt = new Date().toISOString()
  } catch (err) {
    console.error(`[workflow] execution error:`, err)
    execution.status = 'error'
    execution.completedAt = new Date().toISOString()
    for (const ns of execution.nodeStates) {
      if (ns.status === 'running' || ns.status === 'pending') {
        ns.status = 'error'
        ns.completedAt = execution.completedAt
        ns.error = err instanceof Error ? err.message : String(err)
      }
    }
  } finally {
    activeRuns.delete(execution.runId)
    if (!parkedOnGate) {
      // Hand the trigger back so an identical one can run again immediately
      // instead of waiting out the dedupe window.
      void window.api
        .releaseWorkflowRun({
          workflowId: workflow.id,
          params: active.dedupeParams,
          runId: execution.runId
        })
        .catch((err) => console.warn('[workflow] failed to release run claim:', err))
    }
  }

  const state = useAppStore.getState()
  const terminals = state.terminals
  const headlessById = new Map(state.headlessSessions.map((s) => [s.id, s]))
  for (const ns of execution.nodeStates) {
    if (ns.sessionId && !ns.agentSessionId) {
      const agentSid =
        terminals.get(ns.sessionId)?.session.agentSessionId ??
        headlessById.get(ns.sessionId)?.agentSessionId
      if (agentSid) {
        ns.agentSessionId = agentSid
      }
    }
  }

  persistExecution(execution)

  if (workflow.autoCleanupWorktrees) {
    // Skip 'inherited' worktrees: a contextual workflow reused the parent
    // card/terminal's worktree, so deleting it would nuke work the user is
    // actively in.
    const worktreeMap = new Map<string, string>()
    for (const ns of execution.nodeStates) {
      if (!ns.worktreePath || worktreeMap.has(ns.worktreePath)) continue
      if (ns.worktreeOrigin === 'inherited') continue
      const node = workflow.nodes.find((n) => n.id === ns.nodeId)
      if (!node || node.type !== 'launchAgent') continue
      const cfg = node.config as LaunchAgentConfig
      if (getWorktreeMode(cfg) === 'new') {
        worktreeMap.set(ns.worktreePath, ns.projectPath || cfg.projectPath)
      }
    }

    await Promise.allSettled(
      Array.from(worktreeMap.entries()).map(async ([wtPath, projectPath]) => {
        if (!projectPath) return
        const { count } = await window.api.getWorktreeActiveSessions(wtPath)
        if (count > 0) {
          console.log(`[workflow] skipping worktree cleanup (${count} active sessions): ${wtPath}`)
          return
        }
        const dirty = await window.api.isWorktreeDirty(wtPath)
        if (dirty) {
          console.log(`[workflow] skipping dirty worktree cleanup: ${wtPath}`)
          return
        }
        await window.api.removeWorktree(projectPath, wtPath, false)
        console.log(`[workflow] auto-cleaned worktree: ${wtPath}`)
      })
    )
  }

  // Report completion to main process for schedule log + workflow status update
  await window.api.reportWorkflowComplete({
    workflowId: workflow.id,
    workflowName: workflow.name,
    completedAt: execution.completedAt!,
    status: execution.status,
    sessionsLaunched: actionNodeCount,
    source: options?.source
  })

  if (Notification.permission === 'granted') {
    new Notification('Vorn', {
      body: `Workflow "${workflow.name}" ${execution.status === 'success' ? 'completed' : 'failed'} — ${actionNodeCount} node${actionNodeCount !== 1 ? 's' : ''}`
    })
  }

  return execution
}

/**
 * Only `triggerTaskId` is persisted — trigger.fromStatus/toStatus aren't,
 * so `{{trigger.fromStatus}}` template vars in post-gate nodes are empty on resume.
 */
function rebuildContextForResume(
  execution: WorkflowExecution
): WorkflowExecutionContext | undefined {
  if (!execution.triggerTaskId) return undefined
  const task = (useAppStore.getState().config?.tasks || []).find(
    (t) => t.id === execution.triggerTaskId
  )
  return task ? { task } : undefined
}

function resolveWaitingGate(
  execution: WorkflowExecution,
  nodeId: string,
  caller: 'approve' | 'reject'
): { workflow: WorkflowDefinition } | null {
  const workflow = (useAppStore.getState().config?.workflows || []).find(
    (w) => w.id === execution.workflowId
  )
  if (!workflow) {
    console.warn(`[workflow] ${caller}WorkflowGate: workflow ${execution.workflowId} not found`)
    return null
  }

  const ns = execution.nodeStates.find((s) => s.nodeId === nodeId)
  if (!ns || ns.status !== 'waiting') {
    console.warn(
      `[workflow] ${caller}WorkflowGate: node ${nodeId} not waiting (status=${ns?.status})`
    )
    return null
  }

  const key = gateKey(execution.runId, nodeId)
  const timer = gateTimers.get(key)
  if (timer) {
    clearTimeout(timer)
    gateTimers.delete(key)
  }

  return { workflow }
}

/** Safe to call on an execution loaded from the database (cross-session resume). */
export async function approveWorkflowGate(
  execution: WorkflowExecution,
  nodeId: string
): Promise<WorkflowExecution> {
  const resolved = resolveWaitingGate(execution, nodeId, 'approve')
  if (!resolved) return execution
  const { workflow } = resolved

  const now = new Date().toISOString()
  updateNodeState(execution, nodeId, {
    status: 'success',
    completedAt: now,
    approvedAt: now
  })
  persistExecution(execution)

  const context = rebuildContextForResume(execution)
  return runExecution(workflow, execution, context)
}

export async function rejectWorkflowGate(
  execution: WorkflowExecution,
  nodeId: string,
  reason = 'Rejected by user'
): Promise<WorkflowExecution> {
  const resolved = resolveWaitingGate(execution, nodeId, 'reject')
  if (!resolved) return execution
  const { workflow } = resolved

  const now = new Date().toISOString()
  updateNodeState(execution, nodeId, {
    status: 'error',
    completedAt: now,
    error: reason
  })

  const { successors, predecessors } = buildGraph(workflow.edges)
  const isTerminal = (id: string): boolean => {
    const s = execution.nodeStates.find((n) => n.nodeId === id)
    return !!s && (s.status === 'success' || s.status === 'error' || s.status === 'skipped')
  }
  for (const succ of successors.get(nodeId) || []) {
    const branch = collectSkippedBranch(succ, successors, predecessors, isTerminal)
    for (const id of branch) {
      const state = execution.nodeStates.find((n) => n.nodeId === id)
      if (state?.status === 'pending') {
        updateNodeState(execution, id, { status: 'skipped', completedAt: now })
      }
    }
  }

  persistExecution(execution)

  const context = rebuildContextForResume(execution)
  return runExecution(workflow, execution, context)
}
