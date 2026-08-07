import { useCallback, useState } from 'react'

const MIN_WIDTH = 280
const MAX_WIDTH = 720

/**
 * Drag-to-resize for the Inbox list column. Width is clamped rather than
 * collapsible — the list is the navigation surface, so there is no useful
 * zero-width state.
 */
export function useRunsListResize(initialWidth = 420): {
  listWidth: number
  isResizing: boolean
  handleResizeStart: (e: React.PointerEvent) => void
  resetWidth: () => void
} {
  const [listWidth, setListWidth] = useState(initialWidth)
  const [isResizing, setIsResizing] = useState(false)

  const handleResizeStart = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault()
      setIsResizing(true)
      const startX = e.clientX
      const startWidth = listWidth

      const handleMove = (moveEvent: PointerEvent): void => {
        const next = startWidth + (moveEvent.clientX - startX)
        setListWidth(Math.min(Math.max(next, MIN_WIDTH), MAX_WIDTH))
      }
      const handleUp = (): void => {
        setIsResizing(false)
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
    [listWidth]
  )

  const resetWidth = useCallback(() => setListWidth(initialWidth), [initialWidth])

  return { listWidth, isResizing, handleResizeStart, resetWidth }
}
