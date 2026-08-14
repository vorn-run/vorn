import { useState, useRef, useEffect, useCallback } from 'react'
import { useAppStore } from '../stores'
import { SortMode, StatusFilter } from '../stores/types'
import { SlidersHorizontal } from 'lucide-react'
import { Tooltip } from './Tooltip'
import { OptionRow } from './OptionRow'

const isMac = navigator.platform.toUpperCase().includes('MAC')

/* ── Options ─────────────────────────────────────────────────── */

const STATUS_OPTIONS: { value: StatusFilter; label: string; dot: string }[] = [
  { value: 'all', label: 'All', dot: 'bg-gray-400' },
  { value: 'running', label: 'Running', dot: 'bg-green-500' },
  { value: 'waiting', label: 'Waiting', dot: 'bg-yellow-500' },
  { value: 'idle', label: 'Idle', dot: 'bg-gray-500' },
  { value: 'error', label: 'Error', dot: 'bg-red-500' }
]

const SORT_OPTIONS: { value: SortMode; label: string }[] = [
  { value: 'manual', label: 'Manual' },
  { value: 'created', label: 'Created' },
  { value: 'recent', label: 'Recent' }
]

const COLUMN_OPTIONS: { value: string; label: string; title?: string }[] = [
  {
    value: '0',
    label: 'Auto',
    title: 'Adapts columns and rows to viewport and card count'
  },
  { value: '1', label: '1 Column' },
  { value: '2', label: '2 Columns' },
  { value: '3', label: '3 Columns' },
  { value: '4', label: '4 Columns' },
  { value: '-1', label: 'Flexible' }
]

/* ── Icons ───────────────────────────────────────────────────── */

const GridIcon = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <rect x="3" y="3" width="7" height="7" />
    <rect x="14" y="3" width="7" height="7" />
    <rect x="3" y="14" width="7" height="7" />
    <rect x="14" y="14" width="7" height="7" />
  </svg>
)

const TabIcon = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <path d="M3 9h18" />
    <path d="M9 3v6" />
  </svg>
)

/* ── Component ───────────────────────────────────────────────── */

export function GridToolbar() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  const sortMode = useAppStore((s) => s.sortMode)
  const setSortMode = useAppStore((s) => s.setSortMode)
  const statusFilter = useAppStore((s) => s.statusFilter)
  const setStatusFilter = useAppStore((s) => s.setStatusFilter)
  const gridColumns = useAppStore((s) => s.gridColumns)
  const setGridColumns = useAppStore((s) => s.setGridColumns)
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const layoutMode = config?.defaults?.layoutMode ?? 'grid'

  const setLayoutMode = (mode: 'grid' | 'tabs'): void => {
    if (!config || layoutMode === mode) return
    const updated = {
      ...config,
      defaults: { ...config.defaults, layoutMode: mode }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  const hasActiveFilters =
    statusFilter !== 'all' || sortMode !== 'manual' || gridColumns !== 0 || layoutMode !== 'grid'

  const toggle = useCallback(() => setOpen((o) => !o), [])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    // Use timeout to avoid the opening click from immediately closing
    const id = setTimeout(() => document.addEventListener('mousedown', handleClick), 0)
    return () => {
      clearTimeout(id)
      document.removeEventListener('mousedown', handleClick)
    }
  }, [open])

  useEffect(() => {
    window.addEventListener('toggle-view-options', toggle)
    return () => window.removeEventListener('toggle-view-options', toggle)
  }, [toggle])

  return (
    <div className="relative flex items-center" ref={ref}>
      <Tooltip label="View options" shortcut={`${isMac ? '⌘' : 'Ctrl+'}J`} position="bottom">
        <button
          onClick={toggle}
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
                     border border-white/[0.08] rounded-lg shadow-xl overflow-hidden"
          style={{ background: 'var(--color-surface-overlay)' }}
        >
          {/* Status section */}
          <div className="py-1.5">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
              Status
            </div>
            {STATUS_OPTIONS.map((opt) => (
              <OptionRow
                key={opt.value}
                selected={statusFilter === opt.value}
                dot={opt.dot}
                label={opt.label}
                onClick={() => setStatusFilter(opt.value)}
              />
            ))}
          </div>

          {/* Sort section */}
          <div className="py-1.5 border-t border-white/[0.06]">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">Sort</div>
            {SORT_OPTIONS.map((opt) => (
              <OptionRow
                key={opt.value}
                selected={sortMode === opt.value}
                label={opt.label}
                onClick={() => setSortMode(opt.value)}
              />
            ))}
          </div>

          {/* Layout section */}
          <div className="py-1.5 border-t border-white/[0.06]">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
              Layout
            </div>
            <div className="flex gap-1 px-3 py-1">
              <button
                onClick={() => setLayoutMode('grid')}
                className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] transition-colors ${
                  layoutMode === 'grid'
                    ? 'bg-white/[0.1] text-white'
                    : 'text-gray-500 hover:text-gray-300 bg-white/[0.04]'
                }`}
              >
                {GridIcon}
                Grid
              </button>
              <button
                onClick={() => setLayoutMode('tabs')}
                className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] transition-colors ${
                  layoutMode === 'tabs'
                    ? 'bg-white/[0.1] text-white'
                    : 'text-gray-500 hover:text-gray-300 bg-white/[0.04]'
                }`}
              >
                {TabIcon}
                Tabs
              </button>
            </div>
          </div>

          {/* Columns section (grid mode only) */}
          {layoutMode !== 'tabs' && (
            <div className="py-1.5 border-t border-white/[0.06]">
              <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
                Columns
              </div>
              {COLUMN_OPTIONS.map((opt) => (
                <OptionRow
                  key={opt.value}
                  selected={String(gridColumns) === opt.value}
                  label={opt.label}
                  title={opt.title}
                  onClick={() => setGridColumns(Number(opt.value))}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
