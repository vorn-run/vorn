// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { useState } from 'react'
import {
  appendNodeAfter,
  insertBeforeFork,
  insertConditionBetween,
  insertNodeBetween,
  placeNewNodes,
  createScriptNode
} from '../src/renderer/lib/workflow-helpers'
import { useDefinitionHistory } from '../src/renderer/lib/use-definition-history'
import type { WorkflowEdge, WorkflowNode } from '../packages/shared/src/types'

const node = (
  id: string,
  type: WorkflowNode['type'],
  label: string,
  position: { x: number; y: number },
  config: Record<string, unknown> = {}
): WorkflowNode => ({
  id,
  type,
  label,
  config: config as WorkflowNode['config'],
  position
})

/** An arranged workflow: someone has dragged it, so positions are theirs. */
const arrangedNodes = [
  node('t', 'trigger', 'Manual', { x: -140, y: 0 }, { triggerType: 'manual' }),
  node('a', 'script', 'After', { x: -140, y: 200 }, { scriptType: 'bash', scriptContent: '' })
]
const arrangedEdges: WorkflowEdge[] = [{ id: 'e1', source: 't', target: 'a' }]

describe('placing the nodes a mutation creates', () => {
  it('hangs a chain of new nodes off where each predecessor actually landed', () => {
    // A condition insert creates three nodes at once, chained through the new condition.
    const result = insertConditionBetween(arrangedNodes, arrangedEdges, 't', 'a')
    const cond = result.nodes.find((n) => n.type === 'condition')!
    const trueBranch = result.nodes.find((n) => n.label === 'True Branch')!
    const falseBranch = result.nodes.find((n) => n.label === 'False Branch')!

    expect(cond.position.y).toBeGreaterThan(0)
    expect(trueBranch.position.y).toBeGreaterThan(cond.position.y)
    expect(falseBranch.position.y).toBeGreaterThan(cond.position.y)
    // Two cards on one coordinate cannot both be seen.
    expect(trueBranch.position).not.toEqual(falseBranch.position)
  })

  // A card that hangs a few pixels off its parent draws a line with a lean in
  // it, which reads as a fork nobody asked for.
  it('hangs a new card on exactly its parent x', () => {
    const newNode = createScriptNode()
    const nextNodes = [...arrangedNodes, newNode]
    const nextEdges = [...arrangedEdges, { id: 'e2', source: 'a', target: newNode.id }]
    const placed = placeNewNodes(arrangedNodes, nextNodes, nextEdges)
    const parent = placed.find((n) => n.id === 'a')!
    expect(placed.find((n) => n.id === newNode.id)!.position.x).toBe(parent.position.x)
  })

  it('centres a card under a loop, which is the wider of the two', () => {
    const loop = node(
      'loop',
      'loop',
      'Repeat',
      { x: -156, y: 0 },
      {
        nodeType: 'loop',
        bodyNodeIds: [],
        maxIterations: 2
      }
    )
    const newNode = createScriptNode()
    const placed = placeNewNodes(
      [loop],
      [loop, newNode],
      [{ id: 'e1', source: 'loop', target: newNode.id }]
    )
    const child = placed.find((n) => n.id === newNode.id)!
    // Centres agree: -156 + 312/2 is the same line as -140 + 280/2.
    expect(child.position.x + 280 / 2).toBe(-156 + 312 / 2)
  })

  it('leaves every pre-existing node exactly where it was', () => {
    const newNode = createScriptNode()
    const nextNodes = [...arrangedNodes, newNode]
    const nextEdges = [...arrangedEdges, { id: 'e2', source: 'a', target: newNode.id }]
    const placed = placeNewNodes(arrangedNodes, nextNodes, nextEdges)
    expect(placed.find((n) => n.id === 't')!.position).toEqual({ x: -140, y: 0 })
    expect(placed.find((n) => n.id === 'a')!.position).toEqual({ x: -140, y: 200 })
    expect(placed.find((n) => n.id === newNode.id)!.position).toEqual({ x: -140, y: 340 })
  })
})

describe('more placement paths', () => {
  it('probes past an occupied spot instead of covering it', () => {
    const crowd = [
      ...arrangedNodes,
      node('c', 'script', 'Crowder', { x: -140, y: 340 }, { scriptType: 'bash', scriptContent: '' })
    ]
    const newNode = createScriptNode()
    const next = [...crowd, newNode]
    const nextEdges = [...arrangedEdges, { id: 'e2', source: 'a', target: newNode.id }]
    const placed = placeNewNodes(crowd, next, nextEdges)
    const landed = placed.find((n) => n.id === newNode.id)!.position
    // a's slot below is taken by Crowder; the new node keeps moving down.
    expect(landed.y).toBeGreaterThan(340)
  })

  it('keeps a branch tag on the first half of a spliced edge', () => {
    const tagged = [
      node(
        'cond',
        'condition',
        'Ready?',
        { x: 0, y: 0 },
        { variable: 'x', operator: 'equals', value: '1' }
      ),
      node('yes', 'script', 'Yes', { x: 0, y: 200 }, { scriptType: 'bash', scriptContent: '' })
    ]
    const taggedEdges: WorkflowEdge[] = [
      { id: 't1', source: 'cond', target: 'yes', conditionBranch: 'true' }
    ]
    const result = insertNodeBetween(tagged, taggedEdges, 't1', createScriptNode())
    const first = result.edges.find((e) => e.source === 'cond')!
    expect(first.conditionBranch).toBe('true')
  })

  it('appends after a step and inserts before a fork without reflowing the rest', () => {
    const appended = appendNodeAfter(arrangedNodes, arrangedEdges, 'a', createScriptNode())
    expect(appended.nodes.find((n) => n.id === 't')!.position).toEqual({ x: -140, y: 0 })

    const beforeFork = insertBeforeFork(arrangedNodes, arrangedEdges, 't', createScriptNode())
    expect(beforeFork.edges.some((e) => e.source === 't')).toBe(true)
    expect(beforeFork.nodes.find((n) => n.id === 'a')!.position.x).toBe(-140)
  })
})

describe('what undo may reach', () => {
  function useEditorLike() {
    const [nodes, setNodes] = useState<WorkflowNode[]>([])
    const [edges, setEdges] = useState<WorkflowEdge[]>([])
    const history = useDefinitionHistory(nodes, edges, setNodes, setEdges, 'wf-1')
    return { nodes, edges, setNodes, setEdges, history }
  }

  it('never undoes past the loaded definition', () => {
    const { result } = renderHook(useEditorLike)

    // The load effect lands one commit after mount, exactly like the editor.
    act(() => {
      result.current.setNodes(arrangedNodes)
      result.current.setEdges(arrangedEdges)
    })
    act(() => result.current.history.undo())

    // A wipe here would let a save persist an emptied workflow.
    expect(result.current.nodes).toEqual(arrangedNodes)
  })

  it('starts a fresh history when a different workflow loads', () => {
    const { result, rerender } = renderHook(
      ({ resetKey }: { resetKey: string }) => {
        const [nodes, setNodes] = useState<WorkflowNode[]>([])
        const [edges, setEdges] = useState<WorkflowEdge[]>([])
        const history = useDefinitionHistory(nodes, edges, setNodes, setEdges, resetKey)
        return { nodes, setNodes, setEdges, history }
      },
      { initialProps: { resetKey: 'wf-1' } }
    )
    act(() => result.current.setNodes(arrangedNodes))
    act(() => result.current.setNodes([...arrangedNodes, createScriptNode()]))
    rerender({ resetKey: 'wf-2' })
    act(() => result.current.history.undo())
    // The old workflow's edits are unreachable across the switch.
    expect(result.current.nodes).toHaveLength(3)
  })

  it('still undoes a real edit back to the loaded definition', () => {
    const { result } = renderHook(useEditorLike)
    act(() => {
      result.current.setNodes(arrangedNodes)
      result.current.setEdges(arrangedEdges)
    })

    const edited = [...arrangedNodes, createScriptNode()]
    act(() => result.current.setNodes(edited))
    expect(result.current.nodes).toHaveLength(3)

    act(() => result.current.history.undo())
    expect(result.current.nodes).toEqual(arrangedNodes)

    act(() => result.current.history.redo())
    expect(result.current.nodes).toEqual(edited)
  })
})
