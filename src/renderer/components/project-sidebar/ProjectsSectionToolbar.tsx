import { useState, useRef, useEffect, useCallback } from 'react'
import { useAppStore } from '../../stores'
import {
  ProjectSortMode,
  WorktreeSortMode,
  WorktreeFilter,
  SidebarViewMode
} from '../../stores/types'
import { ListFilter } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { OptionRow } from '../OptionRow'

const PROJECT_SORT_OPTIONS: { value: ProjectSortMode; label: string }[] = [
  { value: 'manual', label: 'Manual' },
  { value: 'name', label: 'Name' },
  { value: 'recent', label: 'Recent activity' }
]

const WORKTREE_SORT_OPTIONS: { value: WorktreeSortMode; label: string }[] = [
  { value: 'name', label: 'Name' },
  { value: 'recent', label: 'Recent activity' }
]

const VIEW_MODE_OPTIONS: { value: SidebarViewMode; label: string }[] = [
  { value: 'worktrees', label: 'Projects > Worktrees' },
  { value: 'worktrees-sessions', label: 'Projects > Worktrees > Sessions' },
  { value: 'sessions', label: 'Projects > Sessions' },
  { value: 'sessions-flat', label: 'Sessions (Flat)' }
]

const FILTER_OPTIONS: { value: WorktreeFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'active', label: 'With active sessions' }
]

export function ProjectsSectionToolbar() {
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState({ top: 0, left: 0 })
  const buttonRef = useRef<HTMLButtonElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  const projectSort = useAppStore((s) => s.sidebarProjectSort)
  const worktreeSort = useAppStore((s) => s.sidebarWorktreeSort)
  const worktreeFilter = useAppStore((s) => s.sidebarWorktreeFilter)
  const viewMode = useAppStore((s) => s.sidebarViewMode)
  const setProjectSort = useAppStore((s) => s.setSidebarProjectSort)
  const setWorktreeSort = useAppStore((s) => s.setSidebarWorktreeSort)
  const setWorktreeFilter = useAppStore((s) => s.setSidebarWorktreeFilter)
  const setViewMode = useAppStore((s) => s.setSidebarViewMode)

  const hasNonDefault =
    viewMode !== 'worktrees' ||
    projectSort !== 'manual' ||
    worktreeSort !== 'name' ||
    worktreeFilter !== 'all'

  const toggle = useCallback(() => {
    setOpen((o) => {
      if (!o && buttonRef.current) {
        const rect = buttonRef.current.getBoundingClientRect()
        setPos({ top: rect.bottom + 4, left: rect.left })
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
      <Tooltip label="Filter & sort" position="bottom">
        <button
          ref={buttonRef}
          onClick={toggle}
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
            <span className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-ink-secondary" />
          )}
        </button>
      </Tooltip>

      {open && (
        <div
          ref={dropdownRef}
          className="fixed z-50 w-[200px] border border-white/[0.08] rounded-lg shadow-xl overflow-hidden"
          style={{ background: '#1a1a1e', top: pos.top, left: pos.left }}
        >
          <div className="py-1.5">
            <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">Show</div>
            {VIEW_MODE_OPTIONS.map((opt) => (
              <OptionRow
                key={opt.value}
                selected={viewMode === opt.value}
                label={opt.label}
                onClick={() => setViewMode(opt.value)}
              />
            ))}
          </div>

          {viewMode !== 'sessions-flat' && (
            <div className="py-1.5 border-t border-white/[0.06]">
              <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
                Filter
              </div>
              {FILTER_OPTIONS.map((opt) => (
                <OptionRow
                  key={opt.value}
                  selected={worktreeFilter === opt.value}
                  label={opt.label}
                  onClick={() => setWorktreeFilter(opt.value)}
                />
              ))}
            </div>
          )}

          {viewMode !== 'sessions-flat' && (
            <div className="py-1.5 border-t border-white/[0.06]">
              <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
                Sort projects
              </div>
              {PROJECT_SORT_OPTIONS.map((opt) => (
                <OptionRow
                  key={opt.value}
                  selected={projectSort === opt.value}
                  label={opt.label}
                  onClick={() => setProjectSort(opt.value)}
                />
              ))}
            </div>
          )}

          {viewMode !== 'sessions' && viewMode !== 'sessions-flat' && (
            <div className="py-1.5 border-t border-white/[0.06]">
              <div className="px-3 py-1 text-[10px] text-gray-500 uppercase tracking-wider">
                Sort worktrees
              </div>
              {WORKTREE_SORT_OPTIONS.map((opt) => (
                <OptionRow
                  key={opt.value}
                  selected={worktreeSort === opt.value}
                  label={opt.label}
                  onClick={() => setWorktreeSort(opt.value)}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </>
  )
}
