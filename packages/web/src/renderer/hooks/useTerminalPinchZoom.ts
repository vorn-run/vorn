import { useEffect, useRef, useCallback } from 'react'
import { setAllTerminalsFontSize, getCurrentTerminalFontSize } from '../lib/terminal-registry'
import { useAppStore } from '../stores'

const MIN_FONT_SIZE = 8
const MAX_FONT_SIZE = 28

/**
 * Enables pinch-to-zoom on a container to adjust terminal font size.
 * Works on iOS/Android via touch events (tracking two-finger distance).
 * Only active on touch devices.
 */
export function useTerminalPinchZoom(containerRef: React.RefObject<HTMLDivElement | null>): void {
  const initialDistance = useRef<number | null>(null)
  const initialFontSize = useRef<number>(14)

  const updateFontSize = useCallback((newSize: number) => {
    const clamped = Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, Math.round(newSize)))
    setAllTerminalsFontSize(clamped)
    // Persist to config
    const state = useAppStore.getState()
    if (state.config) {
      const updated = {
        ...state.config,
        defaults: { ...state.config.defaults, fontSize: clamped }
      }
      window.api.saveConfig(updated)
      state.setConfig(updated)
    }
  }, [])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    // Only enable on touch devices
    const isTouchDevice =
      typeof window !== 'undefined' && window.matchMedia('(hover: none)').matches
    if (!isTouchDevice) return

    const getDistance = (touches: TouchList): number => {
      const [a, b] = [touches[0], touches[1]]
      return Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY)
    }

    const onTouchStart = (e: TouchEvent): void => {
      if (e.touches.length !== 2) return
      initialDistance.current = getDistance(e.touches)
      initialFontSize.current = getCurrentTerminalFontSize()
    }

    const onTouchMove = (e: TouchEvent): void => {
      if (e.touches.length !== 2 || initialDistance.current === null) return
      e.preventDefault() // Prevent page zoom
      const currentDistance = getDistance(e.touches)
      const scale = currentDistance / initialDistance.current
      const newSize = initialFontSize.current * scale
      updateFontSize(newSize)
    }

    const onTouchEnd = (): void => {
      initialDistance.current = null
    }

    el.addEventListener('touchstart', onTouchStart, { passive: true })
    el.addEventListener('touchmove', onTouchMove, { passive: false })
    el.addEventListener('touchend', onTouchEnd, { passive: true })
    el.addEventListener('touchcancel', onTouchEnd, { passive: true })

    return () => {
      el.removeEventListener('touchstart', onTouchStart)
      el.removeEventListener('touchmove', onTouchMove)
      el.removeEventListener('touchend', onTouchEnd)
      el.removeEventListener('touchcancel', onTouchEnd)
    }
  }, [containerRef, updateFontSize])
}
