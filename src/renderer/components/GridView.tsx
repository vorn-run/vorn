import { memo, useRef, useState, useCallback, useMemo, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { useShallow } from 'zustand/react/shallow'
import { motion } from 'framer-motion'
import { GridLayout, noCompactor, type EventCallback, type Layout } from 'react-grid-layout'
import 'react-grid-layout/css/styles.css'
import 'react-resizable/css/styles.css'
import { useAppStore } from '../stores'
import { PaneRenderer } from './PaneRenderer'
import { parsePaneId } from '../lib/pane-id'
import { PromptLauncher } from './PromptLauncher'
import { GridContextMenu } from './GridContextMenu'
import { AgentIcon } from './AgentIcon'
import { useVisibleTerminals } from '../hooks/useVisibleTerminals'
import { useIsMobile } from '../hooks/useIsMobile'
import { resolveActiveProject } from '../lib/session-utils'
import { getDisplayName, getBranchLabel } from '../lib/terminal-display'
import type { TerminalState, FlexibleLayoutRect } from '../stores/types'
import { GitBranch, FolderGit2 } from 'lucide-react'
import { pickAutoLayout, AUTO_MIN_CARD_H, fitMaxRows } from '../lib/auto-grid-layout'

interface DragState {
  draggingId: string
  startX: number
  startY: number
  offsetX: number
  offsetY: number
  isDragging: boolean
  pointerX: number
  pointerY: number
  width: number
}

function useContainerSize(): {
  size: { width: number; height: number } | null
  setNode: (el: HTMLElement | null) => void
} {
  const [size, setSize] = useState<{ width: number; height: number } | null>(null)
  const observerRef = useRef<ResizeObserver | null>(null)
  // Round to integer px and skip state updates when dimensions haven't
  // actually changed — ResizeObserver fires on every subpixel shift, which
  // would otherwise thrash pickAutoLayout downstream.
  const updateSize = useCallback((w: number, h: number) => {
    if (w <= 0 || h <= 0) return
    const width = Math.round(w)
    const height = Math.round(h)
    setSize((prev) =>
      prev && prev.width === width && prev.height === height ? prev : { width, height }
    )
  }, [])
  const setNode = useCallback(
    (el: HTMLElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!el) return
      const r = el.getBoundingClientRect()
      updateSize(r.width, r.height)
      // Older runtimes (and some test environments) don't ship ResizeObserver.
      // The one-shot getBoundingClientRect above is enough to kick off smart
      // Auto; live resize updates are best-effort.
      if (typeof ResizeObserver === 'undefined') return
      const observer = new ResizeObserver((entries) => {
        const rect = entries[0]?.contentRect
        if (rect) updateSize(rect.width, rect.height)
      })
      observer.observe(el)
      observerRef.current = observer
    },
    [updateSize]
  )
  useEffect(() => () => observerRef.current?.disconnect(), [])
  return { size, setNode }
}

export const GridView = memo(function GridView() {
  const { gridColumns, sortMode, statusFilter, reorderTerminals, rowHeight } = useAppStore(
    useShallow((s) => ({
      gridColumns: s.gridColumns,
      sortMode: s.sortMode,
      statusFilter: s.statusFilter,
      reorderTerminals: s.reorderTerminals,
      rowHeight: s.rowHeight
    }))
  )
  const [dragState, setDragState] = useState<DragState | null>(null)
  const [dropTargetIndex, setDropTargetIndex] = useState<number | null>(null)
  const [gridContextMenu, setGridContextMenu] = useState<{ x: number; y: number } | null>(null)
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map())
  const { size: wrapperSize, setNode: setGridWrapperNode } = useContainerSize()

  const terminals = useAppStore((s) => s.terminals)
  const { orderedIds: allOrderedIds } = useVisibleTerminals()
  const maximizedPaneId = useAppStore((s) => s.maximizedPaneId)

  // Session-scoped maximize: the maximized pane takes over its *owner session's*
  // footprint, so only that session's sibling panes drop out. Other sessions keep
  // rendering, which is what makes maximize usable for side-by-side comparison.
  const orderedIds = useMemo(() => {
    if (!maximizedPaneId || !allOrderedIds.includes(maximizedPaneId)) return allOrderedIds
    const owner = parsePaneId(maximizedPaneId).sessionId
    return allOrderedIds.filter(
      (id) => id === maximizedPaneId || parsePaneId(id).sessionId !== owner
    )
  }, [allOrderedIds, maximizedPaneId])

  // How many grid cells the maximized pane absorbs, so it can span them.
  const maximizedSpan = useMemo(() => {
    if (!maximizedPaneId || !allOrderedIds.includes(maximizedPaneId)) return 1
    const owner = parsePaneId(maximizedPaneId).sessionId
    return allOrderedIds.filter((id) => parsePaneId(id).sessionId === owner).length
  }, [allOrderedIds, maximizedPaneId])

  const isMobile = useIsMobile()

  const isFiltered = statusFilter !== 'all'

  const isSmartAuto = !isMobile && gridColumns === 0
  // Only wire the ResizeObserver when smart-auto is active. In fixed-column
  // or mobile modes the wrapper size isn't used, so there's no reason to
  // trigger re-renders on every window resize.
  const attachWrapperRef = useCallback(
    (el: HTMLElement | null) => setGridWrapperNode(isSmartAuto ? el : null),
    [isSmartAuto, setGridWrapperNode]
  )
  const autoLayout = useMemo(
    () =>
      isSmartAuto && wrapperSize
        ? pickAutoLayout(orderedIds.length, wrapperSize.width, wrapperSize.height)
        : null,
    [isSmartAuto, wrapperSize, orderedIds.length]
  )

  // For the first paint before the ResizeObserver fires we still want a
  // sensible column layout so cards don't flash stacked-in-one-column.
  // Use n cards wide (capped at 4) — close to the final smart choice for
  // small n and prevents the "single column" flicker.
  const firstPaintCols = Math.max(1, Math.min(4, orderedIds.length || 1))

  const gridStyle: React.CSSProperties = isMobile
    ? { gridTemplateColumns: '1fr', gridAutoRows: 'auto' }
    : gridColumns > 0
      ? {
          gridTemplateColumns: `repeat(${gridColumns}, 1fr)`,
          gridAutoRows: `${rowHeight + 42}px`
        }
      : autoLayout?.mode === 'fit'
        ? {
            gridTemplateColumns: `repeat(${autoLayout.cols}, 1fr)`,
            gridTemplateRows: `repeat(${autoLayout.rows}, 1fr)`
          }
        : autoLayout?.mode === 'scroll'
          ? {
              gridTemplateColumns: `repeat(${autoLayout.cols}, 1fr)`,
              // Scroll-mode row height = viewport / (fit-mode max rows).
              // This makes the first N rows exactly fill the viewport so the
              // "above the fold" looks identical to fit mode — card N+1 is
              // then a clean scroll-down away, not a half-visible slice.
              gridAutoRows: `${
                wrapperSize?.height
                  ? Math.max(
                      AUTO_MIN_CARD_H,
                      Math.floor(wrapperSize.height / fitMaxRows(wrapperSize.height))
                    )
                  : rowHeight + 42
              }px`,
              overflowY: 'auto'
            }
          : {
              gridTemplateColumns: `repeat(${firstPaintCols}, 1fr)`,
              gridTemplateRows: '1fr'
            }

  const DRAG_THRESHOLD = 5

  const handleDragStart = useCallback(
    (terminalId: string, e: React.PointerEvent) => {
      if (sortMode !== 'manual') return
      if (e.button !== 0) return
      const el = cardRefs.current.get(terminalId)
      const rect = el?.getBoundingClientRect()
      setDragState({
        draggingId: terminalId,
        startX: e.clientX,
        startY: e.clientY,
        offsetX: rect ? e.clientX - rect.left : 0,
        offsetY: rect ? e.clientY - rect.top : 0,
        isDragging: false,
        pointerX: e.clientX,
        pointerY: e.clientY,
        width: rect?.width ?? 320
      })
    },
    [sortMode]
  )

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!dragState) return

      const dx = e.clientX - dragState.startX
      const dy = e.clientY - dragState.startY

      if (!dragState.isDragging && Math.abs(dx) + Math.abs(dy) < DRAG_THRESHOLD) return

      if (!dragState.isDragging) {
        setDragState((prev) =>
          prev ? { ...prev, isDragging: true, pointerX: e.clientX, pointerY: e.clientY } : prev
        )
      } else {
        setDragState((prev) =>
          prev ? { ...prev, pointerX: e.clientX, pointerY: e.clientY } : prev
        )
      }

      const targetIndex = getDropIndex(e.clientX, e.clientY, orderedIds, cardRefs.current)
      setDropTargetIndex(targetIndex)
    },
    [dragState, orderedIds]
  )

  const handlePointerUp = useCallback(() => {
    if (dragState?.isDragging && dropTargetIndex !== null) {
      const fromIndex = orderedIds.indexOf(dragState.draggingId)
      if (fromIndex !== -1 && fromIndex !== dropTargetIndex) {
        reorderTerminals(fromIndex, dropTargetIndex)
      }
    }
    setDragState(null)
    setDropTargetIndex(null)
  }, [dragState, dropTargetIndex, orderedIds, reorderTerminals])

  const handlePointerCancel = useCallback(() => {
    setDragState(null)
    setDropTargetIndex(null)
  }, [])

  const createNewSession = useCallback(() => {
    const state = useAppStore.getState()
    const project = resolveActiveProject()
    if (!project) {
      state.setNewAgentDialogOpen(true)
      return
    }
    const agentType = state.config?.defaults.defaultAgent || 'claude'
    window.api
      .createTerminal({ agentType, projectName: project.name, projectPath: project.path })
      .then((session) => state.addTerminal(session))
  }, [])

  const handleGridDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target !== e.currentTarget) return
      createNewSession()
    },
    [createNewSession]
  )

  const handleGridContextMenu = useCallback((e: React.MouseEvent) => {
    if (e.target !== e.currentTarget) return
    e.preventDefault()
    setGridContextMenu({ x: e.clientX, y: e.clientY })
  }, [])

  return (
    <div
      className={`h-full flex flex-col ${isSmartAuto ? 'overflow-hidden' : 'overflow-auto'}`}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerLeave={handlePointerUp}
      onPointerCancel={handlePointerCancel}
      onDoubleClick={handleGridDoubleClick}
      onContextMenu={handleGridContextMenu}
    >
      {orderedIds.length === 0 ? (
        isFiltered ? (
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
        )
      ) : gridColumns === -1 ? (
        <FlexibleGrid
          orderedIds={orderedIds}
          allOrderedIds={allOrderedIds}
          maximizedPaneId={maximizedPaneId}
          onCreateSession={createNewSession}
          onShowContextMenu={setGridContextMenu}
        />
      ) : (
        <div
          ref={attachWrapperRef}
          className={`grid gap-0 ${isSmartAuto ? 'flex-1 min-h-0' : ''}`}
          style={gridStyle}
          onDoubleClick={handleGridDoubleClick}
          onContextMenu={handleGridContextMenu}
        >
          {orderedIds.map((id, index) => (
            <div
              key={id}
              className="min-w-0 min-h-0"
              style={
                id === maximizedPaneId && maximizedSpan > 1
                  ? { gridColumn: `span ${maximizedSpan}` }
                  : undefined
              }
            >
              <PaneRenderer
                ref={(el) => {
                  if (el) cardRefs.current.set(id, el)
                  else cardRefs.current.delete(id)
                }}
                paneId={id}
                index={index}
                isDragTarget={dragState?.isDragging === true && dropTargetIndex === index}
                onDragStart={sortMode === 'manual' ? handleDragStart : undefined}
              />
            </div>
          ))}
        </div>
      )}
      {gridContextMenu && (
        <GridContextMenu position={gridContextMenu} onClose={() => setGridContextMenu(null)} />
      )}
      {dragState?.isDragging && <GridDragGhost dragState={dragState} terminals={terminals} />}
    </div>
  )
})

/* ── Flexible Grid (Grafana-style free positioning) ──────────── */

const FLEX_COLS = 12
const FLEX_ROW_H = 80
const FLEX_DEFAULT_W = 4
const FLEX_DEFAULT_H = 3

// noCompactor gives free positioning; preventCollision blocks drag/resize into a neighbour.
const flexCompactor = { ...noCompactor, preventCollision: true }

function getStableKey(session: TerminalState['session']): string {
  return session.hookSessionId || session.agentSessionId || ''
}

function FlexibleGrid({
  orderedIds,
  allOrderedIds,
  maximizedPaneId,
  onCreateSession,
  onShowContextMenu
}: {
  orderedIds: string[]
  /** Includes panes hidden by maximize — needed to compute the group's footprint. */
  allOrderedIds: string[]
  maximizedPaneId: string | null
  onCreateSession: () => void
  onShowContextMenu: (pos: { x: number; y: number } | null) => void
}) {
  const { size: containerSize, setNode: setContainerNode } = useContainerSize()
  const containerWidth = containerSize?.width ?? 0

  // Narrow selector: only extract the stable keys we need, not the full terminals Map.
  // Child panes key off their owner session's stable key, prefixed by kind, so a
  // session's tree and file keep independent saved rects that survive restarts
  // alongside the terminal's.
  const stableKeys = useAppStore(
    useShallow((s) => {
      const keys: Record<string, string> = {}
      for (const id of allOrderedIds) {
        const { kind, sessionId } = parsePaneId(id)
        const t = s.terminals.get(sessionId)
        if (!t) continue
        const base = getStableKey(t.session) || sessionId
        keys[id] = kind === 'terminal' ? base : `${kind}:${base}`
      }
      return keys
    })
  )
  const flexibleLayouts = useAppStore((s) => s.flexibleLayouts)
  const setFlexibleLayouts = useAppStore((s) => s.setFlexibleLayouts)

  const layout = useMemo(() => {
    // Stack new items below all previously-placed cards so they don't overlap
    let nextY = 0
    for (const id of orderedIds) {
      const key = stableKeys[id]
      if (!key) continue
      const saved = flexibleLayouts[key]
      if (saved) nextY = Math.max(nextY, saved.y + saved.h)
    }

    let autoIndex = 0
    const items = orderedIds.map((id) => {
      const key = stableKeys[id]
      if (!key) return { i: id, x: 0, y: 0, w: FLEX_DEFAULT_W, h: FLEX_DEFAULT_H }
      const saved = flexibleLayouts[key]
      if (saved) {
        return { i: id, x: saved.x, y: saved.y, w: saved.w, h: saved.h }
      }
      const maxPerRow = Math.floor(FLEX_COLS / FLEX_DEFAULT_W)
      const x = (autoIndex % maxPerRow) * FLEX_DEFAULT_W
      const y = nextY + Math.floor(autoIndex / maxPerRow) * FLEX_DEFAULT_H
      autoIndex++
      return { i: id, x, y, w: FLEX_DEFAULT_W, h: FLEX_DEFAULT_H }
    })

    // Maximized pane takes the bounding box of its owner session's rects. This is
    // computed for render only and never written back to `flexibleLayouts`, so
    // restoring returns every sibling to its own saved position.
    if (maximizedPaneId) {
      const owner = parsePaneId(maximizedPaneId).sessionId
      const group = allOrderedIds.filter((id) => parsePaneId(id).sessionId === owner)
      if (group.length > 1) {
        const rects = group
          .map((id) => flexibleLayouts[stableKeys[id]])
          .filter((r): r is FlexibleLayoutRect => Boolean(r))
        if (rects.length > 0) {
          const x = Math.min(...rects.map((r) => r.x))
          const y = Math.min(...rects.map((r) => r.y))
          const right = Math.max(...rects.map((r) => r.x + r.w))
          const bottom = Math.max(...rects.map((r) => r.y + r.h))
          const target = items.find((it) => it.i === maximizedPaneId)
          if (target) {
            target.x = x
            target.y = y
            target.w = right - x
            target.h = bottom - y
          }
        }
      }
    }

    return items
  }, [orderedIds, allOrderedIds, stableKeys, flexibleLayouts, maximizedPaneId])

  const persistLayout = useCallback(
    (updatedLayout: Layout) => {
      const merged: Record<string, FlexibleLayoutRect> = { ...flexibleLayouts }
      for (const item of updatedLayout) {
        // While maximized, the pane's rect is a computed bounding box over its
        // siblings — persisting it would destroy their saved positions.
        if (item.i === maximizedPaneId) continue
        const key = stableKeys[item.i]
        if (!key) continue
        merged[key] = { x: item.x, y: item.y, w: item.w, h: item.h }
      }
      setFlexibleLayouts(merged)
    },
    [stableKeys, flexibleLayouts, setFlexibleLayouts, maximizedPaneId]
  )

  const handleDragStop: EventCallback = useCallback(
    (layout) => persistLayout(layout),
    [persistLayout]
  )

  const handleResizeStop: EventCallback = useCallback(
    (layout) => {
      persistLayout(layout)
      // Notify terminals to refit after RGL finishes resizing their containers
      setTimeout(() => window.dispatchEvent(new Event('resize')), 50)
    },
    [persistLayout]
  )

  const handleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      if ((e.target as HTMLElement).closest('.react-grid-item')) return
      onCreateSession()
    },
    [onCreateSession]
  )

  const handleContextMenu = useCallback(
    (e: React.MouseEvent) => {
      if ((e.target as HTMLElement).closest('.react-grid-item')) return
      e.preventDefault()
      onShowContextMenu({ x: e.clientX, y: e.clientY })
    },
    [onShowContextMenu]
  )

  if (containerWidth === 0) {
    return <div ref={setContainerNode} className="w-full min-h-[200px]" />
  }

  return (
    <div ref={setContainerNode} onDoubleClick={handleDoubleClick} onContextMenu={handleContextMenu}>
      <GridLayout
        className="flexible-grid"
        layout={layout}
        width={containerWidth}
        gridConfig={{
          cols: FLEX_COLS,
          rowHeight: FLEX_ROW_H,
          margin: [0, 0],
          containerPadding: null,
          maxRows: Infinity
        }}
        compactor={flexCompactor}
        dragConfig={{ enabled: true, handle: '.drag-handle', bounded: false, threshold: 3 }}
        resizeConfig={{ enabled: true, handles: ['se'] }}
        onDragStop={handleDragStop}
        onResizeStop={handleResizeStop}
      >
        {orderedIds.map((id, index) => (
          <div key={id} className="h-full">
            <PaneRenderer paneId={id} index={index} flexible />
          </div>
        ))}
      </GridLayout>
    </div>
  )
}

function GridDragGhost({
  dragState,
  terminals
}: {
  dragState: DragState
  terminals: Map<string, TerminalState>
}) {
  const terminal = terminals.get(dragState.draggingId)
  if (!terminal) return null

  const session = terminal.session
  const displayName = getDisplayName(session)

  return createPortal(
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 0.9, scale: 1 }}
      className="fixed rounded-lg border border-white/[0.12] overflow-hidden pointer-events-none"
      style={{
        left: dragState.pointerX - dragState.offsetX,
        top: dragState.pointerY - dragState.offsetY,
        width: dragState.width,
        zIndex: 9999,
        background: '#1a1a1e',
        boxShadow: '0 8px 32px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.06)'
      }}
    >
      <div className="flex items-center gap-2 px-3 py-2.5">
        <AgentIcon agentType={session.agentType} size={14} />
        <div className="flex-1 min-w-0">
          <span className="text-[13px] font-medium text-gray-300 truncate block">
            {displayName}
          </span>
          {session.branch && (
            <div className="flex items-center gap-1 mt-0.5">
              {session.isWorktree ? (
                <FolderGit2 size={10} className="text-amber-500 shrink-0" strokeWidth={1.5} />
              ) : (
                <GitBranch size={10} className="text-gray-600 shrink-0" strokeWidth={1.5} />
              )}
              <span
                className={`text-[10px] font-mono truncate ${session.isWorktree ? 'text-amber-400' : 'text-gray-500'}`}
              >
                {getBranchLabel(session)}
              </span>
            </div>
          )}
        </div>
      </div>
    </motion.div>,
    document.body
  )
}

function getDropIndex(
  pointerX: number,
  pointerY: number,
  orderedIds: string[],
  refs: Map<string, HTMLDivElement>
): number | null {
  let closestIndex: number | null = null
  let closestDist = Infinity

  for (let i = 0; i < orderedIds.length; i++) {
    const el = refs.get(orderedIds[i])
    if (!el) continue
    const rect = el.getBoundingClientRect()
    const cx = rect.left + rect.width / 2
    const cy = rect.top + rect.height / 2
    const dist = Math.hypot(pointerX - cx, pointerY - cy)
    if (dist < closestDist) {
      closestDist = dist
      closestIndex = i
    }
  }

  return closestIndex
}
