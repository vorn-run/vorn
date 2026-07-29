import { useCallback, useEffect, useState } from 'react'
import {
  getTerminalBufferMetrics,
  onTerminalReady,
  onTerminalScroll
} from '../lib/terminal-registry'
import { onCommandBlocksChange } from '../lib/command-blocks'

/**
 * How many rows the live terminal actually needs right now.
 *
 * With finished commands lifted into the block log the terminal is reset each
 * time, so the cursor's row is the height of whatever is currently running —
 * one row at an idle prompt, more while a command draws. Sizing to that keeps
 * the log from being squeezed against a mostly-empty terminal, which reads as
 * a separate scrolling pane rather than one surface.
 */

/** An idle prompt still needs its own row plus somewhere to type. */
export const MIN_LIVE_ROWS = 2

/**
 * Past this the live region stops growing and the terminal scrolls
 * internally, so one noisy command cannot push the log off screen.
 */
export const MAX_LIVE_ROWS = 16

/** `null` means the terminal should take the whole pane. */
export function clampLiveRows(metrics: {
  cursorLine: number
  isAlternate: boolean
}): number | null {
  // A full-screen program draws to the whole screen and asks the pty how big
  // it is, so sizing it to the last command's height would hand `vim` or a
  // pager a two-row window.
  if (metrics.isAlternate) return null
  return Math.min(MAX_LIVE_ROWS, Math.max(MIN_LIVE_ROWS, metrics.cursorLine + 1))
}

export function useLiveTerminalRows(terminalId: string, enabled: boolean): number | null {
  const [rows, setRows] = useState<number | null>(MIN_LIVE_ROWS)

  const measure = useCallback(() => {
    const metrics = getTerminalBufferMetrics(terminalId)
    if (!metrics) return
    // baseY is 0 for the live region because the terminal is reset after each
    // command, so the cursor line is the row offset within it.
    setRows(clampLiveRows(metrics))
  }, [terminalId])

  useEffect(() => {
    if (!enabled) return
    let disposeScroll: (() => void) | undefined
    const disposeReady = onTerminalReady(terminalId, () => {
      measure()
      disposeScroll = onTerminalScroll(terminalId, measure)
    })
    // A finished command resets the terminal, so the live region shrinks back.
    const disposeBlocks = onCommandBlocksChange(terminalId, measure)
    return () => {
      disposeReady()
      disposeScroll?.()
      disposeBlocks()
    }
  }, [terminalId, enabled, measure])

  return enabled ? rows : null
}
