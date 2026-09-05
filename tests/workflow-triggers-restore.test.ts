import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { TerminalSession, WorkflowDefinition } from '../packages/shared/src/types'

const executeWorkflow = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  executeWorkflow: (...args: unknown[]) => executeWorkflow(...args)
}))

const state = { config: { workflows: [] as WorkflowDefinition[] } }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: { getState: () => state }
}))

import {
  fireSessionRestoredTrigger,
  resetRestoreQueues
} from '../src/renderer/lib/workflow-triggers'

function workflow(
  config: Record<string, unknown>,
  over: Partial<WorkflowDefinition> = {}
): WorkflowDefinition {
  return {
    id: 'wf',
    name: 'Bring the dev server back',
    enabled: true,
    nodes: [{ id: 't', type: 'trigger', label: 'When a session is restored', config }],
    edges: [],
    ...over
  } as unknown as WorkflowDefinition
}

function session(over: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'a-session',
    agentType: 'claude',
    projectName: 'vorn',
    projectPath: '/dev/vorn',
    status: 'running',
    createdAt: 1,
    pid: 1,
    ...over
  } as TerminalSession
}

/** A run that finishes when told to, so ordering can be observed. */
function pendingRun(): { promise: Promise<void>; finish: () => void } {
  let finish!: () => void
  const promise = new Promise<void>((resolve) => {
    finish = resolve
  })
  return { promise, finish }
}

const tick = (): Promise<void> => new Promise((r) => setTimeout(r, 0))

beforeEach(() => {
  vi.restoreAllMocks()
  executeWorkflow.mockReset()
  executeWorkflow.mockResolvedValue(undefined)
  resetRestoreQueues()
  state.config.workflows = []
})

describe('a workflow that runs when a session comes back', () => {
  it('fires for a cold restore, carrying the session and what the reboot check found', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored' })]
    const environment = {
      worktree: 'ok' as const,
      branch: { recorded: 'main', actual: 'main' },
      head: { recorded: 'a', actual: 'a' }
    }
    fireSessionRestoredTrigger(session(), { restore: 'cold', environment })
    await tick()

    expect(executeWorkflow).toHaveBeenCalledTimes(1)
    const [, context] = executeWorkflow.mock.calls[0]
    expect(context.source.id).toBe('a-session')
    expect(context.trigger).toEqual({ type: 'sessionRestored', restore: 'cold', environment })
  })

  it('hears nothing about a warm attach unless it asked to', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored' })]
    fireSessionRestoredTrigger(session(), { restore: 'warm' })
    await tick()
    expect(executeWorkflow).not.toHaveBeenCalled()

    state.config.workflows = [workflow({ triggerType: 'sessionRestored', restore: 'any' })]
    fireSessionRestoredTrigger(session(), { restore: 'warm' })
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(1)
    expect(executeWorkflow.mock.calls[0][1].trigger.restore).toBe('warm')
  })

  it('respects the project filter', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored', projectFilter: 'other' })]
    fireSessionRestoredTrigger(session(), { restore: 'cold' })
    await tick()
    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('ignores disabled workflows and other trigger types', async () => {
    state.config.workflows = [
      workflow({ triggerType: 'sessionRestored' }, { enabled: false }),
      workflow({ triggerType: 'taskCreated' }, { id: 'other' })
    ]
    fireSessionRestoredTrigger(session(), { restore: 'cold' })
    await tick()
    expect(executeWorkflow).not.toHaveBeenCalled()
  })

  it('runs one at a time within a project, in the order the sessions came back', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored' })]
    const first = pendingRun()
    const second = pendingRun()
    executeWorkflow.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

    fireSessionRestoredTrigger(session({ id: 'one' }), { restore: 'cold' })
    fireSessionRestoredTrigger(session({ id: 'two' }), { restore: 'cold' })
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(1)
    expect(executeWorkflow.mock.calls[0][1].source.id).toBe('one')

    first.finish()
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(2)
    expect(executeWorkflow.mock.calls[1][1].source.id).toBe('two')
  })

  it('lets two projects run side by side', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored' })]
    executeWorkflow.mockReturnValue(pendingRun().promise)
    fireSessionRestoredTrigger(session({ id: 'one', projectName: 'vorn' }), { restore: 'cold' })
    fireSessionRestoredTrigger(session({ id: 'two', projectName: 'site' }), { restore: 'cold' })
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(2)
  })

  it('runs any number at once when the author says so', async () => {
    state.config.workflows = [
      workflow({ triggerType: 'sessionRestored', concurrency: 'unbounded' })
    ]
    executeWorkflow.mockReturnValue(pendingRun().promise)
    fireSessionRestoredTrigger(session({ id: 'one' }), { restore: 'cold' })
    fireSessionRestoredTrigger(session({ id: 'two' }), { restore: 'cold' })
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(2)
  })

  it('does not let a failed run block the next in the project', async () => {
    state.config.workflows = [workflow({ triggerType: 'sessionRestored' })]
    executeWorkflow.mockRejectedValueOnce(new Error('port in use')).mockResolvedValueOnce(undefined)
    vi.spyOn(console, 'error').mockImplementation(() => {})

    fireSessionRestoredTrigger(session({ id: 'one' }), { restore: 'cold' })
    fireSessionRestoredTrigger(session({ id: 'two' }), { restore: 'cold' })
    await tick()
    await tick()
    expect(executeWorkflow).toHaveBeenCalledTimes(2)
  })
})
