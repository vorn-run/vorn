import { describe, expect, it } from 'vitest'
import { openingViewport } from '../src/renderer/lib/workflow-canvas-layout'

describe('where a workflow opens', () => {
  const placed = [
    { type: 'step', position: { x: -140, y: 0 } },
    { type: 'step', position: { x: -140, y: 114 } },
    { type: 'addStep', position: { x: -12, y: 230 } }
  ]

  it('is 100%, with the steps centred and the first near the top', () => {
    expect(openingViewport(placed, 800)).toEqual({ x: 400, y: 48, zoom: 1 })
  })

  it('centres the add button of an empty workflow', () => {
    expect(
      openingViewport([{ type: 'addTrigger', position: { x: -20, y: 0 }, width: 40 }], 600)
    ).toEqual({ x: 300, y: 48, zoom: 1 })
  })
})
