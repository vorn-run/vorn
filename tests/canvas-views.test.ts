// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  pruneCanvasViews,
  readCanvasView,
  readOutlineOpen,
  writeCanvasView,
  writeOutlineOpen
} from '../src/renderer/lib/canvas-views'

beforeEach(() => localStorage.clear())

describe('the view each workflow is left at', () => {
  it('is nothing the first time, then exactly where it was left', () => {
    expect(readCanvasView('wf-1')).toBeNull()
    writeCanvasView('wf-1', { x: -120, y: 48, zoom: 0.75 })
    expect(readCanvasView('wf-1')).toEqual({ x: -120, y: 48, zoom: 0.75 })
  })

  it('keeps each workflow to its own place', () => {
    writeCanvasView('wf-1', { x: 1, y: 2, zoom: 1 })
    writeCanvasView('wf-2', { x: 3, y: 4, zoom: 0.5 })
    expect(readCanvasView('wf-1')).toEqual({ x: 1, y: 2, zoom: 1 })
    expect(readCanvasView('wf-2')).toEqual({ x: 3, y: 4, zoom: 0.5 })
  })

  it('forgets the one looked at longest ago once fifty are kept', () => {
    for (let i = 0; i < 51; i++) writeCanvasView(`wf-${i}`, { x: i, y: 0, zoom: 1 }, 1000 + i)
    expect(readCanvasView('wf-0')).toBeNull()
    expect(readCanvasView('wf-1')).toEqual({ x: 1, y: 0, zoom: 1 })
    expect(readCanvasView('wf-50')).toEqual({ x: 50, y: 0, zoom: 1 })
  })

  it('drops the views of workflows that were deleted', () => {
    writeCanvasView('kept', { x: 0, y: 0, zoom: 1 })
    writeCanvasView('gone', { x: 0, y: 0, zoom: 1 })
    pruneCanvasViews(new Set(['kept']))
    expect(readCanvasView('kept')).not.toBeNull()
    expect(readCanvasView('gone')).toBeNull()
  })

  it('treats what it cannot read as never having been there', () => {
    localStorage.setItem(
      'vorn:canvasViews',
      JSON.stringify({ wf: { x: 'left', y: 0, zoom: 1, at: 1 } })
    )
    expect(readCanvasView('wf')).toBeNull()
    localStorage.setItem('vorn:canvasViews', 'not json')
    expect(readCanvasView('wf')).toBeNull()
  })
})

describe('the step outline', () => {
  it('shows until it is closed, and stays the way it was left', () => {
    expect(readOutlineOpen()).toBe(true)
    writeOutlineOpen(false)
    expect(readOutlineOpen()).toBe(false)
    writeOutlineOpen(true)
    expect(readOutlineOpen()).toBe(true)
  })
})
