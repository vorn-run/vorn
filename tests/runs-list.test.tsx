// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'
import type { RunListEntry } from '../src/renderer/hooks/useAllWorkflowRuns'

const mockState = {
  setMainViewMode: vi.fn(),
  setEditingWorkflowId: vi.fn()
}

vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
}))

const NOW = new Date('2026-04-20T12:00:00Z').getTime()

beforeAll(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
})

afterAll(() => {
  vi.useRealTimers()
})

beforeEach(() => {
  mockState.setMainViewMode.mockReset()
  mockState.setEditingWorkflowId.mockReset()
})

const { RunsList } = await import('../src/renderer/components/workflow-runs/RunsList')

function makeNode(id: string, label: string, type: WorkflowNode['type'] = 'launchAgent') {
  return { id, type, label, config: {}, position: { x: 0, y: 0 } } as WorkflowNode
}

function makeRun(
  workflowId: string,
  status: WorkflowExecution['status'],
  overrides: Partial<RunListEntry> = {}
): RunListEntry {
  const startedAt = overrides.startedAt ?? new Date(NOW - 60_000).toISOString()
  return {
    runId: `run-${workflowId}`,
    workflowId,
    startedAt,
    completedAt:
      'completedAt' in overrides ? overrides.completedAt : new Date(NOW - 30_000).toISOString(),
    status,
    nodeStates: overrides.nodeStates ?? [
      {
        nodeId: 'n1',
        status: status === 'success' ? 'success' : status === 'error' ? 'error' : 'running',
        startedAt
      }
    ],
    ...overrides
  } as RunListEntry
}

const noop = (): void => {}

describe('RunsList', () => {
  it('renders the empty state when there are no runs', () => {
    render(
      <RunsList
        runs={[]}
        workflowsById={new Map()}
        filter="all"
        selectedId={null}
        onSelect={noop}
      />
    )
    expect(screen.getByText('No runs to show')).toBeInTheDocument()
  })

  it('shows the visible run count and a waiting chip', () => {
    const runs = [
      makeRun('wf-a', 'success'),
      makeRun('wf-b', 'running', {
        completedAt: undefined,
        nodeStates: [{ nodeId: 'gate', status: 'waiting' }]
      })
    ]
    const wfById = new Map([
      ['wf-a', { name: 'Alpha', nodes: [] }],
      ['wf-b', { name: 'Beta', nodes: [] }]
    ])
    render(
      <RunsList runs={runs} workflowsById={wfById} filter="all" selectedId={null} onSelect={noop} />
    )
    expect(screen.getByText('2')).toBeInTheDocument()
    expect(screen.getByText('1 waiting')).toBeInTheDocument()
  })

  it('filters rows by bucket', () => {
    const runs = [makeRun('wf-a', 'success'), makeRun('wf-b', 'error')]
    const wfById = new Map([
      ['wf-a', { name: 'Alpha', nodes: [] }],
      ['wf-b', { name: 'Beta', nodes: [] }]
    ])
    render(
      <RunsList
        runs={runs}
        workflowsById={wfById}
        filter="error"
        selectedId={null}
        onSelect={noop}
      />
    )
    expect(screen.queryByText('Alpha')).not.toBeInTheDocument()
    expect(screen.getByText('Beta')).toBeInTheDocument()
  })

  it('treats a running run with a waiting node as the waiting bucket', () => {
    const runs = [
      makeRun('wf-w', 'running', {
        completedAt: undefined,
        nodeStates: [{ nodeId: 'gate', status: 'waiting' }]
      })
    ]
    const wfById = new Map([
      ['wf-w', { name: 'Wait', nodes: [makeNode('gate', 'Review', 'approval')] }]
    ])
    render(
      <RunsList
        runs={runs}
        workflowsById={wfById}
        filter="waiting"
        selectedId={null}
        onSelect={noop}
      />
    )
    expect(screen.getByText('Wait')).toBeInTheDocument()
  })

  it('reports the selected run through onSelect', () => {
    const onSelect = vi.fn()
    const runs = [makeRun('wf-a', 'success')]
    const wfById = new Map([['wf-a', { name: 'Alpha', nodes: [] }]])
    render(
      <RunsList
        runs={runs}
        workflowsById={wfById}
        filter="all"
        selectedId={null}
        onSelect={onSelect}
      />
    )
    fireEvent.click(screen.getByText('Alpha'))
    expect(onSelect).toHaveBeenCalledWith('run-wf-a')
  })

  it('marks the selected row as pressed', () => {
    const runs = [makeRun('wf-a', 'success')]
    const wfById = new Map([['wf-a', { name: 'Alpha', nodes: [] }]])
    render(
      <RunsList
        runs={runs}
        workflowsById={wfById}
        filter="all"
        selectedId="run-wf-a"
        onSelect={noop}
      />
    )
    expect(screen.getByRole('button', { pressed: true })).toBeInTheDocument()
  })

  it("draws the workflow's own icon in its own colour", () => {
    const wfById = new Map([
      ['wf-a', { name: 'Alpha', icon: 'Zap', iconColor: '#8b5cf6', nodes: [] }]
    ])
    const { container } = render(
      <RunsList
        runs={[makeRun('wf-a', 'success')]}
        workflowsById={wfById}
        filter="all"
        selectedId={null}
        onSelect={noop}
      />
    )
    const icon = container.querySelector('svg[stroke="#8b5cf6"]')
    expect(icon).toBeTruthy()
  })

  it('flags a run whose workflow was deleted and keeps the persisted name', () => {
    const runs = [makeRun('wf-gone', 'success', { workflowName: 'Old Name' })]
    render(
      <RunsList
        runs={runs}
        workflowsById={new Map()}
        filter="all"
        selectedId={null}
        onSelect={noop}
      />
    )
    expect(screen.getByText('Old Name')).toBeInTheDocument()
    expect(screen.getByText('deleted')).toBeInTheDocument()
  })

  it('opens the editor on double-click and not for a deleted workflow', () => {
    const wfById = new Map([['wf-a', { name: 'Alpha', nodes: [] }]])
    const { rerender } = render(
      <RunsList
        runs={[makeRun('wf-a', 'success')]}
        workflowsById={wfById}
        filter="all"
        selectedId={null}
        onSelect={noop}
      />
    )
    fireEvent.doubleClick(screen.getByText('Alpha'))
    expect(mockState.setEditingWorkflowId).toHaveBeenCalledWith('wf-a')
    expect(mockState.setMainViewMode).toHaveBeenCalledWith('workflows')

    mockState.setEditingWorkflowId.mockReset()
    rerender(
      <RunsList
        runs={[makeRun('wf-gone', 'success', { workflowName: 'Gone' })]}
        workflowsById={new Map()}
        filter="all"
        selectedId={null}
        onSelect={noop}
      />
    )
    fireEvent.doubleClick(screen.getByText('Gone'))
    expect(mockState.setEditingWorkflowId).not.toHaveBeenCalled()
  })

  it('renders one progress segment per stage and the outcome label', () => {
    const runs = [
      makeRun('wf-a', 'success', {
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'a', status: 'success' },
          { nodeId: 'b', status: 'success' }
        ]
      })
    ]
    const wfById = new Map([
      [
        'wf-a',
        {
          name: 'Alpha',
          nodes: [makeNode('t', 'Trigger', 'trigger'), makeNode('a', 'A'), makeNode('b', 'B')]
        }
      ]
    ])
    const { container } = render(
      <RunsList runs={runs} workflowsById={wfById} filter="all" selectedId={null} onSelect={noop} />
    )
    expect(container.querySelectorAll('[title$="· success"]')).toHaveLength(3)
    expect(screen.getByText('completed')).toBeInTheDocument()
  })
})
