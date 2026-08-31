// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const approve = vi.fn()
const reject = vi.fn()
vi.mock('../src/renderer/lib/workflow-execution', () => ({
  approveWorkflowGate: (...args: unknown[]) => approve(...args),
  rejectWorkflowGate: (...args: unknown[]) => reject(...args)
}))

const setEditingWorkflowId = vi.fn()
const setWorkflowEditorOpen = vi.fn()
const mockState = { setEditingWorkflowId, setWorkflowEditorOpen }
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
  approve.mockReset()
  reject.mockReset()
  setEditingWorkflowId.mockReset()
  setWorkflowEditorOpen.mockReset()
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

  it('falls back to "Workflow" when workflow prop is missing', () => {
    const { container } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} />
    )
    expect(container.textContent).toContain('Workflow')
  })

  it('opens the workflow editor when the pill is clicked', () => {
    const { container } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    const pill = container.firstElementChild as HTMLElement
    fireEvent.click(pill)
    expect(setEditingWorkflowId).toHaveBeenCalledWith('wf-1')
    expect(setWorkflowEditorOpen).toHaveBeenCalledWith(true)
  })

  it('invokes approveWorkflowGate when Approve is clicked and stops propagation', () => {
    const { getByLabelText } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    fireEvent.click(getByLabelText('Approve'))
    expect(approve).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-1' }), 'n1')
    expect(setEditingWorkflowId).not.toHaveBeenCalled()
  })

  it('invokes rejectWorkflowGate when Reject is clicked and stops propagation', () => {
    const { getByLabelText } = render(
      <WaitingApprovalPill execution={execution()} nodeState={nodeState()} workflow={workflow()} />
    )
    fireEvent.click(getByLabelText('Reject'))
    expect(reject).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-1' }), 'n1')
    expect(setEditingWorkflowId).not.toHaveBeenCalled()
  })
})
