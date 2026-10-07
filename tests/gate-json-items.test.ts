import { describe, expect, it } from 'vitest'
import { buildStepOutputsMap } from '../packages/shared/src/workflow-graph'
import type { WorkflowExecution, WorkflowNode } from '../packages/shared/src/types'

const gate = {
  id: 'gate',
  type: 'approval',
  slug: 'gate',
  label: 'Gate',
  config: {}
} as unknown as WorkflowNode
const outputsFor = (editableText: string, editedText?: string) =>
  buildStepOutputsMap(
    {
      runId: 'r',
      workflowId: 'w',
      startedAt: '',
      status: 'running',
      nodeStates: [{ nodeId: 'gate', status: 'success', round: 1, editableText, editedText }]
    } as WorkflowExecution,
    new Map([['gate', gate]])
  ).gate

describe("a gate's items", () => {
  it('are the rows the reviewer kept, as data', () => {
    const original = JSON.stringify([{ a: 1 }, { a: 2 }])
    expect(outputsFor(original).items).toEqual([{ a: 1 }, { a: 2 }])
    expect(outputsFor(original, JSON.stringify([{ a: 2 }])).items).toEqual([{ a: 2 }])
  })

  it('unwrap an object holding the list', () => {
    expect(outputsFor(JSON.stringify({ findings: [{ a: 1 }] })).items).toEqual([{ a: 1 }])
  })

  it('are absent for a gate whose text is not a list of records', () => {
    expect(outputsFor('Ship the release?')).not.toHaveProperty('items')
    expect(outputsFor('[1, 2]')).not.toHaveProperty('items')
    expect(outputsFor('Ship the release?').text).toBe('Ship the release?')
  })
})
