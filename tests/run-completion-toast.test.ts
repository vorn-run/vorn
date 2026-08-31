import { describe, it, expect } from 'vitest'
import { runCompletionToast } from '../src/renderer/lib/run-presentation'
import type { WorkflowExecution, WorkflowNode } from '../packages/shared/src/types'

const nodes = [
  { id: 't', type: 'trigger', label: 'Manual', position: { x: 0, y: 0 }, config: {} },
  { id: 'a', type: 'script', label: 'Fetch', position: { x: 0, y: 0 }, config: {} },
  { id: 'b', type: 'script', label: 'Deliver', position: { x: 0, y: 0 }, config: {} }
] as unknown as WorkflowNode[]

const run = (over: Partial<WorkflowExecution>): WorkflowExecution =>
  ({
    runId: 'r',
    workflowId: 'wf',
    startedAt: '2026-08-31T14:00:00Z',
    completedAt: '2026-08-31T14:00:42Z',
    status: 'success',
    nodeStates: [],
    ...over
  }) as WorkflowExecution

describe('what a finished run says', () => {
  it('counts only real steps into the success line', () => {
    const note = runCompletionToast(
      run({
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'a', status: 'success' },
          { nodeId: 'b', status: 'success' }
        ]
      }),
      nodes
    )
    expect(note.kind).toBe('success')
    expect(note.message).toBe('Run finished — 2 steps in 42.0s')
  })

  it('blames the step that failed, not the ones skipped because of it', () => {
    const note = runCompletionToast(
      run({
        status: 'error',
        nodeStates: [
          { nodeId: 't', status: 'success' },
          { nodeId: 'a', status: 'error', error: 'Skipped: "Fetch" failed' },
          { nodeId: 'b', status: 'error', error: 'Exit code 1' }
        ]
      }),
      nodes
    )
    expect(note.kind).toBe('error')
    expect(note.message).toBe('Run failed at "Deliver"')
    expect(note.failedNodeId).toBe('b')
  })

  it('stays quiet for a cancelled run', () => {
    expect(runCompletionToast(run({ status: 'cancelled' }), nodes).kind).toBe('quiet')
  })
})
