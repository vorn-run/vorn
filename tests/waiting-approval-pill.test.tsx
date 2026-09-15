// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// Answering a gate is a request to the server; the pill only makes it.
const resolveWorkflowGate = vi.fn()

const showRun = vi.fn()
const mockState = { showRun }
vi.mock('../src/renderer/stores', () => ({
  useAppStore: (selector?: (state: unknown) => unknown) =>
    selector ? selector(mockState) : mockState
}))

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

import { WaitingApprovalPill } from '../src/renderer/components/WaitingApprovalPill'
import type { WorkflowExecution, NodeExecutionState, WorkflowDefinition } from '../src/shared/types'

function execution(): WorkflowExecution {
  return {
    runId: 'wf-1:2026-04-20T10:00:00Z',
    workflowId: 'wf-1',
    startedAt: '2026-04-20T10:00:00Z',
    status: 'running',
    nodeStates: [{ nodeId: 'n1', status: 'waiting' }]
  }
}

function nodeState(): NodeExecutionState {
  return { nodeId: 'n1', status: 'waiting' }
}

function workflow(msg?: string): WorkflowDefinition {
  return {
    id: 'wf-1',
    name: 'Deploy',
    icon: 'Zap',
    iconColor: '#ff0',
    enabled: true,
    nodes: [
      {
        id: 'n1',
        type: 'approval',
        label: 'Gate',
        position: { x: 0, y: 0 },
        config: msg !== undefined ? { message: msg } : {}
      }
    ],
    edges: []
  }
}

beforeEach(() => {
  resolveWorkflowGate.mockReset()
  ;(window as unknown as { api: unknown }).api = { resolveWorkflowGate }
  showRun.mockReset()
})

describe('WaitingApprovalPill', () => {
  it('renders workflow name and message', () => {
    const { container } = render(
      <WaitingApprovalPill
        execution={execution()}
        nodeState={nodeState()}
        workflow={workflow('please review')}
      />
    )
    expect(container.textContent).toContain('Deploy')
    expect(container.textContent).toContain('please review')
  })

  it('shows the message as it was filled in, not its template', () => {
    const { container } = render(
      <WaitingApprovalPill
        execution={execution()}
        nodeState={{ nodeId: 'n1', status: 'waiting', message: 'Post this: hello' }}
        workflow={workflow('{{steps.check.output}}')}
      />
    )
    expect(container.textContent).toContain('Post this: hello')
    expect(container.textContent).not.toContain('{{steps')
  })

  it('falls back to "Workflow" when workflow prop is missing', () => {
    const { container } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} />
    )
    expect(container.textContent).toContain('Workflow')
  })

  it('opens the run in All runs when the pill is clicked', () => {
    const { container } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    const pill = container.firstElementChild as HTMLElement
    fireEvent.click(pill)
    expect(showRun).toHaveBeenCalledWith('wf-1:2026-04-20T10:00:00Z')
  })

  it('asks the server to approve, and does not open the run', () => {
    const { getByLabelText } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    fireEvent.click(getByLabelText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'wf-1:2026-04-20T10:00:00Z',
      nodeId: 'n1',
      decision: 'approve'
    })
    expect(showRun).not.toHaveBeenCalled()
  })

  it('asks the server to reject, and does not open the run', () => {
    const { getByLabelText } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    fireEvent.click(getByLabelText('Reject'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'wf-1:2026-04-20T10:00:00Z',
      nodeId: 'n1',
      decision: 'reject'
    })
    expect(showRun).not.toHaveBeenCalled()
  })
})
