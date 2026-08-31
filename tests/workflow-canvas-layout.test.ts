import { describe, it, expect } from 'vitest'
import {
  canConnect,
  layoutPositions,
  loopBodyMembers,
  positionsAreSeed,
  toCanvasElements
} from '../src/renderer/lib/workflow-canvas-layout'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

const node = (
  id: string,
  type: WorkflowNode['type'],
  label: string,
  config: Record<string, unknown> = {},
  position = { x: 0, y: 0 }
): WorkflowNode => ({
  id,
  type,
  label,
  config: config as WorkflowNode['config'],
  position
})

const chainNodes = [
  node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('a', 'script', 'One', { scriptType: 'bash', scriptContent: '' }),
  node('b', 'script', 'Two', { scriptType: 'bash', scriptContent: '' })
]
const chainEdges: WorkflowEdge[] = [
  { id: 'e1', source: 't', target: 'a' },
  { id: 'e2', source: 'a', target: 'b' }
]

const forkNodes = [
  node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('c', 'condition', 'Ready?', { variable: 'x', operator: 'equals', value: '1' }),
  node('yes', 'script', 'Yes', { scriptType: 'bash', scriptContent: '' }),
  node('no', 'script', 'No', { scriptType: 'bash', scriptContent: '' }),
  node('join', 'script', 'Join', { scriptType: 'bash', scriptContent: '' })
]
const forkEdges: WorkflowEdge[] = [
  { id: 'e1', source: 't', target: 'c' },
  { id: 'e2', source: 'c', target: 'yes', conditionBranch: 'true' },
  { id: 'e3', source: 'c', target: 'no', conditionBranch: 'false' },
  { id: 'e4', source: 'yes', target: 'join' },
  { id: 'e5', source: 'no', target: 'join' }
]

const loopNodes = [
  node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
  node('loop', 'loop', 'Repeat', { nodeType: 'loop', bodyNodeIds: ['w', 'r'], maxIterations: 2 }),
  node('w', 'script', 'Write', { scriptType: 'bash', scriptContent: '' }),
  node('r', 'script', 'Review', { scriptType: 'bash', scriptContent: '' }),
  node('after', 'script', 'After', { scriptType: 'bash', scriptContent: '' })
]
const loopEdges: WorkflowEdge[] = [
  { id: 'e1', source: 't', target: 'loop' },
  { id: 'e2', source: 'loop', target: 'w' },
  { id: 'e3', source: 'w', target: 'r' },
  { id: 'e4', source: 'r', target: 'after' }
]

describe('the definition projected onto the canvas', () => {
  it('keeps a straight chain in trigger order, top to bottom', () => {
    const { positions } = layoutPositions(chainNodes, chainEdges)
    expect(positions.get('t')!.y).toBeLessThan(positions.get('a')!.y)
    expect(positions.get('a')!.y).toBeLessThan(positions.get('b')!.y)
    expect(positions.get('t')!.x).toBe(positions.get('a')!.x)
  })

  it('puts fork branches side by side and the join back on the trunk', () => {
    const { positions, branchMembers } = layoutPositions(forkNodes, forkEdges)
    expect(positions.get('yes')!.x).not.toBe(positions.get('no')!.x)
    expect(positions.get('yes')!.y).toBe(positions.get('no')!.y)
    expect(positions.get('join')!.x).toBe(positions.get('c')!.x)
    expect(branchMembers.has('yes')).toBe(true)
    expect(branchMembers.has('no')).toBe(true)
    expect(branchMembers.has('join')).toBe(false)
  })

  it('draws no canvas nodes for loop body steps', () => {
    const { nodes: rf } = toCanvasElements(loopNodes, loopEdges)
    const ids = rf.map((n) => n.id)
    expect(ids).toContain('loop')
    expect(ids).not.toContain('w')
    expect(ids).not.toContain('r')
  })

  it('redraws the edge that leaves a loop body from the composite', () => {
    const { edges: rf } = toCanvasElements(loopNodes, loopEdges)
    const exit = rf.find((e) => e.id === 'e4')!
    expect(exit.source).toBe('loop')
    // Real endpoints survive so inserts still splice correctly.
    expect(exit.data).toMatchObject({ afterNodeId: 'r', beforeNodeId: 'after' })
    expect(rf.find((e) => e.id === 'e2')).toBeUndefined()
    expect(rf.find((e) => e.id === 'e3')).toBeUndefined()
  })

  it('labels condition branches and keeps the tag on the edge data', () => {
    const { edges: rf } = toCanvasElements(forkNodes, forkEdges)
    const yes = rf.find((e) => e.id === 'e2')!
    expect(yes.label).toBe('True')
    expect(yes.data).toMatchObject({ conditionBranch: 'true', insideBranch: true })
  })

  it('trails every leaf with an add button and nothing else', () => {
    const { nodes: rf } = toCanvasElements(chainNodes, chainEdges)
    const adds = rf.filter((n) => n.type === 'addStep')
    expect(adds.map((n) => n.id)).toEqual(['add:b'])
  })

  it('uses stored positions once anyone has arranged the workflow', () => {
    const arranged = chainNodes.map((n) =>
      n.id === 'a' ? { ...n, position: { x: 120, y: 300 } } : n
    )
    expect(positionsAreSeed(arranged)).toBe(false)
    const { nodes: rf } = toCanvasElements(arranged, chainEdges)
    expect(rf.find((n) => n.id === 'a')!.position).toEqual({ x: 120, y: 300 })
  })
})

describe('layout edge shapes', () => {
  it('sizes an empty loop by its placeholder', () => {
    const emptyLoop = [
      node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
      node('loop', 'loop', 'Repeat', { nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 })
    ]
    const { nodes: rf } = toCanvasElements(emptyLoop, [{ id: 'e1', source: 't', target: 'loop' }])
    const loop = rf.find((n) => n.id === 'loop')!
    expect(loop.height).toBeGreaterThan(120)
  })

  it('gives an empty fork branch a full column of width', () => {
    const halfFork = forkNodes.filter((n) => n.id !== 'no' && n.id !== 'join')
    const halfEdges = forkEdges.filter((e) => ['e1', 'e2', 'e3'].includes(e.id))
    const { positions } = layoutPositions(halfFork, halfEdges)
    // The dangling false edge points at a missing node; the true branch still lays out.
    expect(positions.get('yes')).toBeDefined()
  })
})

describe('what a hand-drawn connection may do', () => {
  it('allows a forward edge between free steps', () => {
    const open = [...chainNodes, node('c', 'script', 'Free', {})]
    expect(canConnect(open, chainEdges, 'b', 'c')).toBe(true)
  })

  it('refuses a cycle', () => {
    expect(canConnect(chainNodes, chainEdges, 'b', 't')).toBe(false)
    expect(canConnect(chainNodes, chainEdges, 'b', 'a')).toBe(false)
  })

  it('refuses edges into the trigger and out of a condition', () => {
    expect(canConnect(forkNodes, forkEdges, 'join', 't')).toBe(false)
    expect(canConnect(forkNodes, forkEdges, 'c', 'join')).toBe(false)
  })

  it('refuses edges touching a loop body', () => {
    expect(canConnect(loopNodes, loopEdges, 'after', 'w')).toBe(false)
    expect(loopBodyMembers(loopNodes)).toEqual(new Set(['w', 'r']))
  })

  it('refuses a duplicate of an existing edge', () => {
    expect(canConnect(chainNodes, chainEdges, 'a', 'b')).toBe(false)
  })
})
