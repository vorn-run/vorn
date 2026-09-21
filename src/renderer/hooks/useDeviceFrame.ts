import { useCallback, useEffect, useRef, useState } from 'react'
import { maxEdgeFor } from '../lib/device-bezel'
import type { DeviceOrientation } from '../../shared/types'

/** How often a visible pane asks main for a fresh still. */
const POLL_MS = 500

/**
 * True when `visibility: hidden` is in force on `el` or anything above it.
 *
 * The one hidden-ness an IntersectionObserver structurally cannot report: the
 * element keeps its box and keeps intersecting, so the observer calls it on
 * screen. Walking ancestors because `visibility` inherits — the hide is applied
 * to the pane wrapper, not to the element being polled. Deliberately not
 * `checkVisibility()`/`offsetParent`, both of which need layout that jsdom
 * never performs, and would report every pane hidden under test.
 */
function isCssHidden(el: HTMLElement): boolean {
  for (let node: HTMLElement | null = el; node; node = node.parentElement) {
    if (getComputedStyle(node).visibility === 'hidden') return true
  }
  return false
}

export interface DeviceFrameState {
  /** Goes on the scrolling stage: what is measured, and what is polled for. */
  containerRef: React.MutableRefObject<HTMLDivElement | null>
  /** The latest still, base64 PNG, or null before the first arrives. */
  frame: string | null
  /** The device's screen in points. */
  screen: { width: number; height: number } | null
  /** Which way up the device is held, which the picture alone cannot say. */
  orientation: DeviceOrientation
  /** The stage, in CSS pixels, for working out what fits. */
  box: { width: number; height: number }
  error: string | null
  dismissed: string | null
  dismiss: () => void
  /** Report a failure from something other than the poll. */
  reportError: (err: unknown) => void
}

/**
 * The pane's running picture of one simulator.
 *
 * Polling is expensive (a full-device PNG per frame before downscaling), so it
 * runs only while the pane is actually on screen: an observer covers unmount,
 * scroll and background-tab, and a per-tick visibility check covers the case an
 * observer structurally cannot see — a sibling maximized over a pane that is
 * still full-size and intersecting, just `visibility: hidden`.
 *
 * `scaleRef` is read rather than depended on. The zoom changes far more often
 * than the poll should restart, and restarting it on every click would cancel
 * the in-flight request and fire an extra screenshot per press.
 */
export function useDeviceFrame(args: {
  sessionId: string
  /** The device being shown, or null when the pane is closed. */
  udid: string | null
  /** CSS pixels per device point, as currently drawn. */
  scaleRef: React.MutableRefObject<number>
}): DeviceFrameState {
  const { sessionId, udid, scaleRef } = args
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [frame, setFrame] = useState<string | null>(null)
  const [screen, setScreen] = useState<{ width: number; height: number } | null>(null)
  const [orientation, setOrientation] = useState<DeviceOrientation>('portrait')
  const [box, setBox] = useState<{ width: number; height: number }>({ width: 0, height: 0 })
  const [error, setError] = useState<string | null>(null)
  // The error message the person has already waved away. Compared by text so
  // a new, different failure still surfaces.
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [visible, setVisible] = useState(false)
  // Remembered separately from `visible` because the two answers can differ:
  // backgrounding the window must stop polling without making the observer
  // forget that the pane is still on screen, or nothing would ever restart it.
  const onScreenRef = useRef(false)
  // Read inside the poll, where depending on the state would restart it.
  const screenRef = useRef<{ width: number; height: number } | null>(null)
  const boxRef = useRef({ width: 0, height: 0 })

  /**
   * A different simulator means everything here describes the wrong device.
   *
   * Left alone, the previous device's picture stays on screen and stays
   * clickable for as long as the first new frame takes to arrive — and a tap on
   * it is computed from the old screen's size, so on a device of another shape
   * it lands somewhere arbitrary. Silent, and nothing on screen would say why.
   *
   * Adjusted during the render that brings the new udid in, not in an effect:
   * an effect runs after the commit, so the old device's picture would be
   * painted once more under the new device's name.
   */
  const [shown, setShown] = useState(udid)
  if (udid !== shown) {
    setShown(udid)
    setFrame(null)
    setScreen(null)
    setOrientation('portrait')
    setError(null)
    setDismissed(null)
  }

  // The poll needs the screen size to work out how many pixels to ask for, but
  // depending on the state would restart it on every frame that changes shape.
  useEffect(() => {
    screenRef.current = screen
  }, [screen])

  // `PaneColumn` hides a non-maximized sibling with `invisible` rather than
  // unmounting it, so React never tells us the pane went away. An observer on
  // the real element is the only signal that survives that — and it also
  // covers a pane scrolled out of a tall column.
  useEffect(() => {
    const el = containerRef.current
    if (!el || typeof IntersectionObserver === 'undefined') {
      onScreenRef.current = true
      setVisible(true)
      return
    }
    const io = new IntersectionObserver((entries) => {
      const onScreen = entries.some((e) => e.isIntersecting)
      onScreenRef.current = onScreen
      setVisible(onScreen && document.visibilityState !== 'hidden')
    })
    io.observe(el)
    return () => io.disconnect()
  }, [])

  useEffect(() => {
    const onVis = (): void => {
      // Only ever forcing this false leaves polling dead after the app is
      // backgrounded once: the IntersectionObserver has nothing new to
      // report, so nothing else would ever set it back.
      if (document.visibilityState === 'hidden') setVisible(false)
      else setVisible(onScreenRef.current)
    }
    document.addEventListener('visibilitychange', onVis)
    return () => document.removeEventListener('visibilitychange', onVis)
  }, [])

  useEffect(() => {
    if (!udid || !visible) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined

    // Chained timeouts, not an interval: a slow device must not queue frames
    // it will never render, which is how a laggy simulator turns into an
    // unbounded backlog of screenshot RPCs.
    const tick = async (): Promise<void> => {
      try {
        const el = containerRef.current
        // An IntersectionObserver cannot see this. Both hide paths render the
        // pane `invisible` — CSS `visibility: hidden` — which keeps the
        // element full-size and intersecting, so the observer happily reports
        // it on screen while a maximized sibling covers it completely. Left to
        // that signal alone, a hidden pane keeps pulling a full-device PNG
        // twice a second: fan spin and battery drain with no visible cause.
        // Rescheduling rather than returning matters — bailing outright would
        // kill the loop for good, since un-hiding fires no event either.
        if (el && isCssHidden(el)) {
          if (!cancelled) timer = setTimeout(() => void tick(), POLL_MS)
          return
        }
        const rect = el?.getBoundingClientRect()
        // The stage is re-measured here rather than by a ResizeObserver: the
        // poll already runs twice a second, which is well inside the time it
        // takes anyone to notice a resized pane, and it costs nothing extra.
        if (
          rect &&
          (rect.width !== boxRef.current.width || rect.height !== boxRef.current.height)
        ) {
          boxRef.current = { width: rect.width, height: rect.height }
          setBox(boxRef.current)
        }
        // The real ratio, not a hard-coded 2: on a non-retina display that
        // constant fetches four times the pixels the pane can show, and on a
        // 3× display it under-fetches and shows a soft image. Main clamps
        // whatever this asks for, so a dragged-large window cannot turn the
        // 2fps poll into a multi-megabyte one.
        const dpr = window.devicePixelRatio || 1
        const known = screenRef.current
        // Until the first frame says how big the screen is, the stage is the
        // only measure there is.
        const maxEdge = known
          ? maxEdgeFor(known, scaleRef.current, dpr)
          : rect
            ? Math.ceil(Math.max(rect.width, rect.height) * dpr)
            : undefined
        const shot = await window.api.deviceScreenshot(sessionId, maxEdge)
        if (cancelled) return
        setFrame(shot.data)
        setScreen(shot.screen)
        setOrientation(shot.orientation ?? 'portrait')
        setError(null)
        // Forget what was waved away, too. Dismissal silences one message
        // while it keeps recurring; a frame that arrives means the condition
        // behind it cleared, so the next occurrence is new news. Left set,
        // the dismissal outlived its cause and the pane would go silent
        // forever about the one failure the person had already seen once —
        // which is exactly the failure most likely to come back.
        setDismissed(null)
      } catch (err) {
        if (cancelled) return
        setError(err instanceof Error ? err.message : String(err))
      }
      if (!cancelled) timer = setTimeout(() => void tick(), POLL_MS)
    }
    void tick()

    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [sessionId, udid, visible, scaleRef])

  const reportError = useCallback((err: unknown) => {
    setError(err instanceof Error ? err.message : String(err))
  }, [])

  // Dismiss the message, not the state. These errors are sticky — no claim, a
  // dropped companion — and the poll re-sets the same string every 500ms, so
  // clearing `error` would put the identical bar back within half a second and
  // make the control look broken. A *different* failure still gets through.
  const dismiss = useCallback(() => setDismissed(error), [error])

  return {
    containerRef,
    frame,
    screen,
    orientation,
    box,
    error,
    dismissed,
    dismiss,
    reportError
  }
}
