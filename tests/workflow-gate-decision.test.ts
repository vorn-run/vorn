import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { WorkflowDefinition, WorkflowExecution } from '../packages/shared/src/types'

/**
 * A gate answered from somewhere else.
 *
 * The answer arrives from a phone, an agent or another window, and lands on the
 * server holding the run. What is worth pinning is what it refuses: a run it
 * has never heard of, a node that already resolved, a node the run does not
 * contain. Acting on any of those would run the branch below the gate twice.
 */

const executions = new Map<string, WorkflowExecution>()
const published: WorkflowExecution[] = []
const reimported: WorkflowDefinition[] = []

vi.mock('../packages/server/src/workflows/host', () => ({
  api: {
    saveWorkflowRun: vi.fn(async () => {}),
    releaseWorkflowRun: vi.fn(async () => {}),
    reportWorkflowComplete: vi.fn(async () => {})
  },
  config: () => ({ workflows: [{ id: 'wf-1', name: 'W', nodes: [], edges: [] }, ...reimported] }),
  publishRun: (execution: WorkflowExecution) => published.push(execution),
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
  published.length = 0
  vi.resetModules()
  ;({ applyGateDecision } = await import('../packages/server/src/workflows/engine'))
})

describe('acting on a gate decision from another client', () => {
  it('does nothing for a run this instance has never heard of', async () => {
    // The ordinary case for a stale client: the run is finished and gone.
    await applyGateDecision('run-nobody-has', 'gate-1', 'approve')

    expect(published).toHaveLength(0)
  })

  it('does nothing for a node that has already resolved', async () => {
    // A duplicate broadcast, or two people answering at once. Approving a gate
    // that is no longer waiting would resume the branch below it twice. The run
    // still goes back out: whoever answered is showing a pill for a gate that
    // is over, and this is what clears it.
    executions.set('run-1', parkedRun('success'))

    await applyGateDecision('run-1', 'gate-1', 'approve')

    expect(published).toHaveLength(1)
    expect(published[0]?.nodeStates.find((n) => n.nodeId === 'gate-1')?.status).toBe('success')
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

    expect(published).toHaveLength(1)
    expect(published[0]?.nodeStates.find((n) => n.nodeId === 'gate-1')?.status).toBe('waiting')
  })

  it('acts when this instance is holding a run parked on that gate', async () => {
    executions.set('run-1', parkedRun('waiting'))

    // Resumption is deliberately allowed to fail here. Approving records the
    // decision and then hands the run back to the engine, which wants the rest
    // of the server around it; stubbing enough of one to let it proceed would
    // make this a test of the engine rather than of the decision. What matters
    // is that the decision was taken and written down.
    await applyGateDecision('run-1', 'gate-1', 'approve').catch(() => {})
    expect(published.length).toBeGreaterThan(0)
    expect(published[0]?.nodeStates.find((n) => n.nodeId === 'gate-1')?.status).toBe('success')
  })
})

describe('a gate decided after its workflow was re-imported', () => {
  const node = (id: string, type: string): WorkflowDefinition['nodes'][number] =>
    ({ id, type, label: id, config: {}, position: { x: 0, y: 0 } }) as never

  // The run started on trigger → gate; the re-import added a step after the gate.
  const started = {
    id: 'wf-2',
    name: 'Notes',
    nodes: [node('t', 'trigger'), node('gate', 'approval')],
    edges: [{ id: 'e1', source: 't', target: 'gate' }]
  } as WorkflowDefinition
  const current = {
    ...started,
    nodes: [...started.nodes, node('late', 'script')],
    edges: [...started.edges, { id: 'e2', source: 'gate', target: 'late' }]
  } as WorkflowDefinition

  function parkedOnGate(definition?: WorkflowDefinition): WorkflowExecution {
    return {
      runId: 'run-2',
      workflowId: 'wf-2',
      startedAt: new Date(0).toISOString(),
      status: 'running',
      nodeStates: [
        { nodeId: 't', status: 'success' },
        { nodeId: 'gate', status: 'waiting' }
      ],
      ...(definition && { definition })
    }
  }

  beforeEach(() => {
    reimported.length = 0
    reimported.push(current)
  })

  it.each([
    ['approve', 'success'],
    ['reject', 'error']
  ] as const)('finishes on its own definition when the gate is %s-ed', async (decision, status) => {
    const run = parkedOnGate(started)
    executions.set('run-2', run)

    await applyGateDecision('run-2', 'gate', decision)

    expect(run.status).toBe(status)
    expect(run.nodeStates.map((ns) => ns.nodeId)).toEqual(['t', 'gate'])
  })

  // A run saved before snapshots resumes on the current definition, whose new step it holds
  // no state for; that step used to be ready again on every wave, for ever.
  it('ends a run holding no state for a step instead of re-running it', async () => {
    const run = parkedOnGate()
    executions.set('run-2', run)

    await applyGateDecision('run-2', 'gate', 'approve')

    expect(run.status).toBe('error')
    expect(run.nodeStates.find((ns) => ns.nodeId === 'late')?.error).toMatch(/no state/)
  })
})
