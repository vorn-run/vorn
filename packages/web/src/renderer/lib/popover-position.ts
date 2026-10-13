import { clamp } from './math'

export interface AnchorRect {
  top: number
  left: number
  right: number
  bottom: number
  width: number
  height: number
}

export interface PopoverSize {
  width: number
  height: number
}

export interface ViewportSize {
  width: number
  height: number
}

export interface PopoverPosition {
  top: number
  left: number
  placement: 'top' | 'bottom'
}

const VIEWPORT_MARGIN = 8
const POPOVER_GAP = 6

export function calculatePopoverPosition(
  anchor: AnchorRect,
  popover: PopoverSize,
  viewport: ViewportSize
): PopoverPosition {
  // Compute left edge directly (no CSS translateX(-50%) needed)
  const centerX = anchor.left + anchor.width / 2
  const idealLeft = centerX - popover.width / 2
  const minLeft = VIEWPORT_MARGIN
  const maxLeft = viewport.width - VIEWPORT_MARGIN - popover.width
  const left = minLeft > maxLeft ? VIEWPORT_MARGIN : clamp(idealLeft, minLeft, maxLeft)

  const bottomTop = anchor.bottom + POPOVER_GAP
  const topTop = anchor.top - popover.height - POPOVER_GAP

  if (bottomTop + popover.height + VIEWPORT_MARGIN <= viewport.height) {
    return { top: bottomTop, left, placement: 'bottom' }
  }

  if (topTop >= VIEWPORT_MARGIN) {
    return { top: topTop, left, placement: 'top' }
  }

  return {
    top: clamp(bottomTop, VIEWPORT_MARGIN, viewport.height - popover.height - VIEWPORT_MARGIN),
    left,
    placement: 'bottom'
  }
}
