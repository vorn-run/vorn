import { describe, it, expect, vi } from 'vitest'
import type { AppConfig, TaskConfig, WorkflowDefinition } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  armedScheduleCount,
  createTriggerOutbox,
  taskTriggersForChange,
  type TaskTrigger
} from '../packages/server/src/workflow-triggers'

/**
 * The task triggers this server reads out of a saved configuration and hands
 * to vornd, which decides which workflows they start.
 *
 * The change is read out of the configuration rather than reported by whoever
 * made it, because it arrives whole, from this app, a phone or an agent. Each
 * trigger names the change in its effect id, so vornd starts its workflows once
 * however often it is sent.
 */

const task = (over: Partial<TaskConfig> = {}): TaskConfig =>
  ({
    id: 't1',
    title: 'Fix it',
    projectName: 'vorn',
    status: 'todo',
    order: 0,
    updatedAt: '2026-09-09T10:00:00Z',
    ...over
  }) as TaskConfig

const config = (tasks: TaskConfig[]): AppConfig => ({ tasks }) as unknown as AppConfig

describe('what a save fires', () => {
  it('a task that appeared', () => {
    expect(taskTriggersForChange(config([]), config([task()]))).toEqual([
      { effectId: 'task-created/t1', kind: 'taskCreated', task: task() }
    ])
  })

  it('a task that moved, named by its move', () => {
    const moved = task({ status: 'in_progress', updatedAt: '2026-09-09T10:05:00Z' })
    expect(taskTriggersForChange(config([task()]), config([moved]))).toEqual([
      {
        effectId: 'task-status/t1/todo/in_progress/2026-09-09T10:05:00Z',
        kind: 'taskStatusChanged',
        task: moved,
        from: 'todo',
        to: 'in_progress'
      }
    ])
  })

  it('nothing when a save changed something other than status', () => {
    expect(taskTriggersForChange(config([task()]), config([task({ title: 'Renamed' })]))).toEqual(
      []
    )
  })

  it('nothing for a client that has not caught up with a status a step just set', () => {
    expect(
      taskTriggersForChange(
        config([task({ status: 'in_progress', updatedAt: '2026-09-09T10:05:00Z' })]),
        config([task({ status: 'todo', updatedAt: '2026-09-09T10:00:00Z' })])
      )
    ).toEqual([])
  })

  it('nothing for a configuration without tasks', () => {
    expect(taskTriggersForChange({} as AppConfig, {} as AppConfig)).toEqual([])
  })
})

/** A channel to a vornd that takes what it is asked, or is not there. */
function channel(answers: Array<'take' | 'absent' | 'refuse'>) {
  const asked: TaskTrigger[] = []
  let subscribed: () => void = () => {}
  return {
    asked,
    resubscribe: () => subscribed(),
    ask: vi.fn(async (_method: string, params: unknown) => {
      asked.push(params as TaskTrigger)
      const answer = answers.shift() ?? 'take'
      if (answer === 'refuse') throw new Error('vornd refused')
      return answer === 'take' ? { received: true } : null
    }) as unknown as <T>(method: string, params: unknown) => Promise<T | null>,
    on: (_event: 'subscribed', listener: () => void) => {
      subscribed = listener
    }
  }
}

const trigger = (id: string): TaskTrigger => ({ effectId: id, kind: 'taskCreated', task: task() })

describe('sending triggers to vornd', () => {
  it('sends each until vornd takes it, in order', async () => {
    const c = channel(['take', 'take'])
    const outbox = createTriggerOutbox(c, 5)
    await outbox.deliver([trigger('a'), trigger('b')])
    expect(c.asked.map((t) => t.effectId)).toEqual(['a', 'b'])
    expect(vi.mocked(c.ask)).toHaveBeenCalledWith('vornd:trigger', trigger('a'))
    expect(outbox.held).toBe(0)
  })

  it('holds what vornd could not take, and sends it again', async () => {
    const c = channel(['absent', 'refuse'])
    const outbox = createTriggerOutbox(c, 5)
    await outbox.deliver([trigger('a')])
    expect(outbox.held).toBe(1)
    await vi.waitFor(() => expect(outbox.held).toBe(0))
    expect(c.asked.map((t) => t.effectId)).toEqual(['a', 'a', 'a'])
  })

  it('sends what it holds as soon as vornd subscribes again', async () => {
    const c = channel(['absent'])
    const outbox = createTriggerOutbox(c, 60_000)
    await outbox.deliver([trigger('a')])
    expect(outbox.held).toBe(1)
    c.resubscribe()
    await vi.waitFor(() => expect(outbox.held).toBe(0))
  })

  it('keeps the newest when vornd is away for long', async () => {
    const c = channel(Array(600).fill('absent'))
    const outbox = createTriggerOutbox(c, 60_000)
    await outbox.deliver(Array.from({ length: 501 }, (_, i) => trigger(`t${i}`)))
    expect(outbox.held).toBe(500)
  })
})

describe('the schedules that keep an idle server up', () => {
  const wf = (config: Record<string, unknown>, enabled = true): WorkflowDefinition =>
    ({
      id: 'w',
      name: 'w',
      enabled,
      nodes: [{ id: 't', type: 'trigger', config }],
      edges: []
    }) as unknown as WorkflowDefinition

  it('counts enabled crons, polls and once triggers still ahead', () => {
    const now = Date.parse('2030-01-01T00:00:00Z')
    expect(
      armedScheduleCount(
        [
          wf({ triggerType: 'recurring', cron: '* * * * *' }),
          wf({ triggerType: 'connectorPoll', cron: '*/5 * * * *' }),
          wf({ triggerType: 'once', runAt: '2030-01-02T00:00:00Z' }),
          wf({ triggerType: 'once', runAt: '2029-01-01T00:00:00Z' }),
          wf({ triggerType: 'recurring', cron: ' ' }),
          wf({ triggerType: 'recurring', cron: '* * * * *' }, false),
          wf({ triggerType: 'manual' })
        ],
        now
      )
    ).toBe(3)
  })
})
