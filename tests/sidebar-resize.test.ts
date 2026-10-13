// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import {
  useSidebarResize,
  COLLAPSED_WIDTH
} from '../packages/web/src/renderer/components/project-sidebar/useSidebarResize'

describe('useSidebarResize', () => {
  it('starts with default width of 256 and not collapsed', () => {
    const { result } = renderHook(() => useSidebarResize())
    expect(result.current.sidebarWidth).toBe(256)
    expect(result.current.isCollapsed).toBe(false)
    expect(result.current.isResizingState).toBe(false)
  })

  it('collapses on double click', () => {
    const { result } = renderHook(() => useSidebarResize())
    act(() => result.current.handleResizeDoubleClick())
    expect(result.current.sidebarWidth).toBe(COLLAPSED_WIDTH)
    expect(result.current.isCollapsed).toBe(true)
  })

  it('restores previous width on second double click', () => {
    const { result } = renderHook(() => useSidebarResize())
    // Collapse
    act(() => result.current.handleResizeDoubleClick())
    expect(result.current.isCollapsed).toBe(true)
    // Restore
    act(() => result.current.handleResizeDoubleClick())
    expect(result.current.sidebarWidth).toBe(256)
    expect(result.current.isCollapsed).toBe(false)
  })

  it('COLLAPSED_WIDTH is 52', () => {
    expect(COLLAPSED_WIDTH).toBe(52)
  })
})
