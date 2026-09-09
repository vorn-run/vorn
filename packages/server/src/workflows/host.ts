import {
  IPC,
  type AppConfig,
  type ConnectorItemContext,
  type CreateTerminalPayload,
  type HeadlessSession,
  type ScriptConfig,
  type SessionEvent,
  type TaskConfig,
  type TaskStatus,
  type TerminalSession,
  type WorkflowExecution
} from '@vornrun/shared/types'
import { callMethod } from '../ws-handler'
import { clientRegistry } from '../broadcast'
import { configManager } from '../config-manager'
import { headlessManager } from '../headless-manager'
import { ptyManager } from '../pty-manager'
import { scriptRunnerEvents } from '../script-runner'
import { getWorkflowRun } from '../database'
import log from '../logger'

/**
 * What the engine used to reach for through `window.api`.
 *
 * The shape is deliberately the preload's, because the engine is the same
 * engine: it moved here from the renderer, and every call site it kept is a
 * call the app made a moment ago. Underneath, each one goes through the
 * server's own method registry rather than a socket -- so a session created by
 * a workflow is created the way a person creates one, transcript naming and
 * broadcasts included, with no second implementation to keep in step.
 */
export const api = {
  completeConnectorInbox: (params: {
    id: number
    leaseToken: string
    disposition: 'processed' | 'retry' | 'defer'
    error?: string
  }): Promise<void> => callMethod('connector:inboxComplete', params),

  renewConnectorInbox: (params: { id: number; leaseToken: string }): Promise<boolean> =>
    callMethod('connector:inboxRenew', params),

  // Writes only. Telling clients is `publishRun`, which the engine calls at the
  // points a viewer cares about -- including ones where nothing is saved.
  saveWorkflowRun: (execution: WorkflowExecution): Promise<void> =>
    callMethod('workflowRun:save', execution),

  listSessionEventsBySession: (sessionId: string, limit?: number): Promise<SessionEvent[]> =>
    callMethod('sessionEvent:listBySession', { sessionId, limit }),

  executeScript: (
    config: ScriptConfig
  ): Promise<{ success: boolean; output: string; error?: string; exitCode?: number }> =>
    callMethod('script:execute', config),

  executeConnectorAction: (params: {
    connectionId: string
    action: string
    args: Record<string, unknown>
  }): Promise<{ success: boolean; output?: Record<string, unknown>; error?: string }> =>
    callMethod('connection:executeAction', params),

  httpRequest: (params: {
    profileConnectionId?: string
    method: string
    url: string
    headers?: Record<string, string>
    body?: string
  }): Promise<{ success: boolean; output?: Record<string, unknown>; error?: string }> =>
    callMethod('http:request', params),

  upsertTaskFromItem: (params: {
    connectionId: string
    item: ConnectorItemContext
    initialStatus: TaskStatus
    project?: string
  }): Promise<{ taskId: string; created: boolean }> =>
    callMethod('connection:upsertFromItem', params),

  createHeadlessSession: (payload: CreateTerminalPayload): Promise<HeadlessSession> =>
    callMethod('headless:create', payload),

  killHeadlessSession: (id: string): Promise<void> => callMethod('headless:kill', id),

  createTerminal: (payload: CreateTerminalPayload): Promise<TerminalSession> =>
    callMethod('terminal:create', payload),

  /** A connector-poll workflow is fanned out by the scheduler, not run directly. */
  runWorkflowManual: (workflowId: string, inputs?: Record<string, unknown>): Promise<void> =>
    callMethod('workflow:runManual', { workflowId, inputs }),

  claimWorkflowRun: (params: {
    workflowId: string
    params?: string
  }): Promise<{ granted: boolean; runId: string }> => callMethod('workflowRun:claim', params),

  releaseWorkflowRun: (params: {
    workflowId: string
    params?: string
    runId: string
  }): Promise<void> => callMethod('workflowRun:release', params),

  reportWorkflowComplete: (data: {
    workflowId: string
    workflowName: string
    completedAt: string
    status: 'success' | 'error' | 'cancelled'
    sessionsLaunched: number
    source?: 'scheduler' | 'manual'
  }): Promise<void> => callMethod('workflow:executionComplete', data),

  getWorktreeActiveSessions: (
    worktreePath: string
  ): Promise<{ count: number; sessionIds: string[] }> =>
    callMethod('worktree:activeSessions', worktreePath),

  isWorktreeDirty: (worktreePath: string): Promise<boolean> =>
    callMethod('git:worktreeDirty', worktreePath),

  removeWorktree: (projectPath: string, worktreePath: string, force?: boolean): Promise<boolean> =>
    callMethod('git:removeWorktree', { projectPath, worktreePath, force })
}

/** How often a run that is only producing output says so. */
const LOG_ONLY_INTERVAL_MS = 3_000

const lastPublished = new Map<string, { at: number; shape: string }>()

/** Which steps a run is on, which is what a viewer redraws for. */
function shapeOf(execution: WorkflowExecution): string {
  return `${execution.status}|${execution.nodeStates.map((ns) => `${ns.nodeId}:${ns.status}`).join(',')}`
}

/**
 * Run progress, sent to whoever is watching.
 *
 * Nothing carried run state before this: a second window learned what a run was
 * doing by re-reading the database. With the run happening here and the windows
 * watching, that is the wrong way round.
 *
 * A step moving goes out at once. Output alone goes out at most every few
 * seconds, because it arrives a chunk at a time and the run carries its logs --
 * a chatty agent would otherwise push a hundred kilobytes down a phone's socket
 * on every line it printed.
 */
export function publishRun(execution: WorkflowExecution): void {
  const shape = shapeOf(execution)
  const previous = lastPublished.get(execution.runId)
  const now = Date.now()

  if (previous && previous.shape === shape && now - previous.at < LOG_ONLY_INTERVAL_MS) return

  if (execution.status === 'running') lastPublished.set(execution.runId, { at: now, shape })
  else lastPublished.delete(execution.runId)

  clientRegistry.broadcast(IPC.WORKFLOW_RUN_UPDATED, execution)
}

/** The configuration the engine reads: projects, tasks, workflows, defaults. */
export function config(): AppConfig | null {
  try {
    return configManager.loadConfig()
  } catch (err) {
    log.warn({ err }, '[workflow] could not read the configuration')
    return null
  }
}

/** A run this process is not holding — a gate answered after a restart. */
export function runById(runId: string): WorkflowExecution | undefined {
  return getWorkflowRun(runId) ?? undefined
}

/** Sessions as the server knows them, for backfilling ids onto finished steps. */
export function activeTerminals(): TerminalSession[] {
  return ptyManager.getActiveSessions()
}

export function activeHeadless(): HeadlessSession[] {
  return headlessManager.getActiveSessions()
}

/** The next queued task for a project, which a launchAgent step can be pointed at. */
export function nextTask(projectName: string): TaskConfig | undefined {
  return (config()?.tasks ?? [])
    .filter((t) => t.projectName === projectName && t.status === 'todo')
    .sort((a, b) => a.order - b.order)[0]
}

/** Streams the engine consumes. In-process now: the emitters are right here. */
export function onHeadlessData(cb: (event: { id: string; data: string }) => void): () => void {
  const listener = (channel: string, payload: unknown): void => {
    if (channel === IPC.HEADLESS_DATA) cb(payload as { id: string; data: string })
  }
  headlessManager.on('client-message', listener)
  return () => headlessManager.off('client-message', listener)
}

export function onHeadlessExit(cb: (event: { id: string; exitCode: number }) => void): () => void {
  const listener = (channel: string, payload: unknown): void => {
    if (channel === IPC.HEADLESS_EXIT) cb(payload as { id: string; exitCode: number })
  }
  headlessManager.on('client-message', listener)
  return () => headlessManager.off('client-message', listener)
}

export function onScriptData(cb: (event: { runId: string; data: string }) => void): () => void {
  const listener = (payload: { runId: string; data: string }): void => cb(payload)
  scriptRunnerEvents.on(IPC.SCRIPT_DATA, listener)
  return () => scriptRunnerEvents.off(IPC.SCRIPT_DATA, listener)
}
