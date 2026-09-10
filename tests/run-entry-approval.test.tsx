// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, fireEvent } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'

// A gate is answered by asking the server; the entry only makes the request.
const resolveWorkflowGate = vi.fn()

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
  resolveWorkflowGate.mockReset()
  ;(window as unknown as { api: unknown }).api = { resolveWorkflowGate, stopWorkflowRun: vi.fn() }
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

  it('asks the server to approve when Approve is clicked', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve'
    })
  })

  it('asks the server to reject when Reject is clicked', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Reject'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'reject'
    })
  })

  it('offers a retry from the failed step when a step failed but the run went on', () => {
    const onRetryRun = vi.fn()
    const execution = makeExec({
      status: 'success',
      nodeStates: [{ nodeId: 'gate', status: 'error', error: 'boom' }]
    })
    const { getByLabelText } = render(
      <RunEntry execution={execution} nodes={[approvalNode]} onRetryRun={onRetryRun} />
    )
    fireEvent.click(getByLabelText('Retry from failed step'))
    expect(onRetryRun).toHaveBeenCalledWith(execution)
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

describe('RunEntry — a step waiting for a sign-in', () => {
  const draftNode = {
    id: 'draft',
    type: 'callConnectorAction',
    label: 'Draft',
    position: { x: 0, y: 0 },
    config: { connectionId: 'conn-sub', action: 'createDraft', args: {} }
  } as unknown as WorkflowNode
  const signedOut = 'Substack was signed out. Sign in again, and this step runs again.'
  const exec = makeExec({
    nodeStates: [{ nodeId: 'draft', status: 'waiting', waitingFor: 'signIn', error: signedOut }]
  })

  it('offers the sign-in window instead of an approval', () => {
    const signInConnection = vi.fn()
    ;(window as unknown as { api: unknown }).api = {
      resolveWorkflowGate,
      stopWorkflowRun: vi.fn(),
      signInConnection
    }
    const { getAllByText, getByText, queryByText } = render(
      <RunEntry execution={exec} nodes={[draftNode]} />
    )
    expect(getAllByText(signedOut).length).toBeGreaterThan(0)
    expect(queryByText('Approve')).toBeNull()
    fireEvent.click(getByText('Sign in'))
    expect(signInConnection).toHaveBeenCalledWith('conn-sub')
    expect(resolveWorkflowGate).not.toHaveBeenCalled()
  })
})
