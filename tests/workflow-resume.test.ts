import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../packages/shared/src/types'

const waiting: WorkflowExecution[] = []
const running: WorkflowExecution[] = []
vi.mock('../packages/server/src/database', () => ({
  listRunsWithWaitingGates: () => waiting,
  listRunningRuns: () => running
}))

const workflows: WorkflowDefinition[] = []
vi.mock('../packages/server/src/config-manager', () => ({
  configManager: { loadConfig: () => ({ workflows }) }
}))

const rescheduleWaitingGateTimers = vi.hoisted(() => vi.fn())
const reconcileRunningExecutions = vi.hoisted(() => vi.fn(async () => {}))
vi.mock('../packages/server/src/workflows/engine', () => ({
  rescheduleWaitingGateTimers,
  reconcileRunningExecutions
}))

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import { resumeRunsAfterStart } from '../packages/server/src/workflows/resume'

/**
 * What the last server left behind.
 *
 * Both of these used to happen on every window open, because the runs lived in
 * the window. Neither survives a process, and the process they belong to is the
 * server now: without this a restart leaves a gate that can never time out and
 * a finished run that says it is still going.
 */
const run = (over: Partial<WorkflowExecution> = {}): WorkflowExecution =>
  ({
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-09-09T00:00:00Z',
    status: 'running',
    nodeStates: [],
    ...over
  }) as WorkflowExecution

beforeEach(() => {
  waiting.length = 0
  running.length = 0
  workflows.length = 0
  workflows.push({ id: 'wf-1', name: 'W' } as WorkflowDefinition)
  rescheduleWaitingGateTimers.mockClear()
  reconcileRunningExecutions.mockClear()
})

describe('picking runs back up at startup', () => {
  it('re-arms a gate that was left waiting', async () => {
    waiting.push(run({ nodeStates: [{ nodeId: 'gate', status: 'waiting' }] }))

    await resumeRunsAfterStart()

    expect(rescheduleWaitingGateTimers).toHaveBeenCalledWith(waiting, workflows)
  })

  it('reconciles a run still marked running, whose agent may have exited', async () => {
    running.push(run())

    await resumeRunsAfterStart()

    expect(reconcileRunningExecutions).toHaveBeenCalledWith(running, workflows)
  })

  it('does nothing when there is nothing to pick up', async () => {
    await resumeRunsAfterStart()

    expect(rescheduleWaitingGateTimers).not.toHaveBeenCalled()
    expect(reconcileRunningExecutions).not.toHaveBeenCalled()
  })

  it('does not start before there is a configuration to read workflows from', async () => {
    workflows.length = 0
    running.push(run())

    await resumeRunsAfterStart()

    expect(reconcileRunningExecutions).not.toHaveBeenCalled()
  })

  it('survives a database that cannot answer, because startup must not fail on it', async () => {
    running.push(run())
    reconcileRunningExecutions.mockRejectedValueOnce(new Error('no table'))

    await expect(resumeRunsAfterStart()).resolves.toBeUndefined()
  })
})
