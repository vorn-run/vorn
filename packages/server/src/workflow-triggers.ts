import type {
  AppConfig,
  TaskConfig,
  TriggerConfig,
  WorkflowDefinition
} from '@vornrun/shared/types'
import log from './logger'

/**
 * The workflow triggers this server sees, handed to vornd, which runs workflows.
 *
 * Task writes arrive here as whole configurations, from this app, a phone or an
 * agent, so a change is read out of the diff rather than reported by whoever
 * made it. Each trigger carries an effect id that names the change itself, and
 * is sent until vornd takes it: vornd receives each effect id once, so a
 * trigger sent twice starts its workflows once.
 */

export interface TaskTrigger {
  effectId: string
  kind: 'taskCreated' | 'taskStatusChanged'
  task: TaskConfig
  from?: string
  to?: string
}

/** The triggers a configuration that replaced another fires. */
export function taskTriggersForChange(before: AppConfig, after: AppConfig): TaskTrigger[] {
  const previous = new Map((before.tasks ?? []).map((t) => [t.id, t]))
  const triggers: TaskTrigger[] = []
  for (const task of after.tasks ?? []) {
    const prior = previous.get(task.id)
    if (!prior) {
      triggers.push({ effectId: `task-created/${task.id}`, kind: 'taskCreated', task })
      continue
    }
    if (prior.status === task.status) continue
    // A client that has not caught up with a status a step just set would
    // otherwise read as a move back.
    if (task.updatedAt < prior.updatedAt) continue
    triggers.push({
      effectId: `task-status/${task.id}/${prior.status}/${task.status}/${task.updatedAt}`,
      kind: 'taskStatusChanged',
      task,
      from: prior.status,
      to: task.status
    })
  }
  return triggers
}

export interface TriggerChannel {
  ask<T>(method: string, params: unknown): Promise<T | null>
  on(event: 'subscribed', listener: () => void): unknown
}

/** Triggers held for vornd at most; past this the oldest are dropped, and said so. */
const MAX_HELD = 500
const RETRY_MS = 2_000

/** Sends triggers to vornd until it takes each, again whenever it (re)subscribes. */
export function createTriggerOutbox(channel: TriggerChannel, retryMs = RETRY_MS) {
  const held: TaskTrigger[] = []
  let sending = false
  let timer: ReturnType<typeof setTimeout> | null = null

  const later = (): void => {
    if (timer || held.length === 0) return
    timer = setTimeout(() => {
      timer = null
      void flush()
    }, retryMs)
    timer.unref?.()
  }

  async function flush(): Promise<void> {
    if (sending) return
    sending = true
    try {
      while (held.length > 0) {
        const next = held[0]
        let taken: unknown = null
        try {
          taken = await channel.ask('vornd:trigger', next)
        } catch (err) {
          log.warn({ err, effectId: next.effectId }, '[workflow] vornd refused a trigger')
        }
        if (taken === null) break
        held.shift()
      }
    } finally {
      sending = false
      later()
    }
  }

  channel.on('subscribed', () => void flush())

  return {
    deliver(triggers: TaskTrigger[]): Promise<void> {
      held.push(...triggers)
      if (held.length > MAX_HELD) {
        const dropped = held.splice(0, held.length - MAX_HELD)
        log.warn(
          { dropped: dropped.length },
          '[workflow] vornd took no triggers; the oldest were dropped'
        )
      }
      return flush()
    },
    get held(): number {
      return held.length
    }
  }
}

/**
 * How many workflows vornd keeps a schedule armed for. vornd ends with this
 * server, so an idle server stays up while a schedule may still fire.
 */
export function armedScheduleCount(workflows: WorkflowDefinition[], now = Date.now()): number {
  return workflows.filter((wf) => {
    if (!wf.enabled) return false
    const trigger = wf.nodes.find((n) => n.type === 'trigger')?.config as TriggerConfig | undefined
    if (trigger?.triggerType === 'recurring' || trigger?.triggerType === 'connectorPoll') {
      return typeof trigger.cron === 'string' && trigger.cron.trim() !== ''
    }
    if (trigger?.triggerType === 'once') return new Date(trigger.runAt).getTime() > now
    return false
  }).length
}

/** The groups whose calls vornd answers, for clients connected to this server. */
const WORK_GROUPS = new Set(['workflow', 'workflowRun', 'scheduler', 'webhook', 'artifact'])

/** How long vornd gets to answer one: publishing may wait on the pane opening. */
const WORK_CALL_TIMEOUT_MS = 60_000

/**
 * A client connected to this server, not vornd, still reaches the work model:
 * its calls are handed to vornd on the channel, and vornd's answer returned.
 */
export function relayWorkCall(channel: {
  ask<T>(method: string, params: unknown, timeoutMs?: number): Promise<T | null>
}): (method: string, params: unknown) => Promise<unknown> | undefined {
  return (method, params) => {
    if (!WORK_GROUPS.has(method.split(':')[0])) return undefined
    return channel
      .ask<{ result?: unknown }>('vornd:work', { method, params }, WORK_CALL_TIMEOUT_MS)
      .then((answer) => {
        if (answer === null)
          throw new Error(`vornd is not running, so ${method} cannot be answered`)
        return answer.result
      })
  }
}
