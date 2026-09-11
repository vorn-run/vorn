import { describe, expect, it } from 'vitest'
import { openingViewport, topAlignedFit } from '../src/renderer/lib/workflow-canvas-layout'

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

describe('fitting a workflow on screen', () => {
  const column = (height: number) => ({ x: -140, y: 0, width: 280, height })

  it('keeps a short one at 100%, its first step at the top rather than centred', () => {
    expect(topAlignedFit(column(300), 800, 700)).toEqual({ x: 400, y: 48, zoom: 1 })
  })

  it('zooms a long one out only as far as it needs, still from the top', () => {
    const view = topAlignedFit(column(2000), 800, 700)
    expect(view.zoom).toBeCloseTo(604 / 2000)
    expect(view.y).toBe(48)
    expect(view.x).toBe(400)
  })

  it('stops at the canvas’s furthest zoom out', () => {
    expect(topAlignedFit(column(20000), 800, 700).zoom).toBe(0.2)
  })
})
