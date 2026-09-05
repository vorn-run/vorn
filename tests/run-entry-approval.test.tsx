// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

const approve = vi.fn()
const reject = vi.fn()
// The real module but for the calls this asserts, so the retry control is gated
// by the rule the entry really uses.
vi.mock('../src/renderer/lib/workflow-execution', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../src/renderer/lib/workflow-execution')>()),
  approveWorkflowGate: (...args: unknown[]) => approve(...args),
  rejectWorkflowGate: (...args: unknown[]) => reject(...args),
  isRunStoppable: (e: { status: string }) => e.status === 'running',
  stopWorkflowRun: vi.fn()
}))

vi.mock('../src/renderer/components/Tooltip', () => ({
  Tooltip: ({ children }: React.PropsWithChildren) => <>{children}</>
}))

import { RunEntry } from '../src/renderer/components/workflow-editor/RunEntry'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'

function makeExec(overrides: Partial<WorkflowExecution> = {}): WorkflowExecution {
  return {
    runId: 'run-1',
    workflowId: 'wf-1',
    startedAt: '2026-04-20T10:00:00Z',
    status: 'running',
    nodeStates: [{ nodeId: 'gate', status: 'waiting', startedAt: '2026-04-20T10:00:00Z' }],
    ...overrides
  }
}

const approvalNode: WorkflowNode = {
  id: 'gate',
  type: 'approval',
  label: 'Ship it?',
  position: { x: 0, y: 0 },
  config: { message: 'Please confirm' }
}

beforeEach(() => {
  approve.mockReset()
  reject.mockReset()
})

describe('RunEntry — approval gate controls', () => {
  it('renders the approval message and controls when expanded', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    expect(getByText('Please confirm')).toBeTruthy()
    expect(getByText('Approve')).toBeTruthy()
    expect(getByText('Reject')).toBeTruthy()
  })

  it('falls back to default text when message is empty', () => {
    const node: WorkflowNode = { ...approvalNode, config: {} }
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[node]} />)
    expect(getByText('Waiting for approval.')).toBeTruthy()
  })

  it('invokes approveWorkflowGate when Approve is clicked', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Approve'))
    expect(approve).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-1' }), 'gate')
  })

  it('invokes rejectWorkflowGate when Reject is clicked', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Reject'))
    expect(reject).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-1' }), 'gate')
  })

  it('does not render approval controls for non-approval waiting nodes', () => {
    const nonApproval: WorkflowNode = {
      id: 'gate',
      type: 'script',
      label: 'Script',
      position: { x: 0, y: 0 },
      config: { scriptType: 'bash', scriptContent: '' }
    }
    const { queryByText } = render(<RunEntry execution={makeExec()} nodes={[nonApproval]} />)
    expect(queryByText('Approve')).toBeNull()
  })
})
