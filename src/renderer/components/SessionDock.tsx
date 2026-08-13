import { Fragment, useRef, useState } from 'react'
import { ChevronsLeft, ChevronsRight, Layers } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { useVisibleTerminals } from '../hooks/useVisibleTerminals'
import { useWaitingApprovals } from '../hooks/useWaitingApprovals'
import { useOutsideClick } from '../hooks/useOutsideClick'
import { MinimizedPill } from './MinimizedPill'
import { WaitingApprovalPill } from './WaitingApprovalPill'
import { Tooltip } from './Tooltip'

const MAX_INLINE = 4

type DockKind = 'waiting' | 'minimized'

interface DockItem {
  kind: DockKind
  key: string
  node: React.ReactNode
}

interface DockGroup {
  kind: DockKind
  items: DockItem[]
}

const KIND_LABEL: Record<DockKind, string> = {
  waiting: 'Waiting approval',
  minimized: 'Minimized'
}

interface Props {
  // Tab mode treats minimize as a no-op (every session is a tab), so its dock
  // omits the minimized group.
  includeMinimized: boolean
}

export function SessionDock({ includeMinimized }: Props) {
  const { minimizedIds } = useVisibleTerminals()
  const waitingApprovals = useWaitingApprovals()
  const { collapsed, toggleCollapsed } = useAppStore(
    useShallow((s) => ({
      collapsed: s.sessionDockCollapsed,
      toggleCollapsed: s.toggleSessionDockCollapsed
    }))
  )

  const [popoverOpen, setPopoverOpen] = useState(false)
  const popoverRef = useRef<HTMLDivElement>(null)
  useOutsideClick(popoverRef, popoverOpen, () => setPopoverOpen(false))

  const minimizedCount = includeMinimized ? minimizedIds.length : 0
  const total = waitingApprovals.length + minimizedCount
  if (total === 0) return null

  const breakdown: string[] = []
  if (waitingApprovals.length > 0)
    breakdown.push(
      `${waitingApprovals.length} waiting approval${waitingApprovals.length === 1 ? '' : 's'}`
    )
  if (minimizedCount > 0) breakdown.push(`${minimizedCount} minimized`)
  const tooltip = breakdown.join(' · ')

  if (collapsed) {
    return (
      <div ref={popoverRef} className="relative flex items-center titlebar-no-drag">
        <Tooltip label={tooltip} position="bottom">
          <button
            type="button"
            onClick={() => setPopoverOpen((v) => !v)}
            className="inline-flex items-center gap-1.5 h-[26px] px-2
                       rounded-md border border-white/[0.06] bg-surface-raised
                       text-[11px] font-medium text-gray-300
                       hover:text-white hover:border-white/[0.12] transition-colors
                       relative"
            aria-haspopup="dialog"
            aria-expanded={popoverOpen}
            aria-label={tooltip}
          >
            <Layers size={11} strokeWidth={1.5} />
            <span className="font-mono leading-none">{total}</span>
            {waitingApprovals.length > 0 && (
              <span
                aria-hidden="true"
                className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-bronzo"
              />
            )}
          </button>
        </Tooltip>

        {popoverOpen && (
          <DockGroupedPopover
            groups={groupConsecutiveByKind(
              buildItems(waitingApprovals, minimizedIds, includeMinimized)
            )}
            align="left"
            onClose={() => setPopoverOpen(false)}
            footer={
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  setPopoverOpen(false)
                  toggleCollapsed()
                }}
                className="inline-flex items-center gap-1 text-[10px] text-gray-400 hover:text-white transition-colors"
                title="Expand dock"
              >
                <ChevronsRight size={11} strokeWidth={1.75} />
                Expand
              </button>
            }
          />
        )}
      </div>
    )
  }

  const items = buildItems(waitingApprovals, minimizedIds, includeMinimized)
  const inline = items.slice(0, MAX_INLINE)
  const overflow = items.slice(MAX_INLINE)
  const groups = groupConsecutiveByKind(inline)

  return (
    <div className="flex items-center gap-1.5 min-w-0 titlebar-no-drag">
      {groups.map((group, gi) => (
        <Fragment key={`${group.kind}-${gi}`}>
          {gi > 0 && <div className="w-px h-4 bg-white/[0.08] shrink-0" aria-hidden="true" />}
          <div className="flex items-center gap-1.5 min-w-0">
            {group.items.map((item) => (
              <Fragment key={item.key}>{item.node}</Fragment>
            ))}
          </div>
        </Fragment>
      ))}

      {overflow.length > 0 && (
        <div ref={popoverRef} className="relative flex items-center">
          <Tooltip label={`${overflow.length} more in dock`} position="bottom">
            <button
              type="button"
              onClick={() => setPopoverOpen((v) => !v)}
              className="inline-flex items-center justify-center h-[26px] min-w-[28px] px-1.5
                         rounded-md border border-white/[0.06] bg-surface-raised
                         text-[11px] font-medium text-gray-400
                         hover:text-white hover:border-white/[0.12] transition-colors"
              aria-haspopup="dialog"
              aria-expanded={popoverOpen}
              aria-label={`${overflow.length} more dock items`}
            >
              +{overflow.length}
            </button>
          </Tooltip>

          {popoverOpen && (
            <DockGroupedPopover
              groups={groupConsecutiveByKind(overflow)}
              align="right"
              onClose={() => setPopoverOpen(false)}
            />
          )}
        </div>
      )}

      <Tooltip label="Collapse dock" position="bottom">
        <button
          type="button"
          onClick={toggleCollapsed}
          className="inline-flex items-center justify-center h-[22px] w-[22px]
                     rounded text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
          aria-label="Collapse session dock"
        >
          <ChevronsLeft size={12} strokeWidth={1.75} />
        </button>
      </Tooltip>
    </div>
  )
}

function buildItems(
  waitingApprovals: ReturnType<typeof useWaitingApprovals>,
  minimizedIds: string[],
  includeMinimized: boolean
): DockItem[] {
  const items: DockItem[] = waitingApprovals.map((w) => ({
    kind: 'waiting',
    key: `wait-${w.execution.workflowId}-${w.execution.startedAt}-${w.nodeState.nodeId}`,
    node: (
      <WaitingApprovalPill execution={w.execution} nodeState={w.nodeState} workflow={w.workflow} />
    )
  }))
  if (includeMinimized) {
    for (const id of minimizedIds) {
      items.push({
        kind: 'minimized',
        key: `min-${id}`,
        node: <MinimizedPill terminalId={id} />
      })
    }
  }
  return items
}

function groupConsecutiveByKind(items: DockItem[]): DockGroup[] {
  const groups: DockGroup[] = []
  for (const item of items) {
    const last = groups[groups.length - 1]
    if (last && last.kind === item.kind) last.items.push(item)
    else groups.push({ kind: item.kind, items: [item] })
  }
  return groups
}

function DockGroupedPopover({
  groups,
  align,
  onClose,
  footer
}: {
  groups: DockGroup[]
  align: 'left' | 'right'
  onClose: () => void
  footer?: React.ReactNode
}) {
  return (
    <div
      role="dialog"
      aria-label="Dock items"
      onClick={onClose}
      className={`absolute top-full ${align === 'left' ? 'left-0' : 'right-0'} mt-1.5 z-50 p-1.5
                 flex flex-col gap-2 max-h-[60vh] overflow-y-auto min-w-[240px]
                 bg-surface-raised border border-white/[0.08] rounded-md shadow-lg`}
    >
      {footer && (
        <div
          className="flex items-center justify-end px-1 pb-1 mb-0.5 border-b border-white/[0.06]"
          onClick={(e) => e.stopPropagation()}
        >
          {footer}
        </div>
      )}
      {groups.map((group, gi) => (
        <div key={`${group.kind}-${gi}`} className="flex flex-col gap-1">
          {groups.length > 1 && (
            <span
              className={`px-1 text-[9px] font-medium uppercase tracking-wider ${
                group.kind === 'waiting' ? 'text-ink-secondary' : 'text-gray-500'
              }`}
            >
              {KIND_LABEL[group.kind]}
            </span>
          )}
          <div className="flex flex-wrap gap-1.5">
            {group.items.map((item) => (
              <Fragment key={item.key}>{item.node}</Fragment>
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}
