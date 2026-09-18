import { describe, expect, it } from 'vitest'
import {
  adoptIntoLoopBody,
  appendToLoopBody,
  insertConditionBetween,
  insertNodeBetween,
  removeNode
} from '../src/renderer/lib/workflow-helpers'
import type { LoopConfig, WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

const node = (
  id: string,
  type: WorkflowNode['type'] = 'script',
  config: Record<string, unknown> = {}
) => ({ id, type, label: id, position: { x: 0, y: 0 }, config }) as unknown as WorkflowNode
const loop = (bodyNodeIds: string[]) =>
  node('loop', 'loop', { nodeType: 'loop', bodyNodeIds, maxIterations: 1 })
const edge = (id: string, source: string, target: string): WorkflowEdge => ({ id, source, target })
const bodyOf = (nodes: WorkflowNode[]) =>
  (nodes.find((n) => n.id === 'loop')!.config as LoopConfig).bodyNodeIds

const nodes = [node('t', 'trigger'), loop(['a', 'b']), node('a'), node('b'), node('after')]
const edges = [
  edge('e1', 't', 'loop'),
  edge('e2', 'loop', 'a'),
  edge('e3', 'a', 'b'),
  edge('e4', 'b', 'after')
]

describe('inserting inside a loop body', () => {
  it('adds a step inserted between two body steps to the body', () => {
    const inserted = insertNodeBetween(nodes, edges, 'e3', node('new'))
    expect(bodyOf(adoptIntoLoopBody(nodes, inserted, 'a', 'b').nodes)).toEqual(['a', 'b', 'new'])
  })

  it('adds a step inserted on the edge from the loop to its first step', () => {
    const inserted = insertNodeBetween(nodes, edges, 'e2', node('new'))
    expect(bodyOf(adoptIntoLoopBody(nodes, inserted, 'loop', 'a').nodes)).toContain('new')
  })

  it('leaves a step inserted on the edge leaving the body outside it', () => {
    const inserted = insertNodeBetween(nodes, edges, 'e4', node('new'))
    expect(bodyOf(adoptIntoLoopBody(nodes, inserted, 'b', 'after').nodes)).toEqual(['a', 'b'])
  })

  it('takes both branches of a condition added inside the body', () => {
    const inserted = insertConditionBetween(nodes, edges, 'a', 'b')
    const body = bodyOf(adoptIntoLoopBody(nodes, inserted, 'a', 'b').nodes)
    expect(body).toHaveLength(5)
  })

  it('appends after the body step it was asked to, taking over what it led to', () => {
    const { nodes: next, edges: nextEdges } = appendToLoopBody(
      nodes,
      edges,
      'loop',
      node('new'),
      'a'
    )
    expect(bodyOf(next)).toEqual(['a', 'b', 'new'])
    expect(nextEdges.some((e) => e.source === 'a' && e.target === 'new')).toBe(true)
    expect(nextEdges.some((e) => e.source === 'new' && e.target === 'b')).toBe(true)
    expect(nextEdges.some((e) => e.source === 'a' && e.target === 'b')).toBe(false)
  })
})

describe('removing a body step', () => {
  it('takes it out of the loop that listed it', () => {
    expect(bodyOf(removeNode(nodes, edges, 'a').nodes)).toEqual(['b'])
  })

  it('takes a condition and its branches out of the body too', () => {
    const withCondition = adoptIntoLoopBody(
      nodes,
      insertConditionBetween(nodes, edges, 'a', 'b'),
      'a',
      'b'
    )
    const condition = withCondition.nodes.find((n) => n.type === 'condition')!
    const removed = removeNode(withCondition.nodes, withCondition.edges, condition.id)
    expect(bodyOf(removed.nodes)).toEqual(['a', 'b'])
  })
})
