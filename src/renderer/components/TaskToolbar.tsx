import { useState, useRef, useEffect, useCallback } from 'react'
import { useAppStore } from '../stores'
import { TaskStatusFilter } from '../stores/types'
import { TaskViewMode } from '../../shared/types'
import { TASK_STATUS_LABEL, TASK_STATUS_DOT, TASK_STATUS_ORDER } from '../lib/task-status'
import { SlidersHorizontal, Archive } from 'lucide-react'
import { Tooltip } from './Tooltip'
import { OptionRow } from './OptionRow'
import { ConnectorIcon } from './ConnectorIcon'
import { ToggleSwitch } from './settings/ToggleSwitch'

const isMac = navigator.platform.toUpperCase().includes('MAC')

/* ── Options ─────────────────────────────────────────────────── */

// The filter legend reads the same labels and tones the board does. It used to
// carry its own copy of both, and painted "All" in todo's colour.
const TASK_STATUS_OPTIONS: { value: TaskStatusFilter; label: string; dot: string }[] = [
  { value: 'all', label: 'All', dot: 'bg-ink-secondary' },
  ...TASK_STATUS_ORDER.map((s) => ({
    value: s,
    label: TASK_STATUS_LABEL[s],
    dot: TASK_STATUS_DOT[s]
  }))
]

/* ── Icons ───────────────────────────────────────────────────── */

const ListIcon = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <line x1="8" y1="6" x2="21" y2="6" />
    <line x1="8" y1="12" x2="21" y2="12" />
    <line x1="8" y1="18" x2="21" y2="18" />
    <line x1="3" y1="6" x2="3.01" y2="6" />
    <line x1="3" y1="12" x2="3.01" y2="12" />
    <line x1="3" y1="18" x2="3.01" y2="18" />
  </svg>
)

const KanbanIcon = (
  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <path d="M9 3v18" />
    <path d="M15 3v18" />
  </svg>
)

/* ── Component ───────────────────────────────────────────────── */

export function TaskToolbar() {
  const [open, setOpen] = useState(false)
  const [connectorNames, setConnectorNames] = useState<Record<string, string>>({})
  const ref = useRef<HTMLDivElement>(null)

  const taskStatusFilter = useAppStore((s) => s.taskStatusFilter)
  const setTaskStatusFilter = useAppStore((s) => s.setTaskStatusFilter)
  const taskSourceFilter = useAppStore((s) => s.taskSourceFilter)
  const setTaskSourceFilter = useAppStore((s) => s.setTaskSourceFilter)
  const taskIncludeArchived = useAppStore((s) => s.taskIncludeArchived)
  const setTaskIncludeArchived = useAppStore((s) => s.setTaskIncludeArchived)
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const taskViewMode = (config?.defaults?.taskViewMode ?? 'list') as TaskViewMode

  // Detect which connectors have tasks
  const connectorIds = new Set(
    (config?.tasks || []).map((t) => t.sourceConnectorId).filter(Boolean) as string[]
  )

  // Resolve pretty display names once the filter dropdown is opened — uses
  // connector manifests so we show "GitHub" instead of a naive "Github".
  useEffect(() => {
    if (!open || connectorIds.size === 0) return
    window.api.listConnectors().then((connectors) => {
      const names: Record<string, string> = {}
      for (const c of connectors) names[c.id] = c.name
      setConnectorNames(names)
    })
  }, [open, connectorIds.size])

  const setViewMode = (mode: TaskViewMode): void => {
    if (!config || taskViewMode === mode) return
    const updated = {
      ...config,
      defaults: { ...config.defaults, taskViewMode: mode }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  const hasActiveFilters =
    taskStatusFilter !== 'all' ||
    taskViewMode !== 'list' ||
    taskSourceFilter !== 'all' ||
    taskIncludeArchived

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
                : 'text-ink-secondary hover:text-white hover:bg-white/[0.06]'
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
            <div className="px-3 py-1 text-[10px] text-ink-faint uppercase tracking-wider">
              Status
            </div>
            {TASK_STATUS_OPTIONS.map((opt) => (
              <OptionRow
                key={opt.value}
                selected={taskStatusFilter === opt.value}
                dot={opt.dot}
                label={opt.label}
                onClick={() => setTaskStatusFilter(opt.value)}
              />
            ))}
          </div>

          {/* Source section (only show if connectors have tasks) */}
          {connectorIds.size > 0 && (
            <div className="py-1.5 border-t border-white/[0.06]">
              <div className="px-3 py-1 text-[10px] text-ink-faint uppercase tracking-wider">
                Source
              </div>
              <OptionRow
                selected={taskSourceFilter === 'all'}
                dot="bg-ink-secondary"
                label="All Sources"
                onClick={() => setTaskSourceFilter('all')}
              />
              <OptionRow
                selected={taskSourceFilter === 'local'}
                dot="bg-ink-faint"
                label="Local Only"
                onClick={() => setTaskSourceFilter('local')}
              />
              {[...connectorIds].map((cid) => (
                <button
                  key={cid}
                  onClick={() => setTaskSourceFilter(cid)}
                  className={`w-full text-left px-3 py-1.5 text-[12px] transition-colors flex items-center gap-2 ${
                    taskSourceFilter === cid
                      ? 'text-white bg-white/[0.06]'
                      : 'text-ink-secondary hover:text-white hover:bg-white/[0.04]'
                  }`}
                >
                  {taskSourceFilter === cid ? (
                    <svg
                      width="11"
                      height="11"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="3"
                    >
                      <polyline points="20 6 9 17 4 12" />
                    </svg>
                  ) : (
                    <span className="w-[11px]" />
                  )}
                  <ConnectorIcon
                    connectorId={cid}
                    size={10}
                    className={taskSourceFilter === cid ? 'text-white' : 'text-ink-faint'}
                  />
                  {connectorNames[cid] || cid.charAt(0).toUpperCase() + cid.slice(1)}
                </button>
              ))}
            </div>
          )}

          <div className="py-1.5 border-t border-white/[0.06]">
            <div className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-ink-secondary">
              <Archive size={12} className="text-ink-faint" />
              <span className="flex-1">Include archived</span>
              <ToggleSwitch checked={taskIncludeArchived} onChange={setTaskIncludeArchived} />
            </div>
          </div>

          {/* View section */}
          <div className="py-1.5 border-t border-white/[0.06]">
            <div className="px-3 py-1 text-[10px] text-ink-faint uppercase tracking-wider">
              View
            </div>
            <div className="flex gap-1 px-3 py-1">
              <button
                onClick={() => setViewMode('list')}
                className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] transition-colors ${
                  taskViewMode === 'list'
                    ? 'bg-white/[0.1] text-white'
                    : 'text-ink-faint hover:text-ink-secondary bg-white/[0.04]'
                }`}
              >
                {ListIcon}
                List
              </button>
              <button
                onClick={() => setViewMode('kanban')}
                className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] transition-colors ${
                  taskViewMode === 'kanban'
                    ? 'bg-white/[0.1] text-white'
                    : 'text-ink-faint hover:text-ink-secondary bg-white/[0.04]'
                }`}
              >
                {KanbanIcon}
                Board
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
