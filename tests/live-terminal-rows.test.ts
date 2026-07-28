import { describe, it, expect } from 'vitest'
import {
  clampLiveRows,
  MIN_LIVE_ROWS,
  MAX_LIVE_ROWS
} from '../src/renderer/hooks/useLiveTerminalRows'

/**
 * The live region sits under the block log holding only the running command.
 * Sizing it to a fixed row count leaves a band of empty terminal below the
 * log — which reads as a second scrolling pane rather than one surface.
 */

describe('clampLiveRows', () => {
  it('gives an idle prompt room to type without a trailing band', () => {
    expect(clampLiveRows({ cursorLine: 0, isAlternate: false })).toBe(MIN_LIVE_ROWS)
  })

  it('grows with the command as it draws', () => {
    expect(clampLiveRows({ cursorLine: 4, isAlternate: false })).toBe(5)
  })

  it('stops growing so one noisy command cannot push the log off screen', () => {
    expect(clampLiveRows({ cursorLine: 500, isAlternate: false })).toBe(MAX_LIVE_ROWS)
  })

  it('hands the whole pane to a full-screen program', () => {
    // vim and pagers ask the pty how tall it is; a two-row window would make
    // them unusable.
    expect(clampLiveRows({ cursorLine: 0, isAlternate: true })).toBeNull()
  })
})
