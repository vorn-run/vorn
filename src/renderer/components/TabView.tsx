import { useEffect, useState, useRef, useCallback, useMemo } from 'react'
import { createPortal } from 'react-dom'
import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { useVisibleTerminals, compareTerminalIds } from '../hooks/useVisibleTerminals'
import { isTerminalPane, isPromotedCardId } from '../lib/pane-id'
import { usePromotedCardsByOwner, usePromotedCardSubject } from '../hooks/usePromotedCards'
import { PromotedPaneCard } from './PromotedPaneCard'
import { CardSubjectIcon } from './CardSubjectIcon'
import { PaneColumn } from './PaneColumn'
import { usePaneColumnEntries } from '../hooks/usePaneColumnEntries'
import { AgentStatusIcon } from './AgentStatusIcon'
import { TerminalPane } from './TerminalPane'
import { terminalTextIndentPx } from '../lib/terminal-indent'
import { PromptLauncher } from './PromptLauncher'
import { InlineRename } from './InlineRename'
import { CardContextMenu } from './CardContextMenu'
import { CardStatusBar } from './card/CardStatusBar'
import { SessionComposer } from './card/SessionComposer'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import { closeTerminalSession } from '../lib/terminal-close'
import { buildTooltip } from '../lib/tab-tooltip'
import { ConfirmPopover } from './ConfirmPopover'
import { Tooltip } from './Tooltip'
import { toast } from './Toast'
import {
  ChevronDown,
  FolderOpen,
  Globe,
  GripVertical,
  Pencil,
  Plus,
  SquareTerminal,
  X
} from 'lucide-react'
import { toggleTerminalsPanel } from '../lib/session-utils'
import { shouldOfferPane } from '../lib/pane-affordance'
import { GridContextMenu } from './GridContextMenu'
import { AppNavCluster } from './AppNavCluster'
import { GridToolbar } from './GridToolbar'
import { WindowControls } from './WindowControls'
import { RecentSessionsButton } from './RecentSessionsButton'
import { SessionDock } from './SessionDock'
import { HeadlessBadge } from './HeadlessBadge'
import { resolveActiveProject, createSessionFromProject } from '../lib/session-utils'
import { MOD, isMac, isWeb, TRAFFIC_LIGHT_PAD_PX } from '../lib/platform'

const DRAG_THRESHOLD = 5

interface DragState {
  draggingId: string
  startX: number
  offsetX: number
  isDragging: boolean
  pointerX: number
  pointerY: number
}

function getHorizontalDropIndex(
  pointerX: number,
  orderedIds: string[],
  refs: Map<string, HTMLElement>
): number | null {
  let closestIndex: number | null = null
  let closestDist = Infinity

  for (let i = 0; i < orderedIds.length; i++) {
    const el = refs.get(orderedIds[i])
    if (!el) continue
    const rect = el.getBoundingClientRect()
    const cx = rect.left + rect.width / 2
    const dist = Math.abs(pointerX - cx)
    if (dist < closestDist) {
      closestDist = dist
      closestIndex = i
    }
  }

  return closestIndex
}

const TAB_ICON_BTN =
  'w-5 h-5 flex items-center justify-center rounded opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity text-gray-200'

export function TabIconButton({
  label,
  icon,
  onClick,
  hoverClass = 'hover:text-gray-300'
}: {
  label: string
  icon: React.ReactNode
  onClick?: () => void
  hoverClass?: string
}) {
  const handleClick = (e: React.MouseEvent): void => {
    if (onClick) {
      e.stopPropagation()
      onClick()
    }
  }
  return (
    <Tooltip label={label} position="bottom">
      <button
        type="button"
        onClick={handleClick}
        className={`${TAB_ICON_BTN} ${hoverClass}`}
        aria-label={label}
      >
        {icon}
      </button>
    </Tooltip>
  )
}

/* ── Plus-button dropdown (reuses GridContextMenu) ──────────── */

function PlusDropdown({
  position,
  onClose
}: {
  position: { top: number; left: number }
  onClose: () => void
}) {
  return <GridContextMenu position={{ x: position.left, y: position.top }} onClose={onClose} />
}

/* ── TabView ─────────────────────────────────────────────────── */

export function TabView() {
  const { orderedIds, minimizedIds } = useVisibleTerminals()
  const terminalOrder = useAppStore((s) => s.terminalOrder)
  const terminals = useAppStore((s) => s.terminals)
  const sortMode = useAppStore((s) => s.sortMode)
  // Tab mode treats minimize as a no-op — every session shows as a tab. The
  // minimizedTerminals set is preserved so switching back to grid restores
  // the dock pills.
  const cardsByOwner = usePromotedCardsByOwner()

  // What Cmd+1-9 actually indexes. The strip's own order is not it: a minimized
  // session or card still shows a tab here (minimize is a no-op in this layout)
  // while dropping out of the shortcut's list, so a badge numbered by tab
  // position named a shortcut that jumped somewhere else.
  const shortcutIds = useAppStore((s) => s.visibleTerminalIds)

  const allTabIds = useMemo(() => {
    // A session's panes ride along with it rather than claiming tabs — they get
    // their own cells in grid mode, where side-by-side means something.
    //
    // A popped-out card does claim one. It is a thing you switch to, and the
    // whole app should offer one way of switching to a thing: leaving it out
    // made a card unreachable here except through the sidebar, which is not how
    // anything else in this strip works.
    const sessions = [...orderedIds, ...minimizedIds]
      .filter(isTerminalPane)
      .sort((a, b) => compareTerminalIds(a, b, terminals, sortMode, terminalOrder))
    // Beside its owner, not sorted among the sessions: `compareTerminalIds`
    // reads the terminals map, which holds no entry for a card.
    return sessions.flatMap((id) => [id, ...(cardsByOwner.get(id) ?? [])])
  }, [orderedIds, minimizedIds, terminalOrder, terminals, sortMode, cardsByOwner])
  const activeTabId = useAppStore((s) => s.activeTabId)
  const setActiveTabId = useAppStore((s) => s.setActiveTabId)
  const setSelected = useAppStore((s) => s.setSelectedTerminal)
  const statusFilter = useAppStore((s) => s.statusFilter)
  const renamingTerminalId = useAppStore((s) => s.renamingTerminalId)
  const setRenamingTerminalId = useAppStore((s) => s.setRenamingTerminalId)
  const renameTerminal = useAppStore((s) => s.renameTerminal)
  const reorderTerminals = useAppStore((s) => s.reorderTerminals)
  const toggleFilesPane = useAppStore((s) => s.toggleFilesPane)
  const toggleBrowserPane = useAppStore((s) => s.toggleBrowserPane)
  // Only to answer "is this one already open" — a shell keeps the control for a
  // pane it has, so the pane can still be closed.
  const browserPanes = useAppStore((s) => s.browserPanes)
  const terminalsPanes = useAppStore((s) => s.terminalsPanes)
  const tasks = useAppStore((s) => s.config?.tasks)
  const isSidebarOpen = useAppStore((s) => s.isSidebarOpen)
  const domBlocks = useAppStore((s) => s.config?.defaults.domBlockRendering ?? true)

  const [contextMenu, setContextMenu] = useState<{
    terminalId: string
    x: number
    y: number
  } | null>(null)
  const [dragState, setDragState] = useState<DragState | null>(null)
  const [dropTargetIndex, setDropTargetIndex] = useState<number | null>(null)
  const [plusDropdownPos, setPlusDropdownPos] = useState<{ top: number; left: number } | null>(null)
  const tabRefs = useRef<Map<string, HTMLDivElement>>(new Map())

  const isFiltered = statusFilter !== 'all'
  const isManualSort = sortMode === 'manual'
  const needsTrafficLightPad = isMac && !isSidebarOpen && !isWeb

  const assignedTaskBySessionId = useMemo(() => {
    type Task = NonNullable<typeof tasks>[number]
    const map = new Map<string, Task>()
    for (const t of tasks ?? []) {
      if (t.status === 'in_progress' && t.assignedSessionId) map.set(t.assignedSessionId, t)
    }
    return map
  }, [tasks])

  useEffect(() => {
    if (allTabIds.length === 0) {
      if (activeTabId !== null) setActiveTabId(null)
      return
    }
    if (!activeTabId || !allTabIds.includes(activeTabId)) {
      setActiveTabId(allTabIds[0])
    }
  }, [allTabIds, activeTabId, setActiveTabId])

  const handleSelectTab = (id: string): void => {
    setActiveTabId(id)
    setSelected(id)
  }

  const handleCloseTab = async (id: string): Promise<void> => {
    const state = useAppStore.getState()

    // A card's tab closes the card. Routing it through closeTerminalSession
    // would tear down the whole session the card came from — every terminal,
    // every pane — on a click that says "close this file".
    if (isPromotedCardId(id)) {
      if (activeTabId === id) {
        const idx = allTabIds.indexOf(id)
        setActiveTabId(allTabIds[idx + 1] ?? allTabIds[idx - 1] ?? null)
      }
      state.closeCard(id)
      return
    }

    const terminal = terminals.get(id)
    const name = terminal ? getDisplayName(terminal.session) : id

    if (state.focusedTerminalId === id) state.setFocusedTerminal(null)

    // Auto-select adjacent tab before removing
    if (activeTabId === id) {
      const idx = allTabIds.indexOf(id)
      const nextId = allTabIds[idx + 1] ?? allTabIds[idx - 1] ?? null
      setActiveTabId(nextId)
    }

    await closeTerminalSession(id)
    toast.success(`Session "${name}" closed`)
  }

  /* ── Drag handlers ─────────────────────────────────────────── */

  const handleDragStart = useCallback(
    (terminalId: string, e: React.PointerEvent) => {
      if (!isManualSort) return
      if (e.button !== 0) return
      const el = tabRefs.current.get(terminalId)
      const rect = el?.getBoundingClientRect()
      setDragState({
        draggingId: terminalId,
        startX: e.clientX,
        offsetX: rect ? e.clientX - rect.left : 0,
        isDragging: false,
        pointerX: e.clientX,
        pointerY: e.clientY
      })
    },
    [isManualSort]
  )

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!dragState) return
      const dx = e.clientX - dragState.startX
      if (!dragState.isDragging && Math.abs(dx) < DRAG_THRESHOLD) return
      if (!dragState.isDragging) {
        setDragState((prev) =>
          prev ? { ...prev, isDragging: true, pointerX: e.clientX, pointerY: e.clientY } : prev
        )
      } else {
        setDragState((prev) =>
          prev ? { ...prev, pointerX: e.clientX, pointerY: e.clientY } : prev
        )
      }
      const targetIndex = getHorizontalDropIndex(e.clientX, allTabIds, tabRefs.current)
      setDropTargetIndex(targetIndex)
    },
    [dragState, allTabIds]
  )

  const handlePointerUp = useCallback(() => {
    if (dragState?.isDragging && dropTargetIndex !== null) {
      const droppedOnId = allTabIds[dropTargetIndex]
      if (droppedOnId && droppedOnId !== dragState.draggingId) {
        reorderTerminals(dragState.draggingId, droppedOnId)
      }
    }
    setDragState(null)
    setDropTargetIndex(null)
  }, [dragState, dropTargetIndex, allTabIds, reorderTerminals])

  const handlePointerCancel = useCallback(() => {
    setDragState(null)
    setDropTargetIndex(null)
  }, [])

  /* ── Render ─────────────────────────────────────────────────── */

  const hasTabs = allTabIds.length > 0
  const activeTerminal = activeTabId ? terminals.get(activeTabId) : null
  // The column's own answer, not a second reading of the same flags. A gate
  // that could disagree with the column left a 380px sidebar of dead air with
  // nothing on screen to say what was holding it open — and it had already been
  // wrong once before, for the device pane, in silence.
  const activeHasPanes = usePaneColumnEntries(activeTabId).length > 0

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div
        className="titlebar-drag shrink-0 flex items-center border-b border-white/[0.06] bg-surface-overlay relative z-[46]"
        style={{
          minHeight: 40,
          paddingLeft: needsTrafficLightPad ? `${TRAFFIC_LIGHT_PAD_PX}px` : undefined
        }}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={handlePointerUp}
        onPointerCancel={handlePointerCancel}
      >
        {!isSidebarOpen && (
          <div className="pl-2 flex items-center">
            <AppNavCluster />
          </div>
        )}

        <div
          className="flex-1 flex items-end gap-1 px-1 overflow-x-auto min-w-0"
          style={{ minHeight: 40 }}
        >
          {allTabIds.map((id, index) => {
            if (isPromotedCardId(id)) {
              return (
                <CardTab
                  key={id}
                  cardId={id}
                  shortcutIndex={shortcutIds.indexOf(id)}
                  registerRef={(el) => {
                    if (el) tabRefs.current.set(id, el)
                    else tabRefs.current.delete(id)
                  }}
                  isActive={id === activeTabId}
                  onSelect={() => handleSelectTab(id)}
                  onClose={() => void handleCloseTab(id)}
                />
              )
            }
            const terminal = terminals.get(id)
            if (!terminal) return null
            const isActive = id === activeTabId
            const shortcutIndex = shortcutIds.indexOf(id)
            const isRenaming = renamingTerminalId === id
            const isDragTarget = dragState?.isDragging === true && dropTargetIndex === index
            const isDragging = dragState?.isDragging === true && dragState.draggingId === id

            const assignedTask = assignedTaskBySessionId.get(id)
            const displayName = terminal.session.displayName?.trim()
              ? getDisplayName(terminal.session)
              : assignedTask
                ? assignedTask.title
                : getDisplayName(terminal.session)

            const tooltipTaskTitle =
              displayName === assignedTask?.title ? undefined : assignedTask?.title
            const isShell = terminal.session.agentType === 'shell'
            const tooltip = buildTooltip(
              displayName,
              terminal.status,
              isShell ? undefined : getBranchLabel(terminal.session),
              isShell ? undefined : terminal.session.isWorktree,
              isShell ? undefined : tooltipTaskTitle,
              isShell ? terminal.session.shellCwd : undefined,
              isShell ? terminal.session.shellExitCode : undefined,
              terminal.session.projectName
            )

            return (
              <div
                key={id}
                role="tab"
                tabIndex={0}
                ref={(el) => {
                  if (el) tabRefs.current.set(id, el)
                  else tabRefs.current.delete(id)
                }}
                onClick={() => handleSelectTab(id)}
                onContextMenu={(e) => {
                  e.preventDefault()
                  e.stopPropagation()
                  setContextMenu({ terminalId: id, x: e.clientX, y: e.clientY })
                }}
                data-testid={`tab-${id}`}
                className={`titlebar-no-drag group relative flex items-center gap-2 pl-3 pr-10 h-[36px] text-[13px] cursor-pointer
                           transition-colors flex-1 min-w-[120px] max-w-[260px] select-none border-b
                           ${isDragTarget ? 'ring-1 ring-blue-500/50' : ''}
                           ${isDragging ? 'opacity-50' : ''}
                           ${
                             isActive
                               ? 'text-white border-white'
                               : 'text-gray-500 hover:text-gray-300 border-transparent'
                           }`}
              >
                {/* Drag handle — left edge, only in manual sort */}
                {isManualSort && (
                  <span
                    className={`absolute left-0 top-0 bottom-0 w-[14px] flex items-center justify-center
                               opacity-0 group-hover:opacity-100 transition-opacity z-10
                               ${isDragging ? 'cursor-grabbing opacity-100' : 'cursor-grab'}`}
                    onPointerDown={(e) => {
                      e.stopPropagation()
                      handleDragStart(id, e)
                    }}
                  >
                    <GripVertical size={8} className="text-gray-600" strokeWidth={2} />
                  </span>
                )}

                <AgentStatusIcon
                  agentType={terminal.session.agentType}
                  status={terminal.status}
                  size={16}
                />

                {isRenaming ? (
                  <InlineRename
                    value={getDisplayName(terminal.session)}
                    onCommit={(name) => {
                      renameTerminal(id, name)
                      setRenamingTerminalId(null)
                      toast.success(`Renamed to "${name}"`)
                    }}
                    onCancel={() => setRenamingTerminalId(null)}
                    className="text-xs w-[100px]"
                  />
                ) : (
                  <span className="truncate flex-1 min-w-0" title={tooltip}>
                    {displayName}
                  </span>
                )}

                <span className="absolute right-2 top-0 bottom-0 flex items-center gap-1 group-hover:bg-surface-overlay group-hover:rounded-l-sm">
                  {shortcutIndex !== -1 && shortcutIndex < 9 && !isRenaming && (
                    <span
                      className="absolute right-0 top-1/2 -translate-y-1/2
                                   opacity-100 group-hover:opacity-0 transition-opacity
                                   px-1 py-0.5 text-[9px] font-mono text-gray-500
                                   bg-white/[0.06] border border-white/[0.1] rounded leading-none
                                   pointer-events-none"
                    >
                      {MOD}
                      {shortcutIndex + 1}
                    </span>
                  )}
                  {!isRenaming && (
                    <>
                      <TabIconButton
                        label="Browse files"
                        icon={<FolderOpen size={13} />}
                        onClick={() => toggleFilesPane(id)}
                      />
                      {shouldOfferPane(terminal.session.agentType, terminalsPanes.has(id)) && (
                        <TabIconButton
                          label={terminalsPanes.has(id) ? 'Close terminals' : 'Add a terminal'}
                          icon={<SquareTerminal size={13} />}
                          onClick={() => void toggleTerminalsPanel(id)}
                        />
                      )}
                      {shouldOfferPane(terminal.session.agentType, browserPanes.has(id)) && (
                        <TabIconButton
                          label="Open browser"
                          icon={<Globe size={13} />}
                          onClick={() => toggleBrowserPane(id)}
                        />
                      )}
                      <TabIconButton
                        label="Rename session"
                        icon={<Pencil size={13} />}
                        onClick={() => setRenamingTerminalId(id)}
                      />
                    </>
                  )}
                  <ConfirmPopover
                    message="Close this session?"
                    confirmLabel="Close"
                    onConfirm={() => handleCloseTab(id)}
                  >
                    <TabIconButton
                      label="Close session"
                      icon={<X size={13} strokeWidth={2} />}
                      hoverClass="hover:text-gray-200 hover:bg-white/[0.1]"
                    />
                  </ConfirmPopover>
                </span>
              </div>
            )
          })}

          <div className="titlebar-no-drag shrink-0 flex items-center">
            <button
              onClick={() => {
                const project = resolveActiveProject()
                if (!project) {
                  useAppStore.getState().setNewAgentDialogOpen(true)
                  return
                }
                const activeWorktreePath = useAppStore.getState().activeWorktreePath
                const activeWt = activeWorktreePath
                  ? useAppStore
                      .getState()
                      .worktreeCache.get(project.path)
                      ?.find((wt) => wt.path === activeWorktreePath)
                  : undefined
                void createSessionFromProject(
                  project,
                  activeWorktreePath
                    ? { branch: activeWt?.branch, existingWorktreePath: activeWorktreePath }
                    : {}
                )
              }}
              className="h-[36px] w-[28px] flex items-center justify-center rounded-md
                         text-gray-200 hover:text-white hover:bg-white/[0.10] transition-colors"
              title="New session"
              aria-label="New session"
            >
              <Plus size={14} strokeWidth={1.5} />
            </button>
            <button
              onClick={(e) => {
                if (plusDropdownPos) {
                  setPlusDropdownPos(null)
                  return
                }
                const rect = e.currentTarget.getBoundingClientRect()
                const menuWidth = 220
                setPlusDropdownPos({
                  left: Math.max(
                    8,
                    Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - 8)
                  ),
                  top: rect.bottom + 4
                })
              }}
              className="h-[36px] w-[22px] flex items-center justify-center rounded-md
                         text-gray-200 hover:text-white hover:bg-white/[0.10] transition-colors"
              title="More session options"
              aria-label="More session options"
            >
              <ChevronDown size={12} strokeWidth={1.5} />
            </button>
          </div>

          {plusDropdownPos && (
            <PlusDropdown position={plusDropdownPos} onClose={() => setPlusDropdownPos(null)} />
          )}
        </div>

        <div className="shrink-0 flex items-center gap-1 px-2 titlebar-no-drag">
          <SessionDock includeMinimized={false} />
          <HeadlessBadge align="right" />
          <GridToolbar />
          <div className="w-px h-4 bg-white/[0.06] mx-0.5" />
          <RecentSessionsButton />
          <WindowControls />
        </div>
      </div>

      {/* Content area */}
      {!hasTabs ? (
        <div className="flex-1 overflow-auto p-4">
          {isFiltered ? (
            <div className="flex flex-col items-center justify-center h-full">
              <svg
                width="64"
                height="64"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
                className="text-white/20 mb-6"
              >
                <rect x="2" y="3" width="20" height="14" rx="2" />
                <path d="M8 21h8M12 17v4" />
                <path d="M7 8l3 3-3 3M12 14h4" />
              </svg>
              <p className="text-2xl font-semibold text-white mb-2">No matching agents</p>
              <p className="text-sm text-gray-500 mb-6">Try changing the status filter</p>
            </div>
          ) : (
            <PromptLauncher mode="inline" />
          )}
        </div>
      ) : activeTabId && isPromotedCardId(activeTabId) ? (
        // A card's tab shows the card, and only the card. The chrome below —
        // terminal, intent bar, status bar, pane column — all belongs to a
        // session, and a card is not one.
        <div className="flex-1 min-h-0 flex" style={{ background: 'var(--color-surface-sunken)' }}>
          <PromotedPaneCard cardId={activeTabId} />
        </div>
      ) : (
        <div className="flex-1 min-h-0 flex" style={{ background: 'var(--color-surface-sunken)' }}>
          <div className="flex-1 min-w-0 flex flex-col">
            <div className="relative flex-1 min-h-0">
              {activeTabId && activeTerminal && (
                <TerminalPane
                  key={activeTabId}
                  terminalId={activeTabId}
                  agentType={activeTerminal.session.agentType}
                  isFocused={true}
                  domBlocks={domBlocks}
                />
              )}
              {activeTabId && activeTerminal && activeTerminal.lastOutputTimestamp === 0 && (
                <div
                  className="absolute inset-0 p-3 space-y-2 pointer-events-none"
                  style={{ background: 'var(--color-surface-sunken)' }}
                >
                  <div className="h-3 w-3/4 rounded bg-white/[0.04] animate-pulse" />
                  <div
                    className="h-3 w-1/2 rounded bg-white/[0.04] animate-pulse"
                    style={{ animationDelay: '0.15s' }}
                  />
                  <div
                    className="h-3 w-5/6 rounded bg-white/[0.04] animate-pulse"
                    style={{ animationDelay: '0.3s' }}
                  />
                  <div
                    className="h-3 w-2/3 rounded bg-white/[0.04] animate-pulse"
                    style={{ animationDelay: '0.45s' }}
                  />
                </div>
              )}
            </div>
            {activeTabId && activeTerminal && (
              <SessionComposer
                terminalId={activeTabId}
                indentPx={terminalTextIndentPx(activeTerminal.session.agentType, domBlocks)}
              />
            )}
            {activeTabId && activeTerminal && <CardStatusBar terminalId={activeTabId} />}
          </div>

          {/* The active session's file panes, alongside its terminal. */}
          {activeTabId && activeTerminal && activeHasPanes && (
            <div className="w-[380px] shrink-0 flex flex-col border-l border-white/[0.06]">
              <PaneColumn sessionId={activeTabId} />
            </div>
          )}
        </div>
      )}

      {contextMenu && (
        <CardContextMenu
          terminalId={contextMenu.terminalId}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
        />
      )}
      {dragState?.isDragging &&
        (() => {
          const terminal = terminals.get(dragState.draggingId)
          if (!terminal) return null
          const displayName = getDisplayName(terminal.session)
          return createPortal(
            <motion.div
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 0.9, scale: 1 }}
              className="fixed flex items-center gap-1.5 px-2.5 h-[28px] rounded-md text-xs
                       bg-white/[0.1] text-white border border-white/[0.12] pointer-events-none"
              style={{
                left: dragState.pointerX - dragState.offsetX,
                top: dragState.pointerY - 14,
                zIndex: 9999,
                boxShadow: '0 4px 16px rgba(0,0,0,0.4)'
              }}
            >
              <AgentStatusIcon
                agentType={terminal.session.agentType}
                status={terminal.status}
                size={12}
              />
              <span className="truncate max-w-[160px]">{displayName}</span>
            </motion.div>,
            document.body
          )
        })()}
    </div>
  )
}

/**
 * A popped-out card's tab.
 *
 * Deliberately not the session chip with pieces removed. That chip carries an
 * agent icon, a status, an assigned task, rename, and toggles for the session's
 * file and browser panes — a card has none of those, and a file is never
 * "running". What it shares is the part that matters: it sits in the strip, it
 * switches on click, it closes on the ✕, and it takes the same Cmd+N position.
 */
function CardTab({
  cardId,
  shortcutIndex,
  isActive,
  registerRef,
  onSelect,
  onClose
}: {
  cardId: string
  /** Position in the Cmd+1-9 list, or -1 when no shortcut reaches this tab. */
  shortcutIndex: number
  isActive: boolean
  /**
   * Card tabs carry no drag handle — a card has no position of its own — but
   * they must still measure, or the drop indicator jumps over their slot while
   * a session tab is dragged across them.
   */
  registerRef: (el: HTMLDivElement | null) => void
  onSelect: () => void
  onClose: () => void
}) {
  const subject = usePromotedCardSubject(cardId)

  if (!subject) return null

  return (
    <div
      role="tab"
      tabIndex={0}
      aria-selected={isActive}
      ref={registerRef}
      data-testid={`tab-${cardId}`}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelect()
        }
      }}
      className={`titlebar-no-drag group relative flex items-center gap-2 pl-3 pr-10 h-[36px] text-[13px]
                 cursor-pointer transition-colors flex-1 min-w-[120px] max-w-[260px] select-none border-b
                 ${isActive ? 'text-white border-white' : 'text-gray-500 hover:text-gray-300 border-transparent'}`}
    >
      <CardSubjectIcon card={subject} />

      <span className="truncate flex-1 min-w-0" title={subject.name}>
        {subject.name}
      </span>

      <span className="absolute right-2 top-0 bottom-0 flex items-center gap-1 group-hover:bg-surface-overlay group-hover:rounded-l-sm">
        {shortcutIndex !== -1 && shortcutIndex < 9 && (
          <span
            className="absolute right-0 top-1/2 -translate-y-1/2
                       opacity-100 group-hover:opacity-0 transition-opacity
                       px-1 py-0.5 text-[9px] font-mono text-gray-500
                       bg-white/[0.06] border border-white/[0.1] rounded leading-none
                       pointer-events-none"
          >
            {MOD}
            {shortcutIndex + 1}
          </span>
        )}
        {/* No confirm popover, unlike a session's tab: closing a card is not
            ending a running agent. It still asks about an unsaved buffer —
            `closeCard` owns that question, so every ✕ that reaches a card asks
            it the same way. */}
        <TabIconButton
          label={`Close ${subject.name}`}
          icon={<X size={13} strokeWidth={2} />}
          onClick={onClose}
          hoverClass="hover:text-gray-200 hover:bg-white/[0.1]"
        />
      </span>
    </div>
  )
}
