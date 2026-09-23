import { describe, it, expect } from 'vitest'
import {
  buildStepOutputsMap,
  canRequestChanges,
  gateMaxRounds,
  nodesBetween
} from '../packages/shared/src/workflow-graph'
import type { WorkflowExecution, WorkflowNode } from '../packages/shared/src/types'

const edges = (...pairs: [string, string][]): { source: string; target: string }[] =>
  pairs.map(([source, target]) => ({ source, target }))

describe('the steps a request for changes sends back', () => {
  const graph = edges(
    ['t', 'a'],
    ['a', 'b'],
    ['a', 'c'],
    ['b', 'gate'],
    ['c', 'gate'],
    ['b', 'side'],
    ['gate', 'after']
  )

  it('takes every step on a path from the chosen step to the gate, and nothing beside it', () => {
    expect([...nodesBetween('a', 'gate', graph)].sort()).toEqual(['a', 'b', 'c', 'gate'])
  })

  it('takes nothing when the gate is not below the chosen step', () => {
    expect(nodesBetween('after', 'gate', graph).size).toBe(0)
    expect(nodesBetween('side', 'gate', graph).size).toBe(0)
    expect(nodesBetween('gate', 'gate', graph).size).toBe(0)
  })
})

describe('how many times a gate may ask', () => {
  it('defaults to three and stays inside what the engine allows', () => {
    expect(gateMaxRounds(undefined)).toBe(3)
    expect(gateMaxRounds({ maxRounds: 50 })).toBe(10)
    expect(gateMaxRounds({ maxRounds: 0 })).toBe(1)
  })

  it('offers changes only while rounds remain on a gate that names where to redo from', () => {
    const config = { feedback: { from: 'a', maxRounds: 3 } }
    expect(canRequestChanges(config, undefined)).toBe(true)
    expect(canRequestChanges(config, { round: 2 })).toBe(true)
    expect(canRequestChanges(config, { round: 3 })).toBe(false)
    expect(canRequestChanges({}, { round: 1 })).toBe(false)
  })
})

describe('what a gate tells the steps it sends back', () => {
  const nodes = new Map<string, WorkflowNode>([
    ['gate', { id: 'gate', type: 'approval', slug: 'approve', label: 'Approve' } as WorkflowNode],
    ['draft', { id: 'draft', type: 'script', slug: 'draft', label: 'Draft' } as WorkflowNode]
  ])
  const run = (gate: WorkflowExecution['nodeStates'][number]): WorkflowExecution =>
    ({
      runId: 'r',
      workflowId: 'w',
      startedAt: '',
      status: 'running',
      nodeStates: [gate, { nodeId: 'draft', status: 'pending' }]
    }) as WorkflowExecution

  it('reads the latest comment, every comment and the round while the gate is sent back', () => {
    const outputs = buildStepOutputsMap(
      run({
        nodeId: 'gate',
        status: 'pending',
        round: 3,
        feedback: [
          { round: 1, decision: 'changes', comment: 'Too neat', at: '' },
          { round: 2, decision: 'changes', comment: 'Shorter', at: '' }
        ]
      }),
      nodes
    )
    expect(outputs.approve).toEqual({
      text: '',
      feedback: 'Shorter',
      feedbackAll: 'Round 1: Too neat\nRound 2: Shorter',
      comments: [],
      round: 3
    })
  })

  it("reads the latest round's page comments, each marked by whether it quotes the page", () => {
    const outputs = buildStepOutputsMap(
      run({
        nodeId: 'gate',
        status: 'pending',
        round: 3,
        feedback: [
          {
            round: 1,
            decision: 'changes',
            comment: '',
            at: '',
            comments: [{ quote: 'old words', comment: 'Gone by now' }]
          },
          {
            round: 2,
            decision: 'changes',
            comment: 'Shorter',
            at: '',
            comments: [
              { quote: '79.8% of the time', comment: 'Add the baseline right after.' },
              { comment: 'Lead with the result.' }
            ]
          }
        ]
      }),
      nodes
    )
    expect(outputs.approve.comments).toEqual([
      { quote: '79.8% of the time', comment: 'Add the baseline right after.', anchored: true },
      { quote: '', comment: 'Lead with the result.', anchored: false }
    ])
  })

  it('leaves a gate that has not asked yet out of the outputs', () => {
    expect(buildStepOutputsMap(run({ nodeId: 'gate', status: 'pending' }), nodes)).toEqual({})
  })

  it("reads the reviewer's rewrite as the gate's text, and the original until there is one", () => {
    const asked = { nodeId: 'gate', status: 'waiting' as const, round: 1 }
    const original = buildStepOutputsMap(run({ ...asked, editableText: 'A tidy draft.' }), nodes)
    expect(original.approve).toMatchObject({ text: 'A tidy draft.' })

    const rewritten = buildStepOutputsMap(
      run({ ...asked, editableText: 'A tidy draft.', editedText: 'My words.' }),
      nodes
    )
    expect(rewritten.approve).toMatchObject({ text: 'My words.' })
  })
})
