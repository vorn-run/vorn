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
import { useAppStore } from '../stores'
import { sendWorkflowGateNotification } from './notifications'

const runningWorkflows = new Set<string>()

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

function gateKey(workflowId: string, nodeId: string): string {
  return `${workflowId}:${nodeId}`
}

function scheduleGateTimeout(
  workflowId: string,
  nodeId: string,
  timeoutMs: number | undefined,
  execution: WorkflowExecution,
  elapsedMs = 0
): void {
  if (!timeoutMs || timeoutMs <= 0) return
  const key = gateKey(workflowId, nodeId)
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
      useAppStore.getState().setWorkflowExecution(execution.workflowId, { ...execution })
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
      scheduleGateTimeout(workflow.id, ns.nodeId, timeoutMs, execution, now - startedAt)
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

function persistExecution(workflowId: string, execution: WorkflowExecution): void {
  useAppStore.getState().setWorkflowExecution(workflowId, { ...execution })
  window.api.saveWorkflowRun(execution)
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
  stepOutputs?: StepOutputs
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
    persistExecution(workflow.id, execution)

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

    scheduleGateTimeout(workflow.id, node.id, config.timeoutMs, execution)
    return
  }

  updateNodeState(execution, node.id, {
    status: 'running',
    startedAt: new Date().toISOString()
  })
  persistExecution(workflow.id, execution)

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
    persistExecution(workflow.id, execution)
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
        useAppStore.getState().setWorkflowExecution(workflow.id, { ...execution })
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
    persistExecution(workflow.id, execution)
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
    persistExecution(workflow.id, execution)
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
      persistExecution(workflow.id, execution)
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
    persistExecution(workflow.id, execution)
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

  if (initialPrompt) {
    initialPrompt = buildWorkflowPrompt({
      workflow,
      stepName: config.displayName || node.label,
      userPrompt: initialPrompt
    })
  }

  if (config.headless) {
    console.log(
      `[workflow] creating headless session for "${node.label}" prompt="${(initialPrompt || '').slice(0, 80)}"`
    )

    let sessionId: string | null = null
    let logs = ''

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
          logs = appendBoundedLog(logs, data)
          updateNodeState(execution, node.id, { logs })
          useAppStore.getState().setWorkflowExecution(workflow.id, { ...execution })
          schedulePersistLogs()
        }
      }
    )

    let resolveExit: (code: number) => void
    const exitPromise = new Promise<number>((resolve) => {
      resolveExit = resolve
    })

    const removeExitListener = window.api.onHeadlessExit(
      ({ id, exitCode: code }: { id: string; exitCode: number }) => {
        if (sessionId && id === sessionId) {
          resolveExit(code)
        }
      }
    )

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
        taskId: resolvedTaskId,
        workflowId: workflow.id,
        workflowName: workflow.name,
        args: config.args
      })

      sessionId = headlessSession.id
      useAppStore.getState().addHeadlessSession(headlessSession)

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
      persistExecution(workflow.id, execution)

      if (resolvedTaskId) {
        useAppStore.getState().startTask(resolvedTaskId, headlessSession.id, effectiveAgent)
      }

      const exitCode = await exitPromise

      if (exitCode !== 0) {
        logs += `\nProcess exited with code ${exitCode}`
      }

      updateNodeState(execution, node.id, {
        status: exitCode === 0 ? 'success' : 'error',
        completedAt: new Date().toISOString(),
        output: logs,
        logs,
        ...(exitCode !== 0 && { error: `Exit code ${exitCode}` })
      })
      persistExecution(workflow.id, execution)

      // Reset task back to todo on failure so it can be retried
      if (exitCode !== 0 && resolvedTaskId) {
        useAppStore.getState().reopenTask(resolvedTaskId)
      }
    } finally {
      removeDataListener()
      removeExitListener()
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
      taskId: resolvedTaskId,
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
    persistExecution(workflow.id, execution)
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
    await window.api.runWorkflowManual(workflow.id)
    const existing = useAppStore.getState().workflowExecutions.get(workflow.id)
    if (existing) return existing
    // Return a minimal synthetic execution so callers don't break. The real
    // executions will land via onSchedulerExecute as the scheduler fans out.
    return {
      workflowId: workflow.id,
      startedAt: new Date().toISOString(),
      status: 'running',
      nodeStates: workflow.nodes.map((n) => ({
        nodeId: n.id,
        status: n.type === 'trigger' ? 'success' : 'pending'
      }))
    }
  }

  if (runningWorkflows.has(workflow.id)) {
    console.warn(`[workflow] skipping execution of "${workflow.name}" — already running`)
    const existing = useAppStore.getState().workflowExecutions.get(workflow.id)
    if (existing) return existing
    throw new Error(`Workflow "${workflow.name}" is already executing`)
  }

  const pending = useAppStore.getState().workflowExecutions.get(workflow.id)
  if (pending && pending.nodeStates.some((ns) => ns.status === 'waiting')) {
    console.warn(
      `[workflow] skipping execution of "${workflow.name}" — existing run is waiting for approval`
    )
    return pending
  }

  const execution: WorkflowExecution = {
    workflowId: workflow.id,
    startedAt: new Date().toISOString(),
    status: 'running',
    nodeStates: workflow.nodes.map((n) => ({
      nodeId: n.id,
      status: n.type === 'trigger' ? 'success' : 'pending'
    })),
    triggerTaskId: context?.task?.id
  }

  const actionNodeCount = workflow.nodes.filter((n) => n.type !== 'trigger').length
  console.log(
    `[workflow] executeWorkflow "${workflow.name}" — ${actionNodeCount} action nodes, triggerTaskId=${context?.task?.id}`
  )

  persistExecution(workflow.id, execution)

  return runExecution(workflow, execution, context, options)
}

async function runExecution(
  workflow: WorkflowDefinition,
  execution: WorkflowExecution,
  context: WorkflowExecutionContext | undefined,
  options?: ExecuteWorkflowOptions
): Promise<WorkflowExecution> {
  if (runningWorkflows.has(workflow.id)) {
    console.warn(`[workflow] runExecution: ${workflow.id} already running, skipping`)
    return execution
  }
  runningWorkflows.add(workflow.id)

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

  try {
    let wave = 0
    while (true) {
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
          await executeNode(node, workflow, execution, context, stepOutputs)
        } catch (err) {
          console.error(`[workflow] node "${node.label}" error:`, err)
          updateNodeState(execution, node.id, {
            status: 'error',
            completedAt: new Date().toISOString(),
            error: err instanceof Error ? err.message : String(err)
          })
          persistExecution(workflow.id, execution)
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
              persistExecution(workflow.id, execution)
            }
          }
        }
      })

      await Promise.all(promises)
    }

    const hasWaiting = execution.nodeStates.some((ns) => ns.status === 'waiting')
    if (hasWaiting) {
      persistExecution(workflow.id, execution)
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
      persistExecution(workflow.id, execution)
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
    runningWorkflows.delete(workflow.id)
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

  persistExecution(workflow.id, execution)

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

  const key = gateKey(workflow.id, nodeId)
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
  persistExecution(workflow.id, execution)

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

  persistExecution(workflow.id, execution)

  const context = rebuildContextForResume(execution)
  return runExecution(workflow, execution, context)
}
