import { useEffect, useRef, useState } from 'react'

/** Only used to choose a direction; the flipped menu is anchored by its edge. */
const MENU_GAP_PX = 4
const VIEWPORT_MARGIN_PX = 8

export interface AnchoredMenuPosition {
  top?: number
  bottom?: number
  left: number
  width: number
}

interface Options {
  /** A guess at the open menu's height, deciding whether it drops down or flips up. */
  estimateHeight: () => number
  /** The menu's width for a trigger of this width; the trigger's own by default. */
  menuWidth?: (triggerWidth: number) => number
}

/** A portal menu anchored to its trigger: flips up near the bottom, stays on screen, closes on outside click or Escape. */
export function useAnchoredMenu({ estimateHeight, menuWidth }: Options) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<AnchoredMenuPosition>({ top: 0, left: 0, width: 0 })

  const toggle = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (open) {
      setOpen(false)
      return
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      const width = Math.min(menuWidth?.(rect.width) ?? rect.width, window.innerWidth - 16)
      const estimated = estimateHeight()
      const flipUp =
        rect.bottom + MENU_GAP_PX + estimated > window.innerHeight - VIEWPORT_MARGIN_PX &&
        rect.top - MENU_GAP_PX - estimated > VIEWPORT_MARGIN_PX
      setPosition({
        top: flipUp ? undefined : rect.bottom + MENU_GAP_PX,
        bottom: flipUp ? window.innerHeight - rect.top + MENU_GAP_PX : undefined,
        left: Math.max(
          VIEWPORT_MARGIN_PX,
          Math.min(rect.left, window.innerWidth - width - VIEWPORT_MARGIN_PX)
        ),
        width
      })
    }
    setOpen(true)
  }

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target)) return
      if (menuRef.current && !menuRef.current.contains(target)) setOpen(false)
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleKey)
    }
  }, [open])

  return { open, setOpen, toggle, triggerRef, menuRef, position }
}
