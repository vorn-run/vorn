import { useEffect, useRef } from 'react'
import { Pencil, Trash2, Power } from 'lucide-react'

const MENU_WIDTH = 180
const MENU_MAX_HEIGHT = 132
const MENU_MARGIN = 8

function clampToViewport(x: number, y: number) {
  const vw = typeof window !== 'undefined' ? window.innerWidth : 1024
  const vh = typeof window !== 'undefined' ? window.innerHeight : 768
  let left = x
  let top = y
  if (left + MENU_WIDTH + MENU_MARGIN > vw) left = vw - MENU_WIDTH - MENU_MARGIN
  if (top + MENU_MAX_HEIGHT + MENU_MARGIN > vh) top = vh - MENU_MAX_HEIGHT - MENU_MARGIN
  if (left < MENU_MARGIN) left = MENU_MARGIN
  if (top < MENU_MARGIN) top = MENU_MARGIN
  return { left, top }
}

export function WorkflowContextMenu({
  onEdit,
  onDelete,
  onToggleEnabled,
  isScheduled,
  isEnabled,
  onClose,
  x,
  y
}: {
  onEdit: () => void
  onDelete: () => void
  onToggleEnabled?: () => void
  isScheduled?: boolean
  isEnabled?: boolean
  onClose: () => void
  x?: number
  y?: number
}) {
  const menuRef = useRef<HTMLDivElement>(null)
  const useFixed = typeof x === 'number' && typeof y === 'number'

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose()
      }
    }
    document.addEventListener('pointerdown', handleClickOutside)
    return () => document.removeEventListener('pointerdown', handleClickOutside)
  }, [onClose])

  const positionStyle = useFixed
    ? (() => {
        const { left, top } = clampToViewport(x ?? 0, y ?? 0)
        return { position: 'fixed' as const, left, top, minWidth: MENU_WIDTH }
      })()
    : undefined

  return (
    <div
      ref={menuRef}
      className={
        useFixed
          ? 'z-50 py-1 border border-white/[0.08] rounded-lg shadow-xl'
          : 'absolute right-0 top-full mt-1 z-50 min-w-[160px] py-1 border border-white/[0.08] rounded-lg shadow-xl'
      }
      style={{ background: 'var(--color-surface-sunken)', ...positionStyle }}
    >
      <button
        onClick={() => {
          onEdit()
          onClose()
        }}
        className="w-full px-3 py-2 text-left text-[12px] text-gray-300 hover:text-white
                   hover:bg-white/[0.06] active:bg-white/[0.1] flex items-center gap-2 transition-colors"
      >
        <Pencil size={12} strokeWidth={1.5} />
        Edit Workflow
      </button>
      {isScheduled && onToggleEnabled && (
        <button
          onClick={() => {
            onToggleEnabled()
            onClose()
          }}
          className="w-full px-3 py-2 text-left text-[12px] text-gray-300 hover:text-white
                     hover:bg-white/[0.06] active:bg-white/[0.1] flex items-center gap-2 transition-colors"
        >
          <Power size={12} strokeWidth={1.5} />
          {isEnabled ? 'Disable Schedule' : 'Enable Schedule'}
        </button>
      )}
      <button
        onClick={() => {
          onDelete()
          onClose()
        }}
        className="w-full px-3 py-2 text-left text-[12px] text-red-400 hover:text-red-300
                   hover:bg-white/[0.06] active:bg-white/[0.1] flex items-center gap-2 transition-colors"
      >
        <Trash2 size={12} strokeWidth={1.5} />
        Delete Workflow
      </button>
    </div>
  )
}
