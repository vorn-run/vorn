// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useRunsListResize } from '../src/renderer/components/workflow-runs/useRunsListResize'

function startDrag(handleResizeStart: (e: React.PointerEvent) => void, clientX: number): void {
  act(() =>
    handleResizeStart({
      clientX,
      preventDefault: () => {}
    } as React.PointerEvent)
  )
}

function movePointer(clientX: number): void {
  act(() => {
    document.dispatchEvent(new PointerEvent('pointermove', { clientX }))
  })
}

describe('useRunsListResize', () => {
  it('starts at the given width and not resizing', () => {
    const { result } = renderHook(() => useRunsListResize())
    expect(result.current.listWidth).toBe(420)
    expect(result.current.isResizing).toBe(false)
  })

  it('widens the list as the pointer moves right', () => {
    const { result } = renderHook(() => useRunsListResize())
    startDrag(result.current.handleResizeStart, 500)
    expect(result.current.isResizing).toBe(true)
    movePointer(560)
    expect(result.current.listWidth).toBe(480)
  })

  it('clamps to the min and max widths', () => {
    const { result } = renderHook(() => useRunsListResize())
    startDrag(result.current.handleResizeStart, 500)

    movePointer(0)
    expect(result.current.listWidth).toBe(280)

    movePointer(5000)
    expect(result.current.listWidth).toBe(720)
  })

  it('restores the initial width on reset', () => {
    const { result } = renderHook(() => useRunsListResize())
    startDrag(result.current.handleResizeStart, 500)
    movePointer(600)
    act(() => {
      document.dispatchEvent(new PointerEvent('pointerup'))
    })

    act(() => result.current.resetWidth())
    expect(result.current.listWidth).toBe(420)
  })

  it('releases the drag when the pointer is cancelled', () => {
    const { result } = renderHook(() => useRunsListResize())
    startDrag(result.current.handleResizeStart, 500)
    movePointer(600)
    expect(result.current.listWidth).toBe(520)

    // An OS gesture can swallow the pointerup entirely. Without a
    // pointercancel path the document would keep a resize cursor and the list
    // would keep tracking a pointer the user is no longer dragging with.
    act(() => {
      document.dispatchEvent(new PointerEvent('pointercancel'))
    })
    expect(result.current.isResizing).toBe(false)
    expect(document.body.style.cursor).toBe('')
    expect(document.body.style.userSelect).toBe('')

    movePointer(700)
    expect(result.current.listWidth).toBe(520)
  })
})
