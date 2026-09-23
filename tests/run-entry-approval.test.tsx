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

  it('asks the server to reject once the rejection is confirmed', () => {
    const { getByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Reject'))
    expect(resolveWorkflowGate).not.toHaveBeenCalled()
    fireEvent.click(getByText('Reject run'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'reject'
    })
  })

  it('keeps the note a rejection carries', () => {
    const { getByText, getByLabelText } = render(
      <RunEntry execution={makeExec()} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Reject'))
    fireEvent.change(getByLabelText('Why the run is rejected'), {
      target: { value: 'Not this week' }
    })
    fireEvent.click(getByText('Reject run'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'reject',
      comment: 'Not this week'
    })
  })

  it('shows the message as it was filled in when the gate opened', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', message: 'Post this: hello' }]
    })
    const { getByText, queryByText } = render(<RunEntry execution={exec} nodes={[approvalNode]} />)
    expect(getByText('Post this: hello')).toBeTruthy()
    expect(queryByText('Please confirm')).toBeNull()
  })

  it('offers no edit on a gate that names no editable text', () => {
    const { queryByText } = render(<RunEntry execution={makeExec()} nodes={[approvalNode]} />)
    expect(queryByText('Edit')).toBeNull()
  })

  it('lets the reviewer rewrite the text, and approves with what they wrote', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Edit'))
    const box = getByLabelText('The text to approve') as HTMLTextAreaElement
    expect(box.value).toBe('A tidy draft.')
    fireEvent.change(box, { target: { value: 'My words.' } })
    fireEvent.click(getByText('Save'))
    fireEvent.click(getByText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve',
      edited: 'My words.'
    })
  })

  it('reopens the editor on the rewrite, not on the text it replaced', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Edit'))
    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'My words.' } })
    fireEvent.click(getByText('Save'))

    fireEvent.click(getByText('Edited'))
    expect((getByLabelText('The text to approve') as HTMLTextAreaElement).value).toBe('My words.')
    fireEvent.click(getByText('Save'))
    fireEvent.click(getByText('Approve'))

    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve',
      edited: 'My words.'
    })
  })

  it('approves with nothing extra when the text is left as it was', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText } = render(<RunEntry execution={exec} nodes={[approvalNode]} />)
    fireEvent.click(getByText('Edit'))
    fireEvent.click(getByText('Save'))
    fireEvent.click(getByText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve'
    })
  })

  it('counts the words, and puts the text back with Revert', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText, getByLabelText, container } = render(
      <RunEntry execution={exec} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Edit'))
    expect(container.textContent).toContain('3 words')

    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'My words.' } })
    expect(container.textContent).toContain('2 words')
    fireEvent.click(getByText('Revert'))
    expect((getByLabelText('The text to approve') as HTMLTextAreaElement).value).toBe(
      'A tidy draft.'
    )
  })

  it('saves the rewrite on cmd+enter and abandons it on escape', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText, getByLabelText } = render(
      <RunEntry execution={exec} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Edit'))
    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'My words.' } })
    fireEvent.keyDown(getByLabelText('The text to approve'), { key: 'Enter', metaKey: true })
    fireEvent.click(getByText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve',
      edited: 'My words.'
    })

    resolveWorkflowGate.mockClear()
    fireEvent.click(getByText('Edited'))
    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'Discarded.' } })
    fireEvent.keyDown(getByLabelText('The text to approve'), { key: 'Escape' })
    fireEvent.click(getByText('Approve'))
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'approve',
      edited: 'My words.'
    })
  })

  it('warns that rejecting throws the rewrite away', () => {
    const exec = makeExec({
      nodeStates: [{ nodeId: 'gate', status: 'waiting', editableText: 'A tidy draft.' }]
    })
    const { getByText, getByLabelText, container } = render(
      <RunEntry execution={exec} nodes={[approvalNode]} />
    )
    fireEvent.click(getByText('Edit'))
    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'My words.' } })
    fireEvent.click(getByText('Save'))
    fireEvent.click(getByText('Reject'))
    expect(container.textContent).toContain('Your edit is discarded.')

    fireEvent.click(getByText('Reject run'))
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

describe('RunEntry — a gate that takes changes', () => {
  const draft: WorkflowNode = {
    id: 'draft',
    type: 'script',
    label: 'Make it sound like a person',
    position: { x: 0, y: 0 },
    config: { scriptType: 'bash', scriptContent: '' }
  }
  const gate: WorkflowNode = {
    ...approvalNode,
    config: { message: 'ok?', feedback: { from: 'draft', maxRounds: 3 } }
  }
  const parked = (round: number, extra: object = {}): WorkflowExecution =>
    makeExec({
      nodeStates: [
        { nodeId: 'draft', status: 'success' },
        { nodeId: 'gate', status: 'waiting', round, ...extra }
      ]
    })

  it('sends a comment back, saying where the work restarts and which round comes next', () => {
    const { getByText, getByLabelText, container } = render(
      <RunEntry execution={parked(1)} nodes={[draft, gate]} />
    )
    expect(getByText('round 1 of 3')).toBeTruthy()
    fireEvent.click(getByText('Request changes'))
    expect(container.textContent).toContain(
      'Runs again from Make it sound like a person with your comment, then asks you. Round 2 of 3.'
    )
    const send = getByText('Send back').closest('button')!
    expect(send).toBeDisabled()
    fireEvent.change(getByLabelText('What should change'), { target: { value: ' Too neat ' } })
    fireEvent.click(send)
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'changes',
      comment: 'Too neat'
    })
  })

  it('sends the rewrite back with the request for changes', () => {
    const { getByText, getByLabelText } = render(
      <RunEntry execution={parked(1, { editableText: 'A tidy draft.' })} nodes={[draft, gate]} />
    )
    fireEvent.click(getByText('Edit'))
    fireEvent.change(getByLabelText('The text to approve'), { target: { value: 'My words.' } })
    fireEvent.click(getByText('Save'))
    fireEvent.click(getByText('Request changes'))
    fireEvent.change(getByLabelText('What should change'), { target: { value: 'Too neat' } })
    fireEvent.click(getByText('Send back').closest('button')!)
    expect(resolveWorkflowGate).toHaveBeenCalledWith({
      runId: 'run-1',
      nodeId: 'gate',
      decision: 'changes',
      comment: 'Too neat',
      edited: 'My words.'
    })
  })

  it('shows the last request under the version that answers it', () => {
    const { getByText } = render(
      <RunEntry
        execution={parked(2, {
          feedback: [{ round: 1, decision: 'changes', comment: 'Too neat', at: '' }]
        })}
        nodes={[draft, gate]}
      />
    )
    expect(getByText('You asked')).toBeTruthy()
    expect(getByText('Too neat')).toBeTruthy()
  })

  it('stops offering changes on the last round', () => {
    const { queryByText, getByText } = render(
      <RunEntry execution={parked(3)} nodes={[draft, gate]} />
    )
    expect(getByText('round 3 of 3')).toBeTruthy()
    expect(queryByText('Request changes')).toBeNull()
  })

  it('opens a review page kept with no comments in a sealed frame on the local server', async () => {
    ;(window as unknown as { api: unknown }).api = {
      resolveWorkflowGate,
      stopWorkflowRun: vi.fn(),
      artifactForGate: vi.fn(async () => null),
      getReachableUrls: vi.fn(async () => ({ urls: [], port: 5050, remote: false }))
    }
    const { getByText, findByTitle, queryByText } = render(
      <RunEntry execution={parked(1)} nodes={[draft, gate]} />
    )
    expect(queryByText('Open review')).toBeNull()
    render(<RunEntry execution={parked(1, { viewToken: 'abc' })} nodes={[draft, gate]} />)
    fireEvent.click(getByText('Open review'))
    const frame = (await findByTitle('Review page for Ship it?')) as HTMLIFrameElement
    expect(frame.getAttribute('src')).toBe('http://127.0.0.1:5050/gate-view/run-1/gate?t=abc')
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts')
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
    fireEvent.click(getByText(/^Sign in to/))
    expect(signInConnection).toHaveBeenCalledWith('conn-sub')
    expect(resolveWorkflowGate).not.toHaveBeenCalled()
  })
})
