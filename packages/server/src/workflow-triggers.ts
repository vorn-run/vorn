import type { TriggerConfig, WorkflowDefinition } from '@vornrun/shared/types'

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
const VORND_GROUPS = new Set([
  'workflow',
  'workflowRun',
  'scheduler',
  'webhook',
  'artifact',
  'config',
  'task',
  'project',
  'sessionEvent',
  'widget',
  'core',
  'env',
  'ssh',
  'script',
  'credential',
  'permission',
  'git',
  'file',
  'worktree',
  'agent',
  'ide',
  'token',
  'pairing',
  'tailscale'
])

/** Calls vornd answers in a group the server still has others of. */
const VORND_METHODS = new Set(['sessions:getRecent'])

/** How long vornd gets to answer one: publishing may wait on the pane opening. */
const WORK_CALL_TIMEOUT_MS = 60_000

/** How long a call waits for vornd's channel while vornd starts. */
const CHANNEL_WAIT_MS = 10_000
const CHANNEL_RETRY_MS = 100

/**
 * A client connected to this server, not vornd, still reaches what vornd
 * answers: its calls are handed to vornd on the channel, and vornd's answer returned.
 * A call made while vornd is still starting waits for its channel a while.
 */
export function relayVorndCall(
  channel: {
    ask<T>(method: string, params: unknown, timeoutMs?: number): Promise<T | null>
  },
  waitMs = CHANNEL_WAIT_MS
): (method: string, params: unknown) => Promise<unknown> | undefined {
  return (method, params) => {
    if (!VORND_GROUPS.has(method.split(':')[0]) && !VORND_METHODS.has(method)) return undefined
    const deadline = Date.now() + waitMs
    const attempt = async (): Promise<unknown> => {
      for (;;) {
        const answer = await channel.ask<{ result?: unknown }>(
          'vornd:work',
          { method, params },
          WORK_CALL_TIMEOUT_MS
        )
        if (answer !== null) return answer.result
        if (Date.now() >= deadline) {
          throw new Error(`vornd is not running, so ${method} cannot be answered`)
        }
        await new Promise((r) => setTimeout(r, CHANNEL_RETRY_MS))
      }
    }
    return attempt()
  }
}
