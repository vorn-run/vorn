import { describe, it, expect } from 'vitest'
import { buildStepOutputsMap } from '../src/renderer/lib/workflow-execution'
import { DEFAULT_OUTPUT_KEYS } from '../src/renderer/lib/template-vars'
import type { WorkflowExecution, WorkflowNode } from '../src/shared/types'

/**
 * The directory a step worked in, offered to the steps after it.
 *
 * An agent asked for a worktree gets one made for the run; nothing else can
 * name it, so without this a later step has no way to work where it worked.
 */

const node = (id: string, slug: string): WorkflowNode =>
  ({ id, type: 'launchAgent', label: id, slug, config: {}, position: { x: 0, y: 0 } }) as never

const run = (states: WorkflowExecution['nodeStates']): WorkflowExecution =>
  ({
    runId: 'r1',
    workflowId: 'wf-1',
    startedAt: '2026-09-01T00:00:00Z',
    status: 'running',
    nodeStates: states
  }) as WorkflowExecution

const nodes = new Map([['research', node('research', 'research')]])

describe('what a step hands the ones after it', () => {
  it('names the worktree it was given', () => {
    const outputs = buildStepOutputsMap(
      run([{ nodeId: 'research', status: 'success', worktreePath: '/tmp/wt/build-1' }]),
      nodes
    )
    expect(outputs.research.worktreePath).toBe('/tmp/wt/build-1')
  })

  it('answers with nothing rather than undefined when it worked in place', () => {
    // A template resolving to the literal `undefined` would be a directory name.
    const outputs = buildStepOutputsMap(run([{ nodeId: 'research', status: 'success' }]), nodes)
    expect(outputs.research.worktreePath).toBe('')
  })

  it('keeps saying so for a step that failed there, which is where to look', () => {
    const outputs = buildStepOutputsMap(
      run([
        { nodeId: 'research', status: 'error', error: 'nope', worktreePath: '/tmp/wt/build-1' }
      ]),
      nodes
    )
    expect(outputs.research).toMatchObject({
      status: 'error',
      error: 'nope',
      worktreePath: '/tmp/wt/build-1'
    })
  })

  it('is offered by name, so the editor can complete it', () => {
    expect(DEFAULT_OUTPUT_KEYS.map((k) => k.key)).toContain('worktreePath')
  })
})
