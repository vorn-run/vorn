import { useState, useRef, useCallback } from 'react'

const MIN_WIDTH = 180
const MAX_WIDTH = 400
export const COLLAPSED_WIDTH = 52
const COLLAPSE_THRESHOLD = 120

export function useSidebarResize() {
  const [sidebarWidth, setSidebarWidth] = useState(256)
  const [isResizingState, setIsResizingState] = useState(false)
  const isResizing = useRef(false)
  const widthBeforeCollapse = useRef(256)

  const isCollapsed = sidebarWidth <= COLLAPSED_WIDTH

  const handleResizeStart = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault()
      isResizing.current = true
      setIsResizingState(true)
      const startX = e.clientX
      const startWidth = sidebarWidth

      const handleMove = (moveEvent: PointerEvent) => {
        const delta = moveEvent.clientX - startX
        const newWidth = startWidth + delta

        if (newWidth < COLLAPSE_THRESHOLD) {
          widthBeforeCollapse.current = startWidth
          setSidebarWidth(COLLAPSED_WIDTH)
        } else {
          setSidebarWidth(Math.min(Math.max(newWidth, MIN_WIDTH), MAX_WIDTH))
        }
      }

      const handleUp = () => {
        isResizing.current = false
        setIsResizingState(false)
        document.removeEventListener('pointermove', handleMove)
        document.removeEventListener('pointerup', handleUp)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
      }

      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
      document.addEventListener('pointermove', handleMove)
      document.addEventListener('pointerup', handleUp)
    },
    [sidebarWidth]
  )

  const handleResizeDoubleClick = useCallback(() => {
    if (isCollapsed) {
      setSidebarWidth(widthBeforeCollapse.current)
    } else {
      widthBeforeCollapse.current = sidebarWidth
      setSidebarWidth(COLLAPSED_WIDTH)
    }
  }, [isCollapsed, sidebarWidth])

  return {
    sidebarWidth,
    isResizingState,
    isCollapsed,
    handleResizeStart,
    handleResizeDoubleClick
  }
}
