// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { RunBucket } from '../src/renderer/stores/types'
import type { RunListEntry } from '../src/renderer/hooks/useAllWorkflowRuns'

const mockState = {
  workflowsLandingTab: 'runs' as 'runs' | 'review',
  workflowsRunFilter: 'all' as RunBucket,
  selectedRunId: null as string | null,
  setSelectedRunId: vi.fn((id: string | null) => {
    mockState.selectedRunId = id
  }),
  setMainViewMode: vi.fn(),
  setEditingWorkflowId: vi.fn(),
  config: { workflows: [] as Array<{ id: string; name: string; nodes: unknown[] }>, tasks: [] }
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
}))

const runsHookMock = vi.fn(() => ({
  runs: [] as RunListEntry[],
  loading: false,
  reload: vi.fn()
}))
vi.mock('../src/renderer/hooks/useAllWorkflowRuns', () => ({
  useAllWorkflowRuns: (...args: unknown[]) => runsHookMock(...(args as []))
}))

vi.mock('../src/renderer/components/LogReplayModal', () => ({
  LogReplayModal: ({ logs }: { logs: string }) => <div data-testid="log-replay-modal">{logs}</div>
}))

vi.mock('../src/renderer/components/workflow-runs/RunsList', () => ({
  RunsList: (props: { filter: string; selectedId: string | null }) => (
    <div
      data-testid="runs-list"
      data-filter={props.filter}
      data-selected={props.selectedId ?? ''}
    />
  )
}))

vi.mock('../src/renderer/components/workflow-runs/RunDetailPane', () => ({
  RunDetailPane: (props: { run: { runId: string } }) => (
    <div data-testid="run-detail-pane" data-run={props.run.runId} />
  ),
  RunDetailEmptyState: () => <div data-testid="run-detail-empty" />
}))

const { WorkflowsLandingView } =
  await import('../src/renderer/components/workflow-runs/WorkflowsLandingView')

function makeRun(runId: string, status = 'success', nodeStates = [{ nodeId: 'n1', status }]) {
  return {
    runId,
    workflowId: 'wf-a',
    startedAt: '2026-04-20T11:59:00Z',
    status,
    nodeStates
  } as unknown as RunListEntry
}

beforeEach(() => {
  mockState.workflowsLandingTab = 'runs'
  mockState.workflowsRunFilter = 'all'
  mockState.selectedRunId = null
  mockState.setSelectedRunId.mockClear()
  mockState.config = { workflows: [], tasks: [] }
  runsHookMock.mockReset()
  runsHookMock.mockReturnValue({ runs: [], loading: false, reload: vi.fn() })
})

describe('WorkflowsLandingView', () => {
  it('renders the list beside the empty detail state when nothing is selected', () => {
    render(<WorkflowsLandingView />)
    expect(screen.getByTestId('runs-list')).toBeInTheDocument()
    expect(screen.getByTestId('run-detail-empty')).toBeInTheDocument()
  })

  it('threads the current filter into the list', () => {
    mockState.workflowsRunFilter = 'error'
    render(<WorkflowsLandingView />)
    expect(screen.getByTestId('runs-list').getAttribute('data-filter')).toBe('error')
  })

  it('forces the waiting filter on the Needs review tab', () => {
    mockState.workflowsLandingTab = 'review'
    mockState.workflowsRunFilter = 'error'
    render(<WorkflowsLandingView />)
    expect(screen.getByTestId('runs-list').getAttribute('data-filter')).toBe('waiting')
  })

  it('auto-selects the newest visible run', () => {
    runsHookMock.mockReturnValue({
      runs: [makeRun('run-1'), makeRun('run-2')],
      loading: false,
      reload: vi.fn()
    })
    render(<WorkflowsLandingView />)
    expect(mockState.setSelectedRunId).toHaveBeenCalledWith('run-1')
  })

  it('renders the detail pane for the selected run', () => {
    mockState.selectedRunId = 'run-2'
    runsHookMock.mockReturnValue({
      runs: [makeRun('run-1'), makeRun('run-2')],
      loading: false,
      reload: vi.fn()
    })
    render(<WorkflowsLandingView />)
    expect(screen.getByTestId('run-detail-pane').getAttribute('data-run')).toBe('run-2')
  })

  it('re-selects when the current run is filtered out of the bucket', () => {
    mockState.selectedRunId = 'run-gone'
    runsHookMock.mockReturnValue({ runs: [makeRun('run-1')], loading: false, reload: vi.fn() })
    render(<WorkflowsLandingView />)
    expect(mockState.setSelectedRunId).toHaveBeenCalledWith('run-1')
  })

  it('clears the selection when no runs are visible', () => {
    mockState.selectedRunId = 'run-1'
    render(<WorkflowsLandingView />)
    expect(mockState.setSelectedRunId).toHaveBeenCalledWith(null)
  })

  it('exposes a resize handle between the panes', () => {
    render(<WorkflowsLandingView />)
    expect(screen.getByRole('separator', { name: 'Resize runs list' })).toBeInTheDocument()
  })
})
