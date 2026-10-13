import { useEffect, useState } from 'react'
import { fitAllTerminals } from '../lib/terminal-registry'

/**
 * Detects when the virtual keyboard is open on mobile and exposes the keyboard height.
 *
 * Strategy:
 * 1. `interactive-widget=resizes-content` in the viewport meta makes Chrome/Android
 *    resize the layout viewport when the keyboard opens, which triggers the normal
 *    window resize flow (the TerminalHost rAF loop already picks this up).
 *
 * 2. Safari/iOS does NOT support `interactive-widget`. Instead, it resizes the
 *    `visualViewport` without changing the layout viewport. We listen to
 *    `visualViewport.resize` events, compute the keyboard height from the
 *    difference between window.innerHeight and visualViewport.height, set a CSS
 *    custom property `--keyboard-height`, and refit all terminals.
 *
 * 3. The VirtualKeyboard API (`navigator.virtualKeyboard`) is a newer standard
 *    that exposes `geometrychange` events. We use it when available for more
 *    accurate keyboard height on supporting browsers.
 *
 * Returns `keyboardHeight` (px) — 0 when keyboard is closed.
 */
export function useVirtualKeyboard(): { keyboardHeight: number } {
  const [keyboardHeight, setKeyboardHeight] = useState(0)

  useEffect(() => {
    // Only relevant on touch devices
    const isTouchDevice =
      typeof window !== 'undefined' && window.matchMedia('(hover: none)').matches
    if (!isTouchDevice) return

    const root = document.documentElement

    const updateHeight = (height: number): void => {
      const rounded = Math.round(height)
      setKeyboardHeight(rounded)
      root.style.setProperty('--keyboard-height', `${rounded}px`)

      // Debounce terminal refit to let the layout settle
      clearTimeout(refitTimer)
      refitTimer = setTimeout(() => fitAllTerminals(), 150)
    }

    let refitTimer: ReturnType<typeof setTimeout>

    // ─── Strategy 1: VirtualKeyboard API (Chrome 94+) ───
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const vk = (navigator as any).virtualKeyboard
    if (vk) {
      vk.overlaysContent = true // Tell browser we handle keyboard layout ourselves
      const onGeometryChange = (): void => {
        const rect = vk.boundingRect as DOMRect
        updateHeight(rect.height)
      }
      vk.addEventListener('geometrychange', onGeometryChange)
      return () => {
        vk.removeEventListener('geometrychange', onGeometryChange)
        clearTimeout(refitTimer)
        root.style.removeProperty('--keyboard-height')
      }
    }

    // ─── Strategy 2: visualViewport resize (Safari/iOS) ───
    const vv = window.visualViewport
    if (vv) {
      // Capture the initial viewport height (no keyboard) for comparison.
      // On iOS, window.innerHeight stays constant; visualViewport.height shrinks
      // when the keyboard opens.
      const initialHeight = window.innerHeight

      const onViewportResize = (): void => {
        // The difference between the layout viewport and visual viewport
        // gives us the keyboard height. We also account for any scroll offset
        // from the viewport being pushed up.
        const kbHeight = Math.max(0, initialHeight - vv.height - vv.offsetTop)
        updateHeight(kbHeight)
      }

      vv.addEventListener('resize', onViewportResize)
      // Also listen to scroll events on the viewport (iOS sometimes scrolls
      // the viewport up instead of resizing it)
      vv.addEventListener('scroll', onViewportResize)

      return () => {
        vv.removeEventListener('resize', onViewportResize)
        vv.removeEventListener('scroll', onViewportResize)
        clearTimeout(refitTimer)
        root.style.removeProperty('--keyboard-height')
      }
    }

    // No API available — nothing to clean up
    return () => {
      clearTimeout(refitTimer)
    }
  }, [])

  return { keyboardHeight }
}
