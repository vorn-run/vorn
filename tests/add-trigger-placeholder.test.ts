import { describe, expect, it } from 'vitest'
import type { WorkflowNode } from '../src/shared/types'
import { toCanvasElements } from '../src/renderer/lib/workflow-canvas-layout'
import { createTriggerNode, createScriptNode } from '../src/renderer/lib/workflow-helpers'

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
})
