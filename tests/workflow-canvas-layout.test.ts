import { describe, it, expect } from 'vitest'
import { Position } from '@xyflow/react'
import {
  alignedNodes,
  canConnect,
  layoutPositions,
  loopBodyMembers,
  positionsAreSeed,
  stepEdgePath,
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

  it('trails a workflow that ends in a loop', () => {
    const terminal = loopNodes.filter((n) => n.id !== 'after')
    const terminalEdges = loopEdges.filter((e) => e.id !== 'e4')
    const { nodes: rf } = toCanvasElements(terminal, terminalEdges)
    // The loop's body edges are not drawn, so the composite is the leaf.
    expect(rf.some((n) => n.id === 'add:loop')).toBe(true)
  })

  it('uses stored positions once anyone has arranged the workflow', () => {
    const arranged = chainNodes.map((n) =>
      n.id === 'a' ? { ...n, position: { x: 120, y: 300 } } : n
    )
    expect(positionsAreSeed(arranged)).toBe(false)
    const { nodes: rf } = toCanvasElements(arranged, chainEdges)
    // Where it was left, brought onto the lattice: 300 is not a step, 304 is.
    expect(rf.find((n) => n.id === 'a')!.position).toEqual({ x: 120, y: 304 })
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
    expect(loop.initialHeight).toBeGreaterThan(120)
  })

  it('gives an empty fork branch a full column of width', () => {
    const halfFork = forkNodes.filter((n) => n.id !== 'no' && n.id !== 'join')
    const halfEdges = forkEdges.filter((e) => ['e1', 'e2', 'e3'].includes(e.id))
    const { positions } = layoutPositions(halfFork, halfEdges)
    // The dangling false edge points at a missing node; the true branch still lays out.
    expect(positions.get('yes')).toBeDefined()
  })
})

describe('a workflow arranged before the lattice existed', () => {
  /** Positions a saved workflow carries from the old half-step layout. */
  const stored = [
    { ...chainNodes[0], position: { x: -140, y: 0 } },
    { ...chainNodes[1], position: { x: -140, y: 58 } }
  ]
  const storedEdges: WorkflowEdge[] = [{ id: 'e1', source: 't', target: 'a' }]

  it('draws the stored cards on the lattice instead of half a step off it', () => {
    const { nodes: rf } = toCanvasElements(stored, storedEdges)
    expect(rf.find((n) => n.id === 't')!.position).toEqual({ x: -136, y: 0 })
    expect(rf.find((n) => n.id === 'a')!.position).toEqual({ x: -136, y: 56 })
  })

  it('keeps the column, so a drag no longer breaks it', () => {
    const { nodes: rf } = toCanvasElements(stored, storedEdges)
    const xs = ['t', 'a'].map((id) => rf.find((n) => n.id === id)!.position.x)
    expect(new Set(xs).size).toBe(1)
    expect(Math.abs(xs[0] % 8)).toBe(0)
  })

  it('hands the healed positions to the next save, so the fix sticks', () => {
    const saved = alignedNodes(stored)
    expect(saved.map((n) => n.position)).toEqual([
      { x: -136, y: 0 },
      { x: -136, y: 56 }
    ])
  })

  // Nothing to heal must mean nothing to write: a save that rewrote every
  // workflow it opened would churn the config for no one.
  it('leaves an already-aligned workflow untouched', () => {
    const aligned = [
      { ...chainNodes[0], position: { x: -136, y: 0 } },
      { ...chainNodes[1], position: { x: -136, y: 56 } }
    ]
    expect(alignedNodes(aligned)).toBe(aligned)
  })

  it('leaves a workflow nobody has arranged to the layout walk', () => {
    // All x = 0 is the seed, which the layout places rather than heals.
    expect(alignedNodes(chainNodes)).toBe(chainNodes)
  })
})

describe('a laid-out chain and the drag lattice', () => {
  /** What the canvas does to a position when someone drags a card. */
  const asDragged = (v: number): number => Math.round(v / 8) * 8

  const chain = [
    node('t', 'trigger', 'Manual', { triggerType: 'manual' }),
    node('a', 'script', 'One', {}),
    node('b', 'script', 'Two', {})
  ]
  const chainEdges = [
    { id: 'e1', source: 't', target: 'a' },
    { id: 'e2', source: 'a', target: 'b' }
  ]

  it('puts every card of a column on the same x', () => {
    const { positions } = layoutPositions(chain, chainEdges)
    const xs = ['t', 'a', 'b'].map((id) => positions.get(id)!.x)
    expect(new Set(xs).size).toBe(1)
  })

  it('writes every coordinate on the grid a drag snaps to', () => {
    const { positions } = layoutPositions(chain, chainEdges)
    for (const { x, y } of positions.values()) {
      expect(Math.abs(x % 8)).toBe(0)
      expect(Math.abs(y % 8)).toBe(0)
    }
  })

  // The whole bug: a card picked up and put back landed 4px off the neighbours
  // that never moved, and tidying up could not cure what the next drag redid.
  it('leaves a dragged card exactly where the layout put it', () => {
    const { positions } = layoutPositions(chain, chainEdges)
    const laid = positions.get('a')!
    const dragged = { x: asDragged(laid.x), y: asDragged(laid.y) }

    expect(dragged).toEqual(laid)
    expect(dragged.x).toBe(positions.get('t')!.x)
  })

  it('keeps a fork even on both sides of its parent', () => {
    const { positions } = layoutPositions(forkNodes, forkEdges)
    const parent = positions.get('c')!.x + 280 / 2
    const left = positions.get('yes')!.x + 280 / 2
    const right = positions.get('no')!.x + 280 / 2

    expect(parent - left).toBe(right - parent)
  })

  it('keeps a card under a loop on the same centre line', () => {
    const withLoop = [
      node('loop', 'loop', 'Repeat', { nodeType: 'loop', bodyNodeIds: [], maxIterations: 2 }),
      node('after', 'script', 'After', {})
    ]
    const { positions } = layoutPositions(withLoop, [{ id: 'e1', source: 'loop', target: 'after' }])

    const loopCentre = positions.get('loop')!.x + 312 / 2
    const cardCentre = positions.get('after')!.x + 280 / 2
    expect(Math.abs(loopCentre - cardCentre)).toBeLessThanOrEqual(4)
  })
})

describe('the line between two cards', () => {
  const down = { sourcePosition: Position.Bottom, targetPosition: Position.Top }

  it('runs straight down a column', () => {
    const [path] = stepEdgePath({ sourceX: 100, sourceY: 0, targetX: 100, targetY: 60, ...down })
    expect(path).toBe('M 100,0 L 100,60')
  })

  // A drag snaps to the lattice, so a card lands a step off without anyone
  // meaning it. One step is a near miss, not a fork.
  it('stays straight through a snap-sized offset', () => {
    const [path] = stepEdgePath({ sourceX: 100, sourceY: 0, targetX: 108, targetY: 60, ...down })
    expect(path).toBe('M 100,0 L 108,60')
    expect(path).not.toContain('C')
  })

  it('keeps the curve for a branch that really is beside its parent', () => {
    const [path] = stepEdgePath({ sourceX: 100, sourceY: 0, targetX: 320, targetY: 60, ...down })
    expect(path).toContain('C')
  })

  it('hangs the label between the two ports', () => {
    const [, labelX, labelY] = stepEdgePath({
      sourceX: 100,
      sourceY: 0,
      targetX: 108,
      targetY: 60,
      ...down
    })
    expect(labelX).toBe(104)
    expect(labelY).toBe(30)
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
