import type { ReactNode } from 'react'
import { clampSplitRatio } from '../lib/split-ratio'

/**
 * Draggable divider between two flex children.
 *
 * `axis: 'x'` splits left/right (terminal vs. pane column), `'y'` splits
 * top/bottom (one stacked pane vs. the next).
 *
 * Two deliberate properties:
 * - The container rect is measured **once**, at pointerdown. Re-measuring per
 *   move would feed back the size the drag itself just produced.
 * - `onRatioCommit` fires **only** on pointerup, so a drag writes to storage
 *   once rather than on every mouse move.
 */
export function SplitDivider({
  axis,
  containerRef,
  onRatioChange,
  onRatioCommit,
  label,
  testId
}: {
  axis: 'x' | 'y'
  containerRef: React.RefObject<HTMLDivElement | null>
  onRatioChange: (ratio: number) => void
  onRatioCommit: (ratio: number) => void
  label: string
  testId?: string
}): ReactNode {
  const handlePointerDown = (e: React.PointerEvent): void => {
    e.preventDefault()
    // Cards select themselves on pointerdown; without this, resizing a divider
    // would also yank selection to the card it lives in.
    e.stopPropagation()
    const container = containerRef.current
    if (!container) return
    const rect = container.getBoundingClientRect()
    let lastRatio: number | null = null

    // Capture on the divider itself. The drag crosses a <webview> (browser pane)
    // and the terminal canvas, both of which consume pointer events in their own
    // layer — without capture the move stream dies the moment the cursor leaves
    // the 4px divider and the split appears frozen.
    const divider = e.currentTarget as HTMLElement
    const pointerId = e.pointerId
    try {
      divider.setPointerCapture(pointerId)
    } catch {
      /* capture is an optimization; the listeners below still work without it */
    }

    const onMove = (ev: PointerEvent): void => {
      const offset = axis === 'x' ? ev.clientX - rect.left : ev.clientY - rect.top
      const extent = axis === 'x' ? rect.width : rect.height
      if (extent <= 0) return
      const ratio = clampSplitRatio(offset / extent)
      lastRatio = ratio
      onRatioChange(ratio)
    }
    const onUp = (): void => {
      divider.removeEventListener('pointermove', onMove)
      divider.removeEventListener('pointerup', onUp)
      divider.removeEventListener('pointercancel', onUp)
      document.removeEventListener('pointermove', onMove)
      document.removeEventListener('pointerup', onUp)
      try {
        divider.releasePointerCapture(pointerId)
      } catch {
        /* already released */
      }
      if (lastRatio !== null) onRatioCommit(lastRatio)
    }
    divider.addEventListener('pointermove', onMove)
    divider.addEventListener('pointerup', onUp)
    divider.addEventListener('pointercancel', onUp)
    document.addEventListener('pointermove', onMove)
    document.addEventListener('pointerup', onUp)
  }

  return (
    <div
      onPointerDown={handlePointerDown}
      // A hairline that reads as a seam rather than a bar, with the grab area
      // widened by a transparent overlay: a 1px hit target is unusable, but a
      // 1px *line* is what the eye wants.
      className={`shrink-0 relative transition-colors group/divider
                  after:absolute after:content-[''] hover:bg-blue-500/40
                  ${
                    axis === 'x'
                      ? 'w-px cursor-col-resize after:inset-y-0 after:-left-[3px] after:-right-[3px]'
                      : 'h-px cursor-row-resize after:inset-x-0 after:-top-[3px] after:-bottom-[3px]'
                  }`}
      style={{ background: 'rgba(255,255,255,0.06)' }}
      aria-label={label}
      role="separator"
      aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      data-testid={testId}
    />
  )
}
