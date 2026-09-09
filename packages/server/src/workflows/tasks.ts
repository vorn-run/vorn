import type { AiAgentType, TaskConfig, TaskStatus } from '@vornrun/shared/types'
import { configManager } from '../config-manager'
import { fireTaskStatusChangedTrigger } from './triggers'

/**
 * The two task writes a running workflow makes.
 *
 * They lived in the renderer's task store, which saved the whole configuration
 * and then fired the status-changed trigger. Both halves matter and both are
 * the server's now: a step that picks up a task marks it in progress, and a
 * step that fails hands it back to the queue -- and either can be the thing
 * that starts another workflow.
 */
function writeTask(
  id: string,
  change: (task: TaskConfig) => TaskConfig
): { task: TaskConfig; from: TaskStatus } | undefined {
  const config = configManager.loadConfig()
  let before: TaskStatus | undefined
  let after: TaskConfig | undefined

  const tasks = (config.tasks ?? []).map((t) => {
    if (t.id !== id) return t
    before = t.status
    after = change(t)
    return after
  })
  if (!after || !before) return undefined

  configManager.saveConfig({ ...config, tasks })
  // Saving writes; telling everyone is a separate step, and the one that makes
  // the board move. The renderer's `config:save` method does both, and this
  // path does not go through it.
  configManager.notifyChanged()
  return { task: after, from: before }
}

/** A step took this task on. */
export function startTask(
  id: string,
  sessionId: string,
  agentType: AiAgentType,
  worktreePath?: string
): void {
  const now = new Date().toISOString()
  const written = writeTask(id, (task) => ({
    ...task,
    status: 'in_progress',
    assignedSessionId: sessionId,
    assignedAgent: agentType,
    worktreePath: worktreePath || task.worktreePath,
    updatedAt: now,
    archivedAt: undefined
  }))
  if (written && written.from !== 'in_progress') {
    fireTaskStatusChangedTrigger(written.task, written.from, 'in_progress')
  }
}

/** The step that held it failed or timed out, so it goes back to the queue. */
export function reopenTask(id: string): void {
  const now = new Date().toISOString()
  const written = writeTask(id, (task) => ({
    ...task,
    status: 'todo',
    updatedAt: now,
    completedAt: undefined,
    archivedAt: undefined,
    assignedSessionId: undefined,
    assignedAgent: undefined
  }))
  if (written && written.from !== 'todo') {
    fireTaskStatusChangedTrigger(written.task, written.from, 'todo')
  }
}
