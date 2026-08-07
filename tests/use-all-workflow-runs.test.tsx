// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { renderHook, waitFor } from '@testing-library/react'
import type { WorkflowExecution } from '../src/shared/types'

const rescheduleMock = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  rescheduleWaitingGateTimers: (...args: unknown[]) => rescheduleMock(...args)
}))

const setWorkflowExecution = vi.fn((id: string, exec: WorkflowExecution) => {
  mockState.workflowExecutions.set(id, exec)
})

const mockState = {
  activeWorkspace: 'personal',
  workflowExecutions: new Map<string, WorkflowExecution>(),
  config: { workflows: [{ id: 'wf-a', name: 'Alpha', nodes: [], workspaceId: 'personal' }] },
  workflowsRunsReloadToken: 0,
  beginWorkflowsRunsLoad: vi.fn(),
  endWorkflowsRunsLoad: vi.fn(),
  setWorkflowExecution
}

const useAppStore = Object.assign(
  (selector?: (state: unknown) => unknown) => (selector ? selector(mockState) : mockState),
  { getState: () => mockState }
)

vi.mock('../src/renderer/stores', () => ({ useAppStore }))

const { useAllWorkflowRuns } = await import('../src/renderer/hooks/useAllWorkflowRuns')

function run(overrides: Partial<WorkflowExecution> = {}): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-a',
    startedAt: '2026-04-20T11:00:00Z',
    status: 'running',
    nodeStates: [],
    ...overrides
  } as WorkflowExecution
}

const listAll = vi.fn()
const listWaiting = vi.fn()

beforeEach(() => {
  mockState.workflowExecutions = new Map()
  rescheduleMock.mockReset()
  setWorkflowExecution.mockClear()
  listAll.mockReset().mockResolvedValue([])
  listWaiting.mockReset().mockResolvedValue([])
  // @ts-expect-error partial preload surface is enough for this hook
  window.api = { listAllWorkflowRuns: listAll, listRunsWithWaitingGates: listWaiting }
})

describe('useAllWorkflowRuns', () => {
  it('hydrates a waiting run this renderer has never seen and starts its timers', async () => {
    const waiting = run({
      status: 'running',
      nodeStates: [{ nodeId: 'gate', status: 'waiting' }]
    } as Partial<WorkflowExecution>)
    listWaiting.mockResolvedValue([waiting])

    renderHook(() => useAllWorkflowRuns(10))

    await waitFor(() => expect(setWorkflowExecution).toHaveBeenCalledWith('run-1', waiting))
    expect(rescheduleMock).toHaveBeenCalledWith([waiting], mockState.config.workflows)
  })

  it('does not re-hydrate or re-time a run already in the live map', async () => {
    mockState.workflowExecutions.set('run-1', run())
    listWaiting.mockResolvedValue([run()])

    renderHook(() => useAllWorkflowRuns(10))

    await waitFor(() => expect(listWaiting).toHaveBeenCalled())
    expect(setWorkflowExecution).not.toHaveBeenCalled()
    expect(rescheduleMock).not.toHaveBeenCalled()
  })

  it('prefers the live entry over the persisted snapshot while the run is in flight', async () => {
    mockState.workflowExecutions.set(
      'run-1',
      run({ nodeStates: [{ nodeId: 'a', status: 'running' }] } as Partial<WorkflowExecution>)
    )
    listAll.mockResolvedValue([
      { ...run(), nodeStates: [], workflowName: 'Alpha' } as WorkflowExecution
    ])

    const { result } = renderHook(() => useAllWorkflowRuns(10))

    // Wait on the merge itself: `workflowName` only exists on persisted rows,
    // so a bare length check would pass on the live-only entry first.
    await waitFor(() => expect(result.current.runs[0]?.workflowName).toBe('Alpha'))
    expect(result.current.runs).toHaveLength(1)
    expect(result.current.runs[0].nodeStates).toHaveLength(1)
    expect(result.current.runs[0].nodeStates[0].status).toBe('running')
  })

  it('lets a finished persisted row override a live entry stuck on running', async () => {
    // The shape left behind when another window owned and completed the run.
    mockState.workflowExecutions.set(
      'run-1',
      run({
        status: 'running',
        nodeStates: [{ nodeId: 'gate', status: 'waiting' }]
      } as Partial<WorkflowExecution>)
    )
    listAll.mockResolvedValue([
      {
        ...run(),
        status: 'success',
        completedAt: '2026-04-20T11:05:00Z',
        nodeStates: [{ nodeId: 'gate', status: 'success' }],
        workflowName: 'Alpha'
      } as WorkflowExecution
    ])

    const { result } = renderHook(() => useAllWorkflowRuns(10))

    await waitFor(() => expect(result.current.runs[0]?.status).toBe('success'))
    expect(result.current.runs).toHaveLength(1)
    expect(result.current.runs[0].nodeStates[0].status).toBe('success')
  })
})
