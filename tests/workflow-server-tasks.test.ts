import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { AppConfig, TaskConfig } from '../packages/shared/src/types'

const saved: AppConfig[] = []
const config = {
  version: 1,
  defaults: {},
  projects: [],
  workflows: [],
  tasks: [] as TaskConfig[]
} as unknown as AppConfig

const notifyChanged = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: {
    loadConfig: () => config,
    saveConfig: (next: AppConfig) => {
      saved.push(next)
      config.tasks = next.tasks ?? []
    },
    notifyChanged
  }
}))

const fireTaskStatusChangedTrigger = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/workflows/triggers', () => ({ fireTaskStatusChangedTrigger }))

import { reopenTask, startTask } from '../packages/server/src/workflows/tasks'

/**
 * The two task writes a running workflow makes.
 *
 * They were in the renderer's task store, which saved the configuration and
 * then fired the status-changed trigger. Both halves are the server's now, and
 * the second is the one worth pinning: a task moved by a step is exactly the
 * kind of change another workflow is waiting for.
 */
const task = (over: Partial<TaskConfig> = {}): TaskConfig =>
  ({
    id: 't1',
    title: 'Fix it',
    projectName: 'vorn',
    status: 'todo',
    order: 0,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    ...over
  }) as TaskConfig

beforeEach(() => {
  saved.length = 0
  config.tasks = [task()]
  fireTaskStatusChangedTrigger.mockClear()
  notifyChanged.mockClear()
})

describe('a step taking a task on', () => {
  it('marks it in progress, with the session and agent that took it', () => {
    startTask('t1', 'sess-1', 'claude', '/worktree')

    const written = saved.at(-1)?.tasks?.[0]
    expect(written).toMatchObject({
      status: 'in_progress',
      assignedSessionId: 'sess-1',
      assignedAgent: 'claude',
      worktreePath: '/worktree'
    })
  })

  it('tells everyone, so the board moves while the step holds the task', () => {
    startTask('t1', 'sess-1', 'claude')

    expect(notifyChanged).toHaveBeenCalled()
  })

  it('fires the status-changed trigger, because another workflow may be waiting', () => {
    startTask('t1', 'sess-1', 'claude')

    expect(fireTaskStatusChangedTrigger).toHaveBeenCalledWith(
      expect.objectContaining({ id: 't1', status: 'in_progress' }),
      'todo',
      'in_progress'
    )
  })

  it('says nothing when the task was already in progress', () => {
    config.tasks = [task({ status: 'in_progress' })]

    startTask('t1', 'sess-2', 'codex')

    expect(fireTaskStatusChangedTrigger).not.toHaveBeenCalled()
  })

  it('does nothing at all for a task that is not there', () => {
    startTask('t-gone', 'sess-1', 'claude')

    expect(saved).toHaveLength(0)
    expect(fireTaskStatusChangedTrigger).not.toHaveBeenCalled()
  })
})

describe('a step handing a task back', () => {
  it('returns it to the queue, unassigned', () => {
    config.tasks = [
      task({ status: 'in_progress', assignedSessionId: 's', assignedAgent: 'claude' })
    ]

    reopenTask('t1')

    expect(saved.at(-1)?.tasks?.[0]).toMatchObject({
      status: 'todo',
      assignedSessionId: undefined,
      assignedAgent: undefined
    })
    expect(fireTaskStatusChangedTrigger).toHaveBeenCalledWith(
      expect.objectContaining({ id: 't1' }),
      'in_progress',
      'todo'
    )
  })

  it('says nothing when it was already back in the queue', () => {
    reopenTask('t1')
    expect(fireTaskStatusChangedTrigger).not.toHaveBeenCalled()
  })
})
