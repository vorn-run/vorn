import { useState, useRef, useEffect, useCallback } from 'react'
import { useAppStore } from '../../stores'
import { WorkflowFilter } from '../../stores/types'
import { ListFilter } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { OptionRow } from '../OptionRow'

const FILTER_OPTIONS: { value: WorkflowFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'manual', label: 'Manual' },
  { value: 'scheduled', label: 'Scheduled' }
]

export function WorkflowFilterToolbar() {
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState({ top: 0, left: 0 })
  const buttonRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  const filter = useAppStore((s) => s.sidebarWorkflowFilter)
  const setFilter = useAppStore((s) => s.setSidebarWorkflowFilter)

  const hasNonDefault = filter !== 'all'

  const toggle = useCallback(() => {
    setOpen((o) => {
      if (!o && buttonRef.current) {
        const rect = buttonRef.current.getBoundingClientRect()
        const margin = 8
        const menuWidth = 160
        const menuMaxHeight = 140
        const vw = window.innerWidth
        const vh = window.innerHeight
        let left = rect.left
        let top = rect.bottom + 4
        if (left + menuWidth + margin > vw) left = vw - menuWidth - margin
        if (top + menuMaxHeight + margin > vh) top = vh - menuMaxHeight - margin
        if (left < margin) left = margin
        if (top < margin) top = margin
        setPos({ top, left })
      }
      return !o
    })
  }, [])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(e.target as Node)
      ) {
        setOpen(false)
      }
    }
    const id = setTimeout(() => document.addEventListener('mousedown', handleClick), 0)
    return () => {
      clearTimeout(id)
      document.removeEventListener('mousedown', handleClick)
    }
  }, [open])

  return (
    <>
      <Tooltip label="Filter workflows" position="bottom">
        <button
          ref={buttonRef}
          onClick={toggle}
          aria-label="Filter workflows"
          aria-haspopup="menu"
          aria-expanded={open}
          className={`relative p-0.5 rounded transition-colors ${
            open
              ? 'text-white bg-white/[0.08]'
              : hasNonDefault
                ? 'text-white'
                : 'text-gray-600 hover:text-white hover:bg-white/[0.08]'
          }`}
        >
          <ListFilter size={13} strokeWidth={1.5} />
          {hasNonDefault && !open && (
            <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-blue-500" />
          )}
        </button>
      </Tooltip>

      {open && (
        <div
          ref={dropdownRef}
          className="fixed z-50 w-[160px] bg-surface-base border border-white/[0.08] rounded-lg shadow-xl overflow-hidden"
          style={{ top: pos.top, left: pos.left }}
        >
          <div className="py-1.5">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
              Filter
            </div>
            {FILTER_OPTIONS.map((opt) => (
              <OptionRow
                key={opt.value}
                selected={filter === opt.value}
                label={opt.label}
                onClick={() => {
                  setFilter(opt.value)
                  setOpen(false)
                }}
              />
            ))}
          </div>
        </div>
      )}
    </>
  )
}
