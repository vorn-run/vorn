import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, Plus, X } from 'lucide-react'
import { useAnchoredMenu } from '../hooks/useAnchoredMenu'
import { Tooltip } from './Tooltip'

const MENU_ROW_PX = 26
const MENU_WIDTH_PX = 220

export interface PaneTab {
  id: string
  /** Plain name, for the tooltip, the overflow list and the close label. */
  name: string
  /** What the tab draws, when it is more than the name. */
  label?: ReactNode
  title?: string
  icon?: ReactNode
  italic?: boolean
  /** Drawn in place of the close button until the tab is hovered. */
  badge?: ReactNode
  closeLabel?: string
}

interface Props {
  tabs: PaneTab[]
  activeId: string | null
  onSelect: (id: string) => void
  onClose: (id: string) => void
  onDoubleClickTab?: (id: string) => void
  /** Extra per-tab buttons, drawn before the close button. */
  tabActions?: (tab: PaneTab) => ReactNode
  leading?: ReactNode
  /** The pane's own controls, seated at the right of the strip. */
  trailing: ReactNode
  /** Shown in the tab area while there are no tabs. */
  emptyTitle?: string
  onAdd?: () => void
  addLabel?: string
  /** Rows appended to the overflow list; call `close` when one is chosen. */
  menuFooter?: (close: () => void) => ReactNode
  ariaLabel: string
  testId?: string
  draggable?: boolean
  onPointerDown?: (e: React.PointerEvent) => void
  onDoubleClick?: () => void
  /** Keep a press on a tab from reaching the card underneath. */
  isolateTabPointer?: boolean
}

/** The tab strip a pane uses as its title bar: tabs, an overflow list, and the pane controls. */
export function PaneTabStrip({
  tabs,
  activeId,
  onSelect,
  onClose,
  onDoubleClickTab,
  tabActions,
  leading,
  trailing,
  emptyTitle,
  onAdd,
  addLabel = 'New tab',
  menuFooter,
  ariaLabel,
  testId,
  draggable,
  onPointerDown,
  onDoubleClick,
  isolateTabPointer
}: Props): ReactNode {
  const scrollerRef = useRef<HTMLDivElement>(null)
  const activeRef = useRef<HTMLDivElement>(null)
  const [overflowing, setOverflowing] = useState(false)
  const { open, setOpen, toggle, triggerRef, menuRef, position } = useAnchoredMenu({
    estimateHeight: () => (tabs.length + (menuFooter ? 1 : 0)) * MENU_ROW_PX + 12,
    menuWidth: () => MENU_WIDTH_PX
  })

  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    const measure = (): void => setOverflowing(el.scrollWidth > el.clientWidth + 1)
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [tabs])

  useEffect(() => {
    activeRef.current?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
  }, [activeId])

  const stop = isolateTabPointer ? (e: React.PointerEvent): void => e.stopPropagation() : undefined

  return (
    <div
      className={`flex items-center gap-1 pl-1.5 pr-1 pt-1 shrink-0 ${
        draggable ? 'drag-handle cursor-grab active:cursor-grabbing' : ''
      }`}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      data-testid={testId}
    >
      {leading}
      <div
        ref={scrollerRef}
        className="flex items-stretch gap-0.5 flex-1 min-w-0 overflow-x-auto [scrollbar-width:none]"
        role="tablist"
        aria-label={ariaLabel}
      >
        {tabs.length === 0 && emptyTitle && (
          <span className="self-center pl-1 text-[12px] font-medium text-ink">{emptyTitle}</span>
        )}
        {tabs.map((tab) => {
          const isActive = tab.id === activeId
          return (
            <div
              key={tab.id}
              ref={isActive ? activeRef : undefined}
              role="tab"
              aria-selected={isActive}
              tabIndex={0}
              onClick={() => onSelect(tab.id)}
              onDoubleClick={
                onDoubleClickTab
                  ? (e) => {
                      e.stopPropagation()
                      onDoubleClickTab(tab.id)
                    }
                  : undefined
              }
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') onSelect(tab.id)
              }}
              onPointerDown={stop}
              title={tab.title ?? tab.name}
              className={`group/tab flex items-center gap-1 pl-2.5 pr-1 py-1 rounded max-w-[170px] shrink-0
                          cursor-default select-none transition-colors ${
                            isActive
                              ? 'bg-white/[0.06] text-gray-200'
                              : 'text-gray-500 hover:text-gray-300 hover:bg-white/[0.03]'
                          }`}
            >
              {tab.icon}
              <span className={`text-[11px] truncate ${tab.italic ? 'italic' : ''}`}>
                {tab.label ?? tab.name}
              </span>
              {tabActions?.(tab)}
              <span className="relative shrink-0 flex items-center justify-center">
                {tab.badge && (
                  <span className="absolute inset-0 flex items-center justify-center group-hover/tab:hidden">
                    {tab.badge}
                  </span>
                )}
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation()
                    onClose(tab.id)
                  }}
                  aria-label={tab.closeLabel ?? `Close ${tab.name}`}
                  className="p-0.5 rounded text-gray-600 hover:text-white
                             opacity-0 group-hover/tab:opacity-100 focus:opacity-100 transition-opacity"
                >
                  <X size={10} strokeWidth={2.5} />
                </button>
              </span>
            </div>
          )
        })}
      </div>

      {onAdd && (
        <button
          type="button"
          onPointerDown={stop}
          onClick={onAdd}
          aria-label={addLabel}
          className="shrink-0 p-1 rounded text-gray-600 hover:text-gray-200
                     hover:bg-white/[0.06] transition-colors"
        >
          <Plus size={13} strokeWidth={2} />
        </button>
      )}

      {overflowing && (
        <Tooltip label="All open tabs">
          <button
            ref={triggerRef}
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={toggle}
            aria-label="All open tabs"
            aria-haspopup="menu"
            aria-expanded={open}
            className={`shrink-0 p-1 rounded text-ink transition-colors hover:bg-white/[0.10] ${
              open ? 'bg-white/[0.10]' : ''
            }`}
          >
            <ChevronDown size={13} strokeWidth={2} />
          </button>
        </Tooltip>
      )}

      {trailing}

      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            aria-label="All open tabs"
            className="fixed z-[200] border border-white/[0.08] rounded p-1 overflow-y-auto"
            style={{
              background: 'var(--color-surface-overlay)',
              top: position.top,
              bottom: position.bottom,
              left: position.left,
              width: position.width,
              maxHeight: 'calc(100vh - 32px)'
            }}
          >
            {tabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                role="menuitem"
                onClick={() => {
                  onSelect(tab.id)
                  setOpen(false)
                }}
                className={`w-full flex items-center gap-1.5 px-2 h-[26px] rounded text-left text-[12px] ${
                  tab.id === activeId
                    ? 'bg-white/[0.08] text-ink'
                    : 'text-ink-secondary hover:bg-white/[0.05]'
                }`}
              >
                {tab.icon}
                <span className="truncate flex-1">{tab.name}</span>
                {tab.badge}
              </button>
            ))}
            {menuFooter && (
              <div className="mt-1 pt-1 border-t border-white/[0.07]">
                {menuFooter(() => setOpen(false))}
              </div>
            )}
          </div>,
          document.body
        )}
    </div>
  )
}
