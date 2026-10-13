import { useState, useEffect, useRef, useLayoutEffect } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { AnchorRect, calculatePopoverPosition } from '../lib/popover-position'
import { useIsMobile } from '../hooks/useIsMobile'

interface ConfirmPopoverProps {
  /** The trigger element — rendered as a child */
  children: React.ReactElement<{ onClick?: (e: React.MouseEvent) => void }>
  /** Confirmation message */
  message: string
  /** Label for the confirm button */
  confirmLabel?: string
  /** Called when user confirms */
  onConfirm: () => void
  /** Destructive styling (red) */
  destructive?: boolean
}

function readAnchorRect(anchorEl: HTMLElement): AnchorRect {
  const rect = anchorEl.getBoundingClientRect()
  return {
    top: rect.top,
    left: rect.left,
    right: rect.right,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height
  }
}

export function ConfirmPopover({
  children,
  message,
  confirmLabel = 'Confirm',
  onConfirm,
  destructive = true
}: ConfirmPopoverProps) {
  const [open, setOpen] = useState(false)
  const [position, setPosition] = useState({
    top: 0,
    left: 0,
    placement: 'bottom' as 'top' | 'bottom'
  })
  const anchorRef = useRef<HTMLSpanElement | null>(null)
  const popoverRef = useRef<HTMLDivElement>(null)
  const isMobile = useIsMobile()

  const updatePosition = (anchorEl: HTMLElement): void => {
    const popoverRect = popoverRef.current?.getBoundingClientRect()
    const { top, left, placement } = calculatePopoverPosition(
      readAnchorRect(anchorEl),
      {
        width: popoverRect?.width ?? 220,
        height: popoverRect?.height ?? 86
      },
      {
        width: window.innerWidth,
        height: window.innerHeight
      }
    )
    setPosition({ top, left, placement })
  }

  const handleTrigger = (e: React.MouseEvent<HTMLSpanElement>) => {
    e.stopPropagation()
    e.preventDefault()
    const anchorEl = anchorRef.current ?? e.currentTarget
    anchorRef.current = anchorEl
    updatePosition(anchorEl)
    setOpen(true)
  }

  const handleConfirm = () => {
    setOpen(false)
    onConfirm()
  }

  // Close on outside click or Escape
  useEffect(() => {
    if (!open) return

    const handleClick = (e: MouseEvent | TouchEvent) => {
      const target = e instanceof TouchEvent ? e.touches[0]?.target : e.target
      if (popoverRef.current && target && !popoverRef.current.contains(target as Node)) {
        setOpen(false)
      }
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }

    document.addEventListener('pointerdown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('pointerdown', handleClick)
      document.removeEventListener('keydown', handleKey)
    }
  }, [open])

  useLayoutEffect(() => {
    if (!open || !anchorRef.current) return

    let frame = window.requestAnimationFrame(() => {
      if (anchorRef.current) updatePosition(anchorRef.current)
    })

    const handleViewportChange = () => {
      window.cancelAnimationFrame(frame)
      frame = window.requestAnimationFrame(() => {
        if (anchorRef.current) updatePosition(anchorRef.current)
      })
    }
    window.addEventListener('resize', handleViewportChange)
    window.addEventListener('scroll', handleViewportChange, true)

    return () => {
      window.cancelAnimationFrame(frame)
      window.removeEventListener('resize', handleViewportChange)
      window.removeEventListener('scroll', handleViewportChange, true)
    }
  }, [open])

  const motionOffset = position.placement === 'top' ? 4 : -4

  return (
    <>
      <span ref={anchorRef} onClick={handleTrigger} className="inline-flex">
        {children}
      </span>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={popoverRef}
              initial={{ opacity: 0, y: motionOffset, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: motionOffset, scale: 0.96 }}
              transition={{ type: 'spring', stiffness: 500, damping: 30 }}
              className={`fixed z-[150] rounded-lg border border-white/[0.1] p-3 ${isMobile ? '' : 'shadow-2xl'}`}
              style={{
                top: position.top,
                left: position.left,
                transformOrigin: position.placement === 'top' ? 'bottom center' : 'top center',
                background: isMobile ? 'var(--glass-bg)' : 'var(--color-surface-overlay)',
                backdropFilter: isMobile ? 'var(--glass-blur)' : undefined,
                WebkitBackdropFilter: isMobile ? 'var(--glass-blur)' : undefined,
                boxShadow: isMobile ? 'var(--glass-shadow)' : undefined,
                minWidth: 180,
                maxWidth: 'min(280px, calc(100vw - 16px))'
              }}
            >
              <p className="text-xs text-gray-300 mb-3">{message}</p>
              <div className="flex justify-end gap-2">
                <button
                  onClick={() => setOpen(false)}
                  className="px-3 py-2 text-xs text-gray-400 hover:text-gray-200
                             bg-white/[0.04] hover:bg-white/[0.08] active:bg-white/[0.12]
                             rounded-md transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleConfirm}
                  autoFocus
                  className={`px-3 py-2 text-xs font-medium rounded-md transition-colors ${
                    destructive
                      ? 'text-red-400 bg-red-500/10 hover:bg-red-500/20 active:bg-red-500/30 border border-red-500/20'
                      : 'text-white bg-white/[0.1] hover:bg-white/[0.15] active:bg-white/[0.2]'
                  }`}
                >
                  {confirmLabel}
                </button>
              </div>
            </motion.div>
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  )
}
