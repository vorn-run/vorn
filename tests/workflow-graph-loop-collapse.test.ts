import { describe, expect, it } from 'vitest'
import {
  collapseLoopBodies,
  loopBodyGraph,
  loopBodyOwners,
  loopStructureError
} from '../packages/shared/src/workflow-graph'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

const node = (id: string, type = 'script', config: Record<string, unknown> = {}): WorkflowNode =>
  ({ id, type, label: id, position: { x: 0, y: 0 }, config }) as unknown as WorkflowNode
const loop = (bodyNodeIds: string[], id = 'loop') =>
  node(id, 'loop', { nodeType: 'loop', bodyNodeIds, maxIterations: 1 })
const edge = (
  source: string,
  target: string,
  conditionBranch?: 'true' | 'false'
): WorkflowEdge => ({
  id: `${source}-${target}`,
  source,
  target,
  ...(conditionBranch && { conditionBranch })
})

describe('loopBodyOwners', () => {
  it('ignores ids that no longer exist and a loop listing itself', () => {
    const owners = loopBodyOwners([loop(['a', 'gone', 'loop']), node('a')])
    expect([...owners]).toEqual([['a', 'loop']])
  })
})

describe('collapseLoopBodies', () => {
  it('shows the main scheduler the loop in place of its body', () => {
    const nodes = [node('t', 'trigger'), loop(['a', 'b']), node('a'), node('b'), node('next')]
    const edges = [edge('t', 'loop'), edge('loop', 'a'), edge('a', 'b'), edge('b', 'next')]
    expect(collapseLoopBodies(nodes, edges).map((e) => [e.source, e.target])).toEqual([
      ['t', 'loop'],
      ['loop', 'next']
    ])
  })

  it('joins two exits of a branching body into one edge from the loop', () => {
    const nodes = [
      loop(['c', 'y', 'n']),
      node('c', 'condition'),
      node('y'),
      node('n'),
      node('next')
    ]
    const edges = [
      edge('loop', 'c'),
      edge('c', 'y', 'true'),
      edge('c', 'n', 'false'),
      edge('y', 'next'),
      edge('n', 'next')
    ]
    const collapsed = collapseLoopBodies(nodes, edges)
    expect(collapsed).toHaveLength(1)
    expect(collapsed[0]).toMatchObject({ source: 'loop', target: 'next' })
    expect(collapsed[0].conditionBranch).toBeUndefined()
  })

  it('leaves a workflow without loops untouched', () => {
    const nodes = [node('a'), node('b')]
    const edges = [edge('a', 'b')]
    expect(collapseLoopBodies(nodes, edges)).toEqual(edges)
  })
})

describe('loopBodyGraph', () => {
  it('chains a body that has no edges of its own, as loops ran before', () => {
    const nodes = [loop(['a', 'b', 'c']), node('a'), node('b'), node('c')]
    const graph = loopBodyGraph(nodes, [], nodes[0])
    expect(graph.entries).toEqual(['a'])
    expect(graph.edges.map((e) => [e.source, e.target])).toEqual([
      ['a', 'b'],
      ['b', 'c']
    ])
  })

  it('finds every step nothing inside the body feeds', () => {
    const nodes = [loop(['a', 'b', 'c']), node('a'), node('b'), node('c')]
    const graph = loopBodyGraph(
      nodes,
      [edge('loop', 'a'), edge('a', 'c'), edge('b', 'c')],
      nodes[0]
    )
    expect(graph.entries).toEqual(['a', 'b'])
    expect(graph.edges).toHaveLength(2)
  })
})

describe('loopStructureError', () => {
  const check = (nodes: WorkflowNode[], edges: WorkflowEdge[]) =>
    loopStructureError(nodes, edges, nodes.find((n) => n.type === 'loop')!)

  it('accepts a branching body', () => {
    const nodes = [loop(['c', 'y', 'n']), node('c', 'condition'), node('y'), node('n')]
    expect(
      check(nodes, [edge('loop', 'c'), edge('c', 'y', 'true'), edge('c', 'n', 'false')])
    ).toBeUndefined()
  })

  it('refuses an empty body, a gate, a nested loop and a trigger', () => {
    expect(check([loop([])], [])).toMatch(/no body steps/)
    expect(check([loop(['g']), node('g', 'approval')], [])).toMatch(/approval gate/)
    expect(check([loop(['inner']), loop(['x'], 'inner'), node('x')], [])).toMatch(/another loop/)
    expect(check([loop(['t']), node('t', 'trigger')], [])).toMatch(/trigger/)
  })

  it('refuses a body fed from outside, a branch leaving it, and a cycle', () => {
    expect(check([loop(['a']), node('a'), node('out')], [edge('out', 'a')])).toMatch(/from outside/)
    expect(
      check([loop(['c']), node('c', 'condition'), node('out')], [edge('c', 'out', 'true')])
    ).toMatch(/branches to a step outside/)
    expect(
      check([loop(['a', 'b']), node('a'), node('b')], [edge('a', 'b'), edge('b', 'a')])
    ).toMatch(/cycle/)
  })
})
