// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import { useState } from 'react'
import {
  insertConditionBetween,
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
