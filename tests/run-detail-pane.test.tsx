// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest'
import { render, screen, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'
import type { RunListEntry } from '../src/renderer/hooks/useAllWorkflowRuns'
import { useAppStore } from '../src/renderer/stores'

// Answering a gate is a request; the pane only makes it.
const resolveWorkflowGate = vi.fn()

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
  resolveWorkflowGate.mockReset()
  ;(window as unknown as { api: unknown }).api = {
    resolveWorkflowGate,
    retryWorkflowRun: vi.fn().mockResolvedValue(null),
    rerunWorkflowRun: vi.fn().mockResolvedValue(null)
  }
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

describe('RunDetailPane — a gate that takes changes', () => {
  const nodes = [
    makeNode('t', 'Manual Trigger', 'trigger', { triggerType: 'manual' }),
    makeNode('draft', 'Make it sound like a person', 'script'),
    makeNode('gate', 'Approve the Note', 'approval', {
      message: 'ok?',
      feedback: { from: 'draft', maxRounds: 3 }
    })
  ]
  const parked = () =>
    makeRun({
      status: 'running',
      completedAt: undefined,
      nodeStates: [
        { nodeId: 't', status: 'success' },
        { nodeId: 'draft', status: 'success' },
        { nodeId: 'gate', status: 'waiting', round: 1, message: 'Post this: hello' }
      ]
    })

  it('shows the filled-in message and sends a comment back', () => {
    renderPane(parked(), nodes)
    expect(screen.getByText('Post this: hello')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Request changes/ }))
    fireEvent.change(screen.getByLabelText('What should change'), { target: { value: 'Too neat' } })
    fireEvent.click(screen.getByRole('button', { name: /Send back/ }))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-abcdef1234',
      nodeId: 'gate',
      decision: 'changes',
      comment: 'Too neat'
    })
  })

  it('keeps R from rejecting while a comment is being written', () => {
    renderPane(parked(), nodes)
    fireEvent.click(screen.getByRole('button', { name: /Request changes/ }))
    fireEvent.keyDown(document, { key: 'r' })
    expect(resolveWorkflowGate).not.toHaveBeenCalled()
  })
})

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
    expect(screen.getByText(/^Run run-abcd · manual · /)).toBeInTheDocument()
  })

  it('does not repeat the workflow name under the title when they are the same', () => {
    renderPane(makeRun())
    expect(screen.getAllByText(/clean branches/)).toHaveLength(1)
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
    expect(screen.getByText(/^clean branches · Run /)).toBeInTheDocument()
    expect(screen.getByText('refactor: split workflow runs panel')).toBeInTheDocument()
  })

  it('offers a retry from the failed step when a step failed but the run went on', () => {
    useAppStore.setState({
      config: {
        ...(useAppStore.getState().config ?? {}),
        workflows: [{ id: 'wf-a', name: 'clean branches', nodes: NODES, edges: [] }]
      }
    } as never)
    renderPane(
      makeRun({ status: 'success', nodeStates: [{ nodeId: 'n1', status: 'error', error: 'boom' }] })
    )
    expect(screen.getByLabelText('Retry from failed step')).toBeInTheDocument()
  })

  it('names the step a failed run stopped at, beside its dot', () => {
    renderPane(
      makeRun({
        status: 'error',
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'n1', status: 'error', error: 'exit 1' }
        ]
      })
    )
    expect(screen.getByText('Failed at Execute Script')).toBeInTheDocument()
    expect(screen.queryByText('exit 1')).not.toBeInTheDocument()
  })

  it('names the step a running run is on', () => {
    renderPane(
      makeRun({
        status: 'running',
        completedAt: undefined,
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'n1', status: 'running', logs: 'Scanning 14 local branches…' }
        ]
      })
    )
    expect(screen.getByText('Running Execute Script')).toBeInTheDocument()
  })

  it('includes the trigger in the trace', () => {
    renderPane(makeRun())
    expect(screen.getByTestId('run-steps-list').getAttribute('data-include-trigger')).toBe('true')
  })

  it('says a plain success completed', () => {
    renderPane(makeRun())
    expect(screen.getByText('Completed')).toBeInTheDocument()
  })

  it('shows the verdict a successful step wrote', () => {
    renderPane(
      makeRun({
        nodeStates: [{ nodeId: 'n1', status: 'success', structuredOutput: { verdict: 'approve' } }]
      })
    )
    expect(screen.getByText('approve')).toBeInTheDocument()
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
      expect(screen.getByText('Waiting at Review')).toBeInTheDocument()
    })

    it('approves the waiting node on click', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.click(screen.getByRole('button', { name: /Approve & continue/ }))
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: gateRun.runId,
        nodeId: 'gate',
        decision: 'approve'
      })
    })

    it('rejects the waiting node on click', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.click(screen.getByRole('button', { name: /Reject run/ }))
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: gateRun.runId,
        nodeId: 'gate',
        decision: 'reject'
      })
    })

    it('approves on cmd+enter and rejects on r', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: gateRun.runId,
        nodeId: 'gate',
        decision: 'approve'
      })
      fireEvent.keyDown(document, { key: 'r' })
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: gateRun.runId,
        nodeId: 'gate',
        decision: 'reject'
      })
    })

    it('ignores an auto-repeated keypress so a held key cannot reject the next run', () => {
      renderPane(gateRun, gateNodes)
      fireEvent.keyDown(document, { key: 'r', repeat: true })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true, repeat: true })
      expect(resolveWorkflowGate).not.toHaveBeenCalled()
    })

    it('mutes the shortcuts while another surface is layered over the pane', () => {
      renderPane(gateRun, gateNodes, { shortcutsEnabled: false })
      fireEvent.keyDown(document, { key: 'r' })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(resolveWorkflowGate).not.toHaveBeenCalled()
    })

    it('ignores the shortcuts while typing in a field', () => {
      renderPane(gateRun, gateNodes)
      const input = document.createElement('input')
      document.body.appendChild(input)
      fireEvent.keyDown(input, { key: 'r' })
      expect(resolveWorkflowGate).not.toHaveBeenCalled()
      input.remove()
    })

    it('does not bind the shortcuts when no gate is open', () => {
      renderPane(makeRun())
      fireEvent.keyDown(document, { key: 'r' })
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(resolveWorkflowGate).not.toHaveBeenCalled()
    })

    it('offers no edit on a gate that names no editable text', () => {
      renderPane(gateRun, gateNodes)
      expect(screen.queryByRole('button', { name: /^Edit$/ })).not.toBeInTheDocument()
    })
  })

  describe('with a gate offering editable text', () => {
    const editNodes = [
      ...NODES,
      makeNode('gate', 'Review', 'approval', {
        message: 'Post this: hello',
        edit: '{{steps.n1.output}}'
      })
    ]
    const editRun = makeRun({
      status: 'running',
      completedAt: undefined,
      nodeStates: [
        { nodeId: 't', status: 'success' },
        {
          nodeId: 'gate',
          status: 'waiting',
          message: 'Post this: hello',
          editableText: 'A tidy draft.'
        }
      ]
    })

    const rewrite = (text: string): void => {
      fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
      fireEvent.change(screen.getByLabelText('The text to approve'), { target: { value: text } })
      fireEvent.click(screen.getByRole('button', { name: /^Save$/ }))
    }

    it('opens the editor on the text the steps produced', () => {
      renderPane(editRun, editNodes)
      fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
      expect((screen.getByLabelText('The text to approve') as HTMLTextAreaElement).value).toBe(
        'A tidy draft.'
      )
    })

    it('approves with the rewrite once it is saved', () => {
      renderPane(editRun, editNodes)
      rewrite('My words.')
      fireEvent.click(screen.getByRole('button', { name: /Approve & continue/ }))
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: editRun.runId,
        nodeId: 'gate',
        decision: 'approve',
        edited: 'My words.'
      })
    })

    it('sends the rewrite with the keyboard approval too', () => {
      renderPane(editRun, editNodes)
      rewrite('My words.')
      fireEvent.keyDown(document, { key: 'Enter', metaKey: true })
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: editRun.runId,
        nodeId: 'gate',
        decision: 'approve',
        edited: 'My words.'
      })
    })

    it('keeps R from rejecting while the text is being edited', () => {
      renderPane(editRun, editNodes)
      fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
      fireEvent.keyDown(document, { key: 'r' })
      expect(resolveWorkflowGate).not.toHaveBeenCalled()
    })

    it('answers as it stood when the editor is cancelled', () => {
      renderPane(editRun, editNodes)
      fireEvent.click(screen.getByRole('button', { name: /^Edit$/ }))
      fireEvent.click(screen.getByRole('button', { name: /^Cancel$/ }))
      fireEvent.click(screen.getByRole('button', { name: /Approve & continue/ }))
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: editRun.runId,
        nodeId: 'gate',
        decision: 'approve'
      })
    })

    it('treats text put back as it was as no rewrite at all', () => {
      renderPane(editRun, editNodes)
      rewrite('A tidy draft.')
      fireEvent.click(screen.getByRole('button', { name: /Approve & continue/ }))
      expect(resolveWorkflowGate).toHaveBeenCalledWith({
        runId: editRun.runId,
        nodeId: 'gate',
        decision: 'approve'
      })
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
