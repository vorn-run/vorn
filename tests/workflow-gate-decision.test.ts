import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { WorkflowExecution } from '../packages/shared/src/types'

/**
 * A gate answered from somewhere else.
 *
 * The answer arrives from a phone, an agent or another window, and lands on the
 * server holding the run. What is worth pinning is what it refuses: a run it
 * has never heard of, a node that already resolved, a node the run does not
 * contain. Acting on any of those would run the branch below the gate twice.
 */

const executions = new Map<string, WorkflowExecution>()
const saved: WorkflowExecution[] = []

vi.mock('../packages/server/src/workflows/host', () => ({
  api: { saveWorkflowRun: vi.fn(async () => {}) },
  config: () => ({ workflows: [{ id: 'wf-1', name: 'W', nodes: [], edges: [] }] }),
  publishRun: (execution: WorkflowExecution) => saved.push(execution),
  runById: (runId: string) => executions.get(runId),
  activeTerminals: () => [],
  activeHeadless: () => [],
  nextTask: () => undefined,
  onHeadlessData: () => () => {},
  onHeadlessExit: () => () => {},
  onScriptData: () => () => {}
}))

vi.mock('../packages/server/src/workflows/tasks', () => ({
  startTask: vi.fn(),
  reopenTask: vi.fn()
}))

vi.mock('../packages/server/src/database', () => ({
  listWorkflowRuns: () => [],
  getWorkflowRun: () => null
}))

const warned = vi.hoisted(() => vi.fn())
vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: warned, error: vi.fn(), debug: vi.fn() }
}))

function parkedRun(
  nodeStatus: WorkflowExecution['nodeStates'][number]['status']
): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: new Date(0).toISOString(),
    status: 'running',
    nodeStates: [{ nodeId: 'gate-1', status: nodeStatus }]
  } as WorkflowExecution
}

let applyGateDecision: typeof import('../packages/server/src/workflows/engine').applyGateDecision

afterEach(() => vi.unstubAllGlobals())

beforeEach(async () => {
  executions.clear()
  warned.mockClear()
  saved.length = 0
  vi.resetModules()
  ;({ applyGateDecision } = await import('../packages/server/src/workflows/engine'))
})

describe('acting on a gate decision from another client', () => {
  it('does nothing for a run this instance has never heard of', async () => {
    // The ordinary case for a stale client: the run is finished and gone.
    await applyGateDecision('run-nobody-has', 'gate-1', 'approve')

    expect(saved).toHaveLength(0)
  })

  it('does nothing for a node that has already resolved', async () => {
    // A duplicate broadcast, or two people answering at once. Approving a gate
    // that is no longer waiting would resume the branch below it twice.
    executions.set('run-1', parkedRun('success'))

    await applyGateDecision('run-1', 'gate-1', 'approve')

    expect(saved).toHaveLength(0)
  })

  it('drops a duplicate quietly, rather than through the layer below', async () => {
    // The gate resolver underneath also refuses a node that is not waiting, so
    // the outcome is the same either way — but it warns when it does, and a
    // broadcast reaching every window would put that warning in every console
    // on every duplicate. Checking for silence is what distinguishes the two.
    executions.set('run-1', parkedRun('success'))

    await applyGateDecision('run-1', 'gate-1', 'approve')

    expect(warned).not.toHaveBeenCalled()
  })

  it('does nothing for a node id the run does not contain', async () => {
    executions.set('run-1', parkedRun('waiting'))

    await applyGateDecision('run-1', 'a-node-from-another-workflow', 'approve')

    expect(saved).toHaveLength(0)
  })

  it('acts when this instance is holding a run parked on that gate', async () => {
    executions.set('run-1', parkedRun('waiting'))

    // Resumption is deliberately allowed to fail here. Approving records the
    // decision and then hands the run back to the engine, which wants the rest
    // of the server around it; stubbing enough of one to let it proceed would
    // make this a test of the engine rather than of the decision. What matters
    // is that the decision was taken and written down.
    await applyGateDecision('run-1', 'gate-1', 'approve').catch(() => {})
    expect(saved.length).toBeGreaterThan(0)
    expect(saved[0]?.nodeStates.find((n) => n.nodeId === 'gate-1')?.status).toBe('success')
  })
})
