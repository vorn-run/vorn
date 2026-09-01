import { describe, expect, it } from 'vitest'
import type { WorkflowNode } from '../src/shared/types'
import { layoutPositions, toCanvasElements } from '../src/renderer/lib/workflow-canvas-layout'
import {
  autoLayoutNodes,
  createTriggerNode,
  createScriptNode
} from '../src/renderer/lib/workflow-helpers'

describe('the add-trigger placeholder', () => {
  it('appears on a canvas with no trigger node', () => {
    const { nodes } = toCanvasElements([], [])
    const placeholder = nodes.find((n) => n.type === 'addTrigger')
    expect(placeholder).toBeDefined()
    expect(placeholder?.draggable).toBe(false)
  })

  it('disappears once a trigger exists', () => {
    const trigger = createTriggerNode({ triggerType: 'manual' })
    const { nodes } = toCanvasElements([trigger], [])
    expect(nodes.find((n) => n.type === 'addTrigger')).toBeUndefined()
  })

  it('still appears when steps exist but no trigger does', () => {
    const script: WorkflowNode = createScriptNode()
    const { nodes } = toCanvasElements([script], [])
    expect(nodes.find((n) => n.type === 'addTrigger')).toBeDefined()
  })

  it('sits centered above the topmost drawn card', () => {
    const script = { ...createScriptNode(), position: { x: 12, y: 120 } }
    const { nodes } = toCanvasElements([script], [])
    const placeholder = nodes.find((n) => n.type === 'addTrigger')!
    expect(placeholder.position).toEqual({ x: 12, y: 120 - 58 - 56 })
  })
})

describe('layout without a trigger', () => {
  it('keeps a trigger-less fork side by side instead of one flat column', () => {
    const a = { ...createScriptNode(), id: 'a' }
    const b = { ...createScriptNode(), id: 'b' }
    const c = { ...createScriptNode(), id: 'c' }
    const edges = [
      { id: 'e1', source: 'a', target: 'b' },
      { id: 'e2', source: 'a', target: 'c' }
    ]
    const { positions } = layoutPositions([a, b, c], edges)
    expect(positions.get('b')!.y).toBeGreaterThan(positions.get('a')!.y)
    expect(positions.get('b')!.x).not.toBe(positions.get('c')!.x)
  })

  it('orders a trigger-less chain top to bottom', () => {
    const a = { ...createScriptNode(), id: 'a' }
    const b = { ...createScriptNode(), id: 'b' }
    const laid = autoLayoutNodes([a, b], [{ id: 'e1', source: 'a', target: 'b' }])
    const byId = new Map(laid.map((n) => [n.id, n]))
    expect(byId.get('b')!.position.y).toBeGreaterThan(byId.get('a')!.position.y)
  })
})
