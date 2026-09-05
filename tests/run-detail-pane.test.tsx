// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'
import type { RunListEntry } from '../src/renderer/hooks/useAllWorkflowRuns'

const approveMock = vi.fn()
const rejectMock = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  approveWorkflowGate: (...args: unknown[]) => approveMock(...args),
  rejectWorkflowGate: (...args: unknown[]) => rejectMock(...args),
  // The real rule, so a retry control is exercised the way the pane gates it.
  hasFailedStep: (e: { nodeStates: Array<{ status: string; error?: string }> }) =>
    e.nodeStates.some((ns) => ns.status === 'error' && !ns.error?.startsWith('Skipped:'))
}))

vi.mock('../src/renderer/components/workflow-runs/StopRunButton', () => ({
  StopRunButton: () => <button type="button">Stop</button>
}))

vi.mock('../src/renderer/components/workflow-editor/RunEntry', () => ({
  StatusDot: ({ status }: { status: string }) => <span data-testid="status-dot">{status}</span>,
  RunStepsList: ({ includeTrigger }: { includeTrigger?: boolean }) => (
    <div data-testid="run-steps-list" data-include-trigger={String(!!includeTrigger)} />
  )
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
  approveMock.mockReset()
  rejectMock.mockReset()
})

const { RunDetailPane, RunDetailEmptyState } =
  await import('../src/renderer/components/workflow-runs/RunDetailPane')

function makeNode(id: string, label: string, type: WorkflowNode['type'], config = {}) {
  return { id, type, label, config, position: { x: 0, y: 0 } } as WorkflowNode
}

function makeRun(overrides: Partial<RunListEntry> = {}): RunListEntry {
  return {
    runId: 'run-abcdef1234',
    workflowId: 'wf-a',
    startedAt: new Date(NOW - 60_000).toISOString(),
    completedAt: new Date(NOW - 30_000).toISOString(),
    status: 'success' as WorkflowExecution['status'],
    nodeStates: [{ nodeId: 'n1', status: 'success' }],
    ...overrides
  } as RunListEntry
}

const NODES = [
  makeNode('t', 'Manual Trigger', 'trigger', { triggerType: 'manual' }),
  makeNode('n1', 'Execute Script', 'script')
]

function renderPane(run: RunListEntry, nodes = NODES, extra = {}) {
  return render(
    <RunDetailPane
      run={run}
      workflow={{ name: 'clean branches', nodes }}
      workflowDeleted={false}
      onOpenWorkflow={vi.fn()}
      {...extra}
    />
  )
}

describe('RunDetailEmptyState', () => {
  it('prompts the user to pick a run', () => {
    render(<RunDetailEmptyState />)
    expect(screen.getByText('Select a run to see its trace')).toBeInTheDocument()
  })
})

describe('RunDetailPane', () => {
  it('renders the run header with its source badge and workflow name', () => {
    renderPane(makeRun())
    expect(screen.getByRole('heading', { name: 'clean branches' })).toBeInTheDocument()
    expect(screen.getByText('manual')).toBeInTheDocument()
    expect(screen.getByText('run run-abcd')).toBeInTheDocument()
  })

  it('does not repeat the workflow name under the title when they are the same', () => {
    renderPane(makeRun())
    expect(screen.getAllByText('clean branches')).toHaveLength(1)
  })

  it('shows the workflow name under the title when the run is named after its subject', () => {
    renderPane(
      makeRun({
        connectorItem: {
          connectionId: 'c1',
          connectorId: 'github',
          externalId: '309',
          externalUrl: 'https://github.com/vorn-run/vorn/pull/309',
          title: 'refactor: split workflow runs panel',
          raw: {}
        }
      } as Partial<RunListEntry>)
    )
    expect(screen.getByRole('heading', { name: 'PR #309' })).toBeInTheDocument()
    expect(screen.getByText('clean branches')).toBeInTheDocument()
    expect(screen.getByText('refactor: split workflow runs panel')).toBeInTheDocument()
  })

  it('summarises a run that finished every stage without pausing', () => {
    renderPane(
      makeRun({
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'n1', status: 'success' }
        ]
      })
    )
    expect(screen.getByText(/ran end to end/)).toBeInTheDocument()
    expect(screen.getByText(/never paused/)).toBeInTheDocument()
  })

  it('reports a run that paused for review', () => {
    renderPane(
      makeRun({
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'n1', status: 'success', approvedAt: new Date(NOW).toISOString() }
        ]
      })
    )
    expect(screen.getByText(/paused for review/)).toBeInTheDocument()
  })

  it('shows the running step logs as the summary', () => {
    renderPane(
      makeRun({
        status: 'running',
        completedAt: undefined,
        nodeStates: [{ nodeId: 'n1', status: 'running', logs: 'Scanning 14 local branches…' }]
      })
    )
    expect(screen.getByText('Scanning 14 local branches…')).toBeInTheDocument()
  })

  it('counts completed stages in the trace header', () => {
    renderPane(
      makeRun({
        status: 'running',
        completedAt: undefined,
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'n1', status: 'running' }
        ]
      })
    )
    expect(screen.getByText('1 of 2 stages complete')).toBeInTheDocument()
  })

  it('includes the trigger in the trace', () => {
    renderPane(makeRun())
    expect(screen.getByTestId('run-steps-list').getAttribute('data-include-trigger')).toBe('true')
  })

  it('hides the approval actions when nothing is waiting', () => {
    renderPane(makeRun())
    expect(screen.queryByRole('button', { name: /Approve/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Reject run/ })).not.toBeInTheDocument()
  })

  describe('with a waiting gate', () => {
    const gateNodes = [
      ...NODES,
      makeNode('gate', 'Review', 'approval', { message: 'Safe to merge.' })
    ]
    const gateRun = makeRun({
      status: 'running',
      completedAt: undefined,
      nodeStates: [
        { nodeId: 't', status: 'success' },
        { nodeId: 'gate', status: 'waiting' }
      ]
    })

    it('offers approve and reject', () => {
      renderPane(gateRun, gateNodes)
      expect(screen.getByRole('button', { name: /Approve & continue/ })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /Reject run/ })).toBeInTheDocument()
      expect(screen.getByText('waiting for approval')).toBeInTheDocument()
    })

    it('approves the waiting node on click', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.click(screen.getByRole('button', { name: /Approve & continue/ }))
      expect(approveMock).toHaveBeenCalledWith(gateRun, 'gate')
    })

    it('rejects the waiting node on click', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.click(screen.getByRole('button', { name: /Reject run/ }))
      expect(rejectMock).toHaveBeenCalledWith(gateRun, 'gate')
    })

    it('approves on cmd+enter and rejects on r', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(approveMock).toHaveBeenCalledWith(gateRun, 'gate')
      fireEvent.keyDown(document, { key: 'r' })
      expect(rejectMock).toHaveBeenCalledWith(gateRun, 'gate')
    })

    it('ignores an auto-repeated keypress so a held key cannot reject the next run', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.keyDown(document, { key: 'r', repeat: true })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true, repeat: true })
      expect(rejectMock).not.toHaveBeenCalled()
      expect(approveMock).not.toHaveBeenCalled()
    })

    it('mutes the shortcuts while another surface is layered over the pane', () => {
      renderPane(gateRun, gateNodes, { shortcutsEnabled: false })
      fireEvent.keyDown(document, { key: 'r' })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(rejectMock).not.toHaveBeenCalled()
      expect(approveMock).not.toHaveBeenCalled()
    })

    it('ignores the shortcuts while typing in a field', () => {
      renderPane(gateRun, gateNodes)
      const input = document.createElement('input')
      document.body.appendChild(input)
      fireEvent.keyDown(input, { key: 'r' })
      expect(rejectMock).not.toHaveBeenCalled()
      input.remove()
    })

    it('does not bind the shortcuts when no gate is open', () => {
      renderPane(makeRun())
      fireEvent.keyDown(document, { key: 'r' })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(approveMock).not.toHaveBeenCalled()
      expect(rejectMock).not.toHaveBeenCalled()
    })
  })

  it('disables "Open workflow" when the workflow is gone', () => {
    render(<RunDetailPane run={makeRun()} workflowDeleted onOpenWorkflow={vi.fn()} />)
    const button = screen.getByRole('button', { name: 'Open workflow' })
    expect(button).toBeDisabled()
    expect(button.title).toBe('Workflow no longer exists')
  })

  it('routes "Open workflow" to the caller', () => {
    const onOpenWorkflow = vi.fn()
    renderPane(makeRun(), NODES, { onOpenWorkflow })
    fireEvent.click(screen.getByRole('button', { name: 'Open workflow' }))
    expect(onOpenWorkflow).toHaveBeenCalled()
  })
})
