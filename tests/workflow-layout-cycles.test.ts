import { describe, it, expect } from 'vitest'
import { computeFlowLayout } from '../src/renderer/lib/workflow-helpers'
import type { WorkflowNode, WorkflowEdge } from '../packages/shared/src/types'

function node(id: string, type: WorkflowNode['type'] = 'script'): WorkflowNode {
  return { id, type, label: id, config: {}, position: { x: 0, y: 0 } }
}

describe('computeFlowLayout with a cyclic graph', () => {
  // Nothing in the editor or the MCP schema rejects a back edge, so the layout
  // has to survive one. Before the visited guard these cases hung the renderer.
  it('terminates on a simple back edge', () => {
    const nodes = [node('trigger', 'trigger'), node('a'), node('b')]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 'trigger', target: 'a' },
      { id: 'e2', source: 'a', target: 'b' },
      { id: 'e3', source: 'b', target: 'a' }
    ]
    const rows = computeFlowLayout(nodes, edges)
    expect(rows.length).toBeGreaterThan(0)
    expect(rows.length).toBeLessThan(10)
  })

  it('terminates when a node points at itself', () => {
    const nodes = [node('trigger', 'trigger'), node('a')]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 'trigger', target: 'a' },
      { id: 'e2', source: 'a', target: 'a' }
    ]
    expect(computeFlowLayout(nodes, edges).length).toBeGreaterThan(0)
  })

  it('terminates when a fork branch loops back to the fork', () => {
    const nodes = [node('trigger', 'trigger'), node('fork', 'condition'), node('x'), node('y')]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 'trigger', target: 'fork' },
      { id: 'e2', source: 'fork', target: 'x', conditionBranch: 'true' },
      { id: 'e3', source: 'fork', target: 'y', conditionBranch: 'false' },
      { id: 'e4', source: 'x', target: 'fork' }
    ]
    expect(computeFlowLayout(nodes, edges).length).toBeGreaterThan(0)
  })

  it('still draws both branches of an ordinary fork that rejoins', () => {
    // The guard must not make one branch swallow the other.
    const nodes = [
      node('trigger', 'trigger'),
      node('fork', 'condition'),
      node('x'),
      node('y'),
      node('join')
    ]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 'trigger', target: 'fork' },
      { id: 'e2', source: 'fork', target: 'x', conditionBranch: 'true' },
      { id: 'e3', source: 'fork', target: 'y', conditionBranch: 'false' },
      { id: 'e4', source: 'x', target: 'join' },
      { id: 'e5', source: 'y', target: 'join' }
    ]
    const rows = computeFlowLayout(nodes, edges)
    const fork = rows.find((r) => r.kind === 'fork')
    expect(fork).toBeDefined()
    if (fork?.kind === 'fork') {
      expect(fork.branches).toHaveLength(2)
      expect(fork.branches[0].length).toBeGreaterThan(0)
      expect(fork.branches[1].length).toBeGreaterThan(0)
    }
  })

  it('leaves an acyclic chain unchanged', () => {
    const nodes = [node('trigger', 'trigger'), node('a'), node('b'), node('c')]
    const edges: WorkflowEdge[] = [
      { id: 'e1', source: 'trigger', target: 'a' },
      { id: 'e2', source: 'a', target: 'b' },
      { id: 'e3', source: 'b', target: 'c' }
    ]
    expect(computeFlowLayout(nodes, edges).map((r) => r.kind)).toEqual([
      'node',
      'node',
      'node',
      'node'
    ])
  })
})
