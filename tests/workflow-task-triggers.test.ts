import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { AppConfig, TaskConfig, WorkflowDefinition } from '../packages/shared/src/types'

const state = { config: { workflows: [] as WorkflowDefinition[] } }
vi.mock('../packages/server/src/workflows/host', () => ({ config: () => state.config }))

const executeWorkflow = vi.hoisted(() => vi.fn(async () => ({})))
vi.mock('../packages/server/src/workflows/engine', () => ({ executeWorkflow }))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { fireTaskTriggersForChange } from '../packages/server/src/workflows/triggers'

/**
 * Which workflows a task change starts.
 *
 * The change is read out of the configuration that was saved rather than
 * reported by whoever made it, because it arrives as a whole configuration —
 * from this app, a phone, or an agent. Reporting was the old way, and it is why
 * moving a card on a phone fired nothing.
 */
function workflow(config: Record<string, unknown>, id = 'wf-1'): WorkflowDefinition {
  return {
    id,
    name: 'On change',
    enabled: true,
    nodes: [{ id: 't', type: 'trigger', label: 'Trigger', config }],
    edges: []
  } as unknown as WorkflowDefinition
}

const task = (over: Partial<TaskConfig> = {}): TaskConfig =>
  ({
    id: 't1',
    title: 'Fix it',
    projectName: 'vorn',
    status: 'todo',
    order: 0,
    ...over
  }) as TaskConfig

const config = (tasks: TaskConfig[]): AppConfig => ({ tasks }) as unknown as AppConfig

beforeEach(() => {
  state.config.workflows = []
  executeWorkflow.mockClear()
})

describe('a task that appeared', () => {
  it('starts the workflows watching for one', () => {
    state.config.workflows = [workflow({ triggerType: 'taskCreated' })]

    fireTaskTriggersForChange(config([]), config([task()]))

    expect(executeWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1' }),
      expect.objectContaining({ trigger: { type: 'taskCreated' } })
    )
  })

  it('respects a project filter', () => {
    state.config.workflows = [
      workflow({ triggerType: 'taskCreated', projectFilter: 'somewhere-else' })
    ]

    fireTaskTriggersForChange(config([]), config([task()]))

    expect(executeWorkflow).not.toHaveBeenCalled()
  })
})

describe('a task that moved', () => {
  it('starts the workflows watching for that transition', () => {
    state.config.workflows = [
      workflow({ triggerType: 'taskStatusChanged', fromStatus: 'todo', toStatus: 'in_progress' })
    ]

    fireTaskTriggersForChange(config([task()]), config([task({ status: 'in_progress' })]))

    expect(executeWorkflow).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf-1' }),
      expect.objectContaining({
        trigger: { type: 'taskStatusChanged', fromStatus: 'todo', toStatus: 'in_progress' }
      })
    )
  })

  it('ignores a transition it was not asked about', () => {
    state.config.workflows = [workflow({ triggerType: 'taskStatusChanged', toStatus: 'done' })]

    fireTaskTriggersForChange(config([task()]), config([task({ status: 'in_progress' })]))

    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('says nothing when a save changed something other than status', () => {
    state.config.workflows = [workflow({ triggerType: 'taskStatusChanged' })]

    fireTaskTriggersForChange(config([task()]), config([task({ title: 'Renamed' })]))

    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('leaves a disabled workflow alone', () => {
    const disabled = workflow({ triggerType: 'taskCreated' })
    disabled.enabled = false
    state.config.workflows = [disabled]

    fireTaskTriggersForChange(config([]), config([task()]))

    expect(executeWorkflow).not.toHaveBeenCalled()
  })
})
