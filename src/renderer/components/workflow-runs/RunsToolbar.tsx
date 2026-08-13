import { useState, useRef, useEffect, useCallback } from 'react'
import { SlidersHorizontal } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { OptionRow } from '../OptionRow'
import { WORKFLOW_STATUS_DOT } from '../../lib/workflow-status'
import type { RunBucket } from '../../stores/types'

const RUN_FILTERS: { key: RunBucket; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'running', label: 'Running' },
  { key: 'waiting', label: 'Waiting' },
  { key: 'error', label: 'Failed' },
  { key: 'success', label: 'Succeeded' }
]

const RUN_FILTER_DOT: Record<RunBucket, string> = {
  all: 'bg-gray-400',
  running: WORKFLOW_STATUS_DOT.running,
  waiting: WORKFLOW_STATUS_DOT.waiting,
  error: WORKFLOW_STATUS_DOT.error,
  success: WORKFLOW_STATUS_DOT.success
}

interface Props {
  filter: RunBucket
  setFilter: (b: RunBucket) => void
}

export function RunsToolbar({ filter, setFilter }: Props) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  const hasActiveFilters = filter !== 'all'

  const toggle = useCallback(() => setOpen((o) => !o), [])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    const id = setTimeout(() => document.addEventListener('mousedown', handleClick), 0)
    return () => {
      clearTimeout(id)
      document.removeEventListener('mousedown', handleClick)
    }
  }, [open])

  return (
    <div className="relative flex items-center" ref={ref}>
      <Tooltip label="View options" position="bottom">
        <button
          onClick={toggle}
          aria-label="Filter runs"
          aria-haspopup="menu"
          aria-expanded={open}
          className={`relative p-1 rounded-md transition-colors ${
            open
              ? 'text-white bg-white/[0.1]'
              : hasActiveFilters
                ? 'text-white bg-white/[0.08] hover:bg-white/[0.12]'
                : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
          }`}
        >
          <SlidersHorizontal size={16} strokeWidth={1.5} />
          {hasActiveFilters && !open && (
            <span className="absolute top-0.5 right-0.5 w-1.5 h-1.5 rounded-full bg-ink-secondary" />
          )}
        </button>
      </Tooltip>

      {open && (
        <div
          className="absolute right-0 top-full mt-1 z-50 w-[200px]
                     border border-white/[0.08] rounded-lg shadow-xl overflow-hidden bg-surface-overlay"
        >
          <div className="py-1.5">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
              Status
            </div>
            {RUN_FILTERS.map((opt) => (
              <OptionRow
                key={opt.key}
                selected={filter === opt.key}
                dot={RUN_FILTER_DOT[opt.key]}
                label={opt.label}
                onClick={() => {
                  setFilter(opt.key)
                  setOpen(false)
                }}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
