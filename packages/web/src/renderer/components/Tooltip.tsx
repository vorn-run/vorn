import { useState, useRef, useCallback, useEffect } from 'react'
import { createPortal } from 'react-dom'

interface Props {
  label: string
  shortcut?: string
  children: React.ReactNode
  position?: 'top' | 'bottom' | 'right'
  delay?: number
}

// Detect touch-primary device (no fine pointer = mobile/tablet)
const isTouchDevice =
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(hover: none)').matches

function getTransform(position: 'top' | 'bottom' | 'right') {
  if (position === 'right') return 'translate(0, -50%)'
  return position === 'top' ? 'translate(-50%, -100%)' : 'translate(-50%, 0)'
}

export function Tooltip({ label, shortcut, children, position = 'top', delay = 400 }: Props) {
  const [visible, setVisible] = useState(false)
  const timeout = useRef<ReturnType<typeof setTimeout> | null>(null)
  const wrapperRef = useRef<HTMLDivElement>(null)
  const tooltipRef = useRef<HTMLDivElement>(null)

  const show = useCallback(() => {
    // Skip tooltips entirely on touch devices — they interfere with taps
    if (isTouchDevice) return
    timeout.current = setTimeout(() => setVisible(true), delay)
  }, [delay])

  const hide = useCallback(() => {
    if (timeout.current) clearTimeout(timeout.current)
    setVisible(false)
  }, [])

  // Hide tooltip on scroll (prevents stale positioned tooltips)
  useEffect(() => {
    if (!visible) return
    const handleScroll = () => hide()
    window.addEventListener('scroll', handleScroll, true)
    return () => window.removeEventListener('scroll', handleScroll, true)
  }, [visible, hide])

  // Position the tooltip via ref callback (no setState in effect)
  const positionTooltip = useCallback(
    (node: HTMLDivElement | null) => {
      tooltipRef.current = node
      if (!node || !wrapperRef.current) return
      const rect = wrapperRef.current.getBoundingClientRect()
      if (position === 'right') {
        node.style.left = `${rect.right + 6}px`
        node.style.top = `${rect.top + rect.height / 2}px`
      } else {
        node.style.left = `${rect.left + rect.width / 2}px`
        node.style.top = `${position === 'top' ? rect.top - 6 : rect.bottom + 6}px`
      }
    },
    [position]
  )

  return (
    <div
      ref={wrapperRef}
      className="inline-flex items-center"
      onMouseEnter={show}
      onMouseLeave={hide}
    >
      {children}
      {visible &&
        createPortal(
          <div
            ref={positionTooltip}
            className="fixed z-[100] px-2 py-1 rounded-md text-[11px] text-gray-200 whitespace-nowrap
                     border border-white/[0.08] shadow-lg pointer-events-none"
            style={{
              background: 'var(--color-surface-overlay)',
              transform: getTransform(position)
            }}
          >
            {label}
            {shortcut && (
              <span className="ml-2 text-gray-500 font-mono text-[10px]">{shortcut}</span>
            )}
          </div>,
          document.body
        )}
    </div>
  )
}
