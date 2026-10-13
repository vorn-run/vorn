import { describe, expect, it } from 'vitest'
import { calculatePopoverPosition } from '../packages/web/src/renderer/lib/popover-position'

describe('calculatePopoverPosition', () => {
  it('clamps the popover horizontally when the trigger is near the right edge', () => {
    const position = calculatePopoverPosition(
      {
        top: 40,
        left: 380,
        right: 404,
        bottom: 64,
        width: 24,
        height: 24
      },
      { width: 220, height: 86 },
      { width: 420, height: 320 }
    )

    // left edge clamped: viewport 420 - margin 8 - popover 220 = 192
    expect(position.left).toBe(192)
    expect(position.placement).toBe('bottom')
  })

  it('flips above the trigger when there is no room below', () => {
    const position = calculatePopoverPosition(
      {
        top: 250,
        left: 100,
        right: 124,
        bottom: 274,
        width: 24,
        height: 24
      },
      { width: 200, height: 72 },
      { width: 400, height: 320 }
    )

    expect(position.top).toBe(172)
    expect(position.placement).toBe('top')
  })
})
