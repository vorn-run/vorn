import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { WorkflowExecution } from '../packages/shared/src/types'

/**
 * A gate answered somewhere else.
 *
 * The decision is broadcast to every connected client, because whoever answered
 * is usually not the instance holding the run — from a phone it never is. So
 * every instance receives it and all but one must do nothing. That is the part
 * worth pinning: a second instance acting would run the branch below the gate
 * a second time.
 */

const executions = new Map<string, WorkflowExecution>()
const saved: WorkflowExecution[] = []

// The module imports from the barrel, not the slice. Mocking the slice looks
// right and applies to nothing, which makes every no-op assertion pass for the
// wrong reason: the real store is empty, so no run is ever found.
vi.mock('../src/renderer/stores', () => ({
  useAppStore: {
    getState: () => ({
      workflowExecutions: executions,
      config: { workflows: [{ id: 'wf-1', name: 'W', nodes: [], edges: [] }] },
      setWorkflowExecution: (_runId: string, execution: WorkflowExecution) => saved.push(execution)
    })
  }
}))

vi.mock('../src/renderer/lib/notifications', () => ({
  notifyWorkflowGate: vi.fn(),
  notifyAgentStatus: vi.fn()
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

let applyGateDecision: typeof import('../src/renderer/lib/workflow-execution').applyGateDecision

beforeEach(async () => {
  // This module runs in a renderer. Only the two calls this path makes are
  // stubbed; anything else it reaches for should fail loudly rather than be
  // quietly satisfied by a permissive stub.
  vi.stubGlobal('window', {
    api: {
      saveWorkflowRun: vi.fn(async () => {}),
      launchHeadlessAgent: vi.fn(async () => ({}))
    }
  })
  executions.clear()
  saved.length = 0
  vi.resetModules()
  ;({ applyGateDecision } = await import('../src/renderer/lib/workflow-execution'))
})

describe('acting on a gate decision from another client', () => {
  it('does nothing for a run this instance has never heard of', async () => {
    // The ordinary case: the broadcast reaches every window, and only one of
    // them — sometimes none — is holding the run.
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
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    executions.set('run-1', parkedRun('success'))

    await applyGateDecision('run-1', 'gate-1', 'approve')

    expect(warn).not.toHaveBeenCalled()
    warn.mockRestore()
  })

  it('does nothing for a node id the run does not contain', async () => {
    executions.set('run-1', parkedRun('waiting'))

    await applyGateDecision('run-1', 'a-node-from-another-workflow', 'approve')

    expect(saved).toHaveLength(0)
  })

  it('acts when this instance is holding a run parked on that gate', async () => {
    executions.set('run-1', parkedRun('waiting'))

    // Resumption is deliberately allowed to fail here. Approving records the
    // decision and then hands the run back to the execution engine, which wants
    // a whole renderer around it; stubbing enough of one to let it proceed would
    // make this a test of the engine rather than of the decision. What matters
    // is that the decision was taken and written down.
    await applyGateDecision('run-1', 'gate-1', 'approve').catch(() => {})
    expect(saved.length).toBeGreaterThan(0)
    expect(saved[0]?.nodeStates.find((n) => n.nodeId === 'gate-1')?.status).toBe('success')
  })
})
