import {
  AppConfig,
  WorkflowDefinition,
  TriggerConfig,
  TaskConfig,
  TaskStatus,
  TerminalSession,
  RestoreEnvironment,
  WorkflowExecutionContext
} from '@vornrun/shared/types'
import { executeWorkflow } from './engine'
import { config } from './host'
import log from '../logger'

function getTriggerConfig(wf: WorkflowDefinition): TriggerConfig | null {
  const triggerNode = wf.nodes.find((n) => n.type === 'trigger')
  if (!triggerNode) return null
  return triggerNode.config as TriggerConfig
}

/**
 * Called after a task is created. Fires any workflows with a `taskCreated` trigger
 * that match the task's project.
 */
export function fireTaskCreatedTrigger(task: TaskConfig): void {
  const workflows = (config()?.workflows || []).filter((wf) => wf.enabled)

  for (const wf of workflows) {
    const trigger = getTriggerConfig(wf)
    if (!trigger || trigger.triggerType !== 'taskCreated') continue
    if (trigger.projectFilter && trigger.projectFilter !== task.projectName) continue

    log.info({ workflow: wf.name, task: task.title }, '[triggers] task created')
    executeWorkflow(wf, {
      task,
      trigger: { type: 'taskCreated' }
    }).catch((err) => log.error({ err, workflow: wf.name }, '[triggers] workflow failed to start'))
  }
}

/**
 * Called after a task's status changes. Fires any workflows with a `taskStatusChanged` trigger
 * that match the transition and project.
 */
export function fireTaskStatusChangedTrigger(
  task: TaskConfig,
  fromStatus: TaskStatus,
  toStatus: TaskStatus
): void {
  if (fromStatus === toStatus) return

  const workflows = (config()?.workflows || []).filter((wf) => wf.enabled)

  for (const wf of workflows) {
    const trigger = getTriggerConfig(wf)
    if (!trigger || trigger.triggerType !== 'taskStatusChanged') continue
    if (trigger.projectFilter && trigger.projectFilter !== task.projectName) continue
    if (trigger.fromStatus && trigger.fromStatus !== fromStatus) continue
    if (trigger.toStatus && trigger.toStatus !== toStatus) continue

    const context: WorkflowExecutionContext = {
      task,
      trigger: { type: 'taskStatusChanged', fromStatus, toStatus }
    }
    executeWorkflow(wf, context).catch((err) =>
      log.error({ err, workflow: wf.name }, '[triggers] workflow failed to start')
    )
  }
}

/** One chain per project. Serialized because a dev server has one port. */
const restoreQueues = new Map<string, Promise<unknown>>()

/**
 * Called after a session comes back. Cold: the server started its process
 * again. Warm: this client attached to one the server never stopped.
 *
 * The trigger reports which; the workflow chooses. A workflow set to cold, the
 * default, hears nothing about a phone opening a live card.
 */
export function fireSessionRestoredTrigger(
  session: TerminalSession,
  how: { restore: 'cold' | 'warm'; environment?: RestoreEnvironment }
): void {
  const workflows = (config()?.workflows || []).filter((wf) => wf.enabled)

  for (const wf of workflows) {
    const trigger = getTriggerConfig(wf)
    if (!trigger || trigger.triggerType !== 'sessionRestored') continue
    if (trigger.projectFilter && trigger.projectFilter !== session.projectName) continue
    if ((trigger.restore ?? 'cold') === 'cold' && how.restore === 'warm') continue

    const context: WorkflowExecutionContext = {
      source: session,
      trigger: { type: 'sessionRestored', restore: how.restore, environment: how.environment }
    }
    const start = (): Promise<unknown> =>
      executeWorkflow(wf, context).catch((err) =>
        log.error({ err, workflow: wf.name }, '[triggers] workflow failed to start')
      )
    if (trigger.concurrency === 'unbounded') {
      void start()
      continue
    }
    const key = session.projectName
    const next = (restoreQueues.get(key) ?? Promise.resolve()).then(start)
    restoreQueues.set(key, next)
  }
}

/** Test-only. */
export function resetRestoreQueues(): void {
  restoreQueues.clear()
}

/**
 * Fire the task triggers for a configuration that just replaced another.
 *
 * Task writes arrive as a whole configuration -- from this app, a phone or an
 * agent -- so the change has to be read out of the diff rather than reported by
 * whoever made it. It used to be reported: the renderer's task store called
 * these directly, which is why moving a card on a phone fired nothing.
 */
export function fireTaskTriggersForChange(before: AppConfig, after: AppConfig): void {
  const previous = new Map((before.tasks ?? []).map((t) => [t.id, t]))

  for (const task of after.tasks ?? []) {
    const prior = previous.get(task.id)
    if (!prior) {
      fireTaskCreatedTrigger(task)
      continue
    }
    if (prior.status === task.status) continue
    // Only a change that is newer than what is stored. Every client posts the
    // whole configuration, so one that has not yet caught up with a status a
    // step just set would otherwise read as a move back -- and start the
    // workflows watching for that move.
    if (task.updatedAt < prior.updatedAt) continue
    fireTaskStatusChangedTrigger(task, prior.status, task.status)
  }
}
