import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Check, ChevronDown } from 'lucide-react'

export interface SelectPickerOption {
  value: string
  label: string
  icon?: React.ReactNode
  hint?: string
}

interface Props {
  value: string
  options: SelectPickerOption[]
  onChange: (value: string) => void
  placeholder?: string
  variant?: 'compact' | 'form'
}

export function SelectPicker({
  value,
  options,
  onChange,
  placeholder = 'Select...',
  variant = 'form'
}: Props) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState({ top: 0, left: 0, width: 0 })

  const current = options.find((o) => o.value === value)

  const handleTrigger = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (open) {
      setOpen(false)
      return
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      setPosition({ top: rect.bottom + 4, left: rect.left, width: Math.max(rect.width, 180) })
    }
    setOpen(true)
  }

  const handleSelect = (v: string) => {
    setOpen(false)
    if (v !== value) onChange(v)
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

  return (
    <>
      <button
        ref={triggerRef}
        onClick={handleTrigger}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={current ? current.label : placeholder}
        className={
          variant === 'form'
            ? 'w-full flex items-center gap-2 px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md text-white hover:border-white/[0.2] transition-colors'
            : 'flex items-center gap-1.5 hover:bg-white/[0.04] rounded px-1.5 py-0.5 -mx-1.5 transition-colors text-[12px] text-gray-300'
        }
      >
        {current?.icon && <span className="shrink-0">{current.icon}</span>}
        <span className={`flex-1 text-left truncate ${current ? '' : 'text-gray-600'}`}>
          {current?.label || placeholder}
        </span>
        <ChevronDown
          size={12}
          className={`shrink-0 text-gray-500 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              initial={{ opacity: 0, scale: 0.95, y: -4 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.95, y: -4 }}
              transition={{ type: 'spring', duration: 0.2, bounce: 0.1 }}
              className="fixed z-[200] border border-white/[0.08] rounded-lg shadow-xl overflow-hidden"
              style={{
                background: 'var(--color-surface-overlay)',
                top: position.top,
                left: position.left,
                minWidth: position.width,
                maxHeight: 280
              }}
            >
              <div className="overflow-y-auto py-1">
                {options.map((opt) => (
                  <button
                    key={opt.value}
                    onMouseDown={(e) => {
                      e.preventDefault()
                      e.stopPropagation()
                      handleSelect(opt.value)
                    }}
                    className={`w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left transition-colors ${
                      value === opt.value
                        ? 'text-white bg-white/[0.06]'
                        : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
                    }`}
                  >
                    {value === opt.value ? (
                      <Check size={11} strokeWidth={3} className="shrink-0" />
                    ) : (
                      <span className="w-[11px] shrink-0" />
                    )}
                    {opt.icon && <span className="shrink-0">{opt.icon}</span>}
                    <span className="flex-1 min-w-0 truncate">{opt.label}</span>
                    {opt.hint && (
                      <span className="text-[10px] text-gray-600 shrink-0">{opt.hint}</span>
                    )}
                  </button>
                ))}
              </div>
            </motion.div>
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  )
}
