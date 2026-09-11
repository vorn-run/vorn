import { describe, it, expect } from 'vitest'
import type { WorkflowExecution } from '../src/shared/types'
import { annotateWaitingGates, resolveGateTarget } from '../packages/mcp/src/tools/workflows'

const run = {
  runId: 'run-1',
  workflowId: 'wf-post',
  startedAt: '2026-09-10T20:00:00Z',
  status: 'running',
  nodeStates: [{ nodeId: 'draft', status: 'waiting', waitingFor: 'signIn' }]
} as WorkflowExecution

describe('a run waiting for a sign-in, seen by an agent', () => {
  it('is not a gate an agent can answer', () => {
    expect(resolveGateTarget(run)).toEqual({
      error: 'this run is waiting for a sign-in in the Vorn app, not for an approval'
    })
    expect(resolveGateTarget(run, 'draft')).toEqual({
      error: 'node "draft" is waiting for a sign-in in the Vorn app, not for an approval'
    })
  })

  it('can still be rejected, which ends the run rather than skipping the step', () => {
    expect(resolveGateTarget(run, undefined, 'reject')).toEqual({ nodeId: 'draft' })
    expect(resolveGateTarget(run, 'draft', 'reject')).toEqual({ nodeId: 'draft' })
  })

  it('says what it waits for', () => {
    const [annotated] = annotateWaitingGates([run], [])
    expect(annotated?.nodeStates[0]).toMatchObject({
      asks: 'Sign in to its connection in the Vorn app, and this step runs again'
    })
  })
})
