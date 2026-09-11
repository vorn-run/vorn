// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { readCanvasView, writeCanvasView } from '../src/renderer/lib/canvas-views'

beforeEach(() => localStorage.clear())

describe('the view each workflow is left at', () => {
  it('is nothing the first time, then exactly where it was left', () => {
    expect(readCanvasView('wf-1')).toBeNull()
    writeCanvasView('wf-1', { x: -120, y: 48, zoom: 0.75 })
    expect(readCanvasView('wf-1')).toEqual({ x: -120, y: 48, zoom: 0.75 })
  })

  it('forgets the one looked at longest ago once fifty are kept', () => {
    for (let i = 0; i < 50; i++) writeCanvasView(`wf-${i}`, { x: i, y: 0, zoom: 1 })
    writeCanvasView('wf-0', { x: 0, y: 0, zoom: 0.5 })
    writeCanvasView('wf-50', { x: 50, y: 0, zoom: 1 })
    expect(readCanvasView('wf-1')).toBeNull()
    expect(readCanvasView('wf-0')).toEqual({ x: 0, y: 0, zoom: 0.5 })
    expect(readCanvasView('wf-50')).toEqual({ x: 50, y: 0, zoom: 1 })
  })

  it('treats what it cannot read as never having been there', () => {
    localStorage.setItem('vorn:workflowViews', JSON.stringify({ wf: { x: 'left', y: 0, zoom: 1 } }))
    expect(readCanvasView('wf')).toBeNull()
    localStorage.setItem('vorn:workflowViews', 'not json')
    expect(readCanvasView('wf')).toBeNull()
  })
})
