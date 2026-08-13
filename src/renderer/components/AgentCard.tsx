import { memo, forwardRef, useRef, useState } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { selectPaneFlags } from '../stores/ui-slice'
import { TerminalPane } from './TerminalPane'
import { terminalTextIndentPx } from '../lib/terminal-indent'
import { CardHeader } from './card/CardHeader'
import { CardStatusBar } from './card/CardStatusBar'
import { IntentBar } from './IntentBar'
import { PaneColumn } from './PaneColumn'
import { SplitDivider } from './SplitDivider'
import { parsePaneId } from '../lib/pane-id'
import { DEFAULT_SPLIT_RATIO } from '../lib/split-ratio'
import { useTerminalScrollButton } from '../hooks/useTerminalScrollButton'

// On touch devices, always show action buttons (no hover available)
const isTouchDevice = typeof window !== 'undefined' && window.matchMedia('(hover: none)').matches

interface Props {
  terminalId: string
  index?: number
  isDragTarget?: boolean
  onDragStart?: (terminalId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

function RowResizeHandle() {
  const handlePointerDown = (e: React.PointerEvent): void => {
    e.preventDefault()
    e.stopPropagation()
    const startY = e.clientY
    const startHeight = useAppStore.getState().rowHeight

    const onMove = (ev: PointerEvent): void => {
      const delta = ev.clientY - startY
      const newHeight = Math.max(100, Math.min(600, startHeight + delta))
      useAppStore.getState().setRowHeight(newHeight)
    }

    const onUp = (): void => {
      document.removeEventListener('pointermove', onMove)
      document.removeEventListener('pointerup', onUp)
    }

    document.addEventListener('pointermove', onMove)
    document.addEventListener('pointerup', onUp)
  }

  return (
    <div
      className="h-1.5 cursor-row-resize bg-transparent hover:bg-white/[0.06] transition-colors shrink-0"
      onPointerDown={handlePointerDown}
    />
  )
}

export const AgentCard = memo(
  forwardRef<HTMLDivElement, Props>(function AgentCard(
    { terminalId, index, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, focusedId, selectedId, setSelected, setFocused } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(terminalId),
        focusedId: s.focusedTerminalId,
        selectedId: s.selectedTerminalId,
        setSelected: s.setSelectedTerminal,
        setFocused: s.setFocusedTerminal
      }))
    )
    const { hasPanes, maximizedPaneId, storedRatio, setCardSplit, storedPanes } = useAppStore(
      useShallow((s) => ({
        hasPanes: selectPaneFlags(s, terminalId).any,
        maximizedPaneId: s.maximizedPaneId,
        storedRatio: s.cardSplits[terminalId]?.terminal,
        storedPanes: s.cardSplits[terminalId]?.panes,
        setCardSplit: s.setCardSplit
      }))
    )
    const bodyRef = useRef<HTMLDivElement | null>(null)
    // The live drag drives local state; the store is written once, on pointerup,
    // so one resize is one localStorage write rather than dozens.
    const [dragRatio, setDragRatio] = useState<number | null>(null)
    const { showScrollBtn, handleScrollToBottom } = useTerminalScrollButton(terminalId)
    const domBlocks = useAppStore((s) => s.config?.defaults.domBlockRendering ?? true)

    if (!terminal) return null

    const isFocused = focusedId === terminalId
    const isSelected = selectedId === terminalId
    // Selection is signalled by dimming the chrome (header + status bar) on
    // every *other* card so the active one stands out by relative brightness,
    // without painting a border that fights with the terminal contents.
    const isChromeDimmed = selectedId !== null && !isSelected && !isFocused

    // Which of this session's panes, if any, is maximized. A pane belonging to
    // another session must not blank this card, hence the owner check.
    const maximized = maximizedPaneId ? parsePaneId(maximizedPaneId) : null
    const hasMaximizedPane =
      hasPanes &&
      maximized !== null &&
      maximized.sessionId === terminalId &&
      maximized.kind !== 'terminal'
    // With no pane column beside it the terminal is a lone flex child, and a
    // grow factor under 1 would leave the rest of the row as dead space — the
    // ratio only means anything when there is a sibling to share with.
    const terminalRatio = hasPanes ? (dragRatio ?? storedRatio ?? DEFAULT_SPLIT_RATIO) : 1

    const handleExpand = (): void => {
      setFocused(terminalId)
    }

    return (
      <div
        ref={ref}
        className={`group/card relative border overflow-hidden flex flex-col h-full
                   transition-colors
                   ${
                     isFocused
                       ? 'border-blue-500/60 ring-1 ring-blue-500/30'
                       : isDragTarget
                         ? 'card-drop-target border-blue-500/30 hover:border-white/[0.12]'
                         : 'border-white/[0.06] hover:border-white/[0.12]'
                   }
                   ${
                     flexible
                       ? ''
                       : isFocused || isSelected || isDragTarget
                         ? 'z-10'
                         : 'hover:z-10 focus-within:z-10'
                   }`}
        style={{ background: '#1a1a1e' }}
        onPointerDown={() => {
          if (!isSelected && !isFocused) setSelected(terminalId)
        }}
      >
        <CardHeader
          terminalId={terminalId}
          variant="mini"
          index={index}
          draggable={Boolean(onDragStart || flexible)}
          onDragStart={onDragStart}
          onDoubleClick={handleExpand}
          revealActions={isTouchDevice}
          dimmed={isChromeDimmed}
        />

        {/* Terminal, plus this session's panes stacked beside it. `relative`
            stays on the terminal itself: it hosts absolutely-positioned
            overlays that would otherwise stretch across the panes. */}
        <div ref={bodyRef} className="flex flex-row flex-1 min-h-0">
          <div
            data-testid={`card-terminal-${terminalId}`}
            className={`relative min-h-0 min-w-0 pt-0.5 ${hasMaximizedPane ? 'hidden' : ''}`}
            style={{
              flexGrow: terminalRatio,
              flexShrink: 1,
              flexBasis: 0,
              background: '#141416'
            }}
          >
            {!isFocused && (
              <TerminalPane
                terminalId={terminalId}
                agentType={terminal.session.agentType}
                isFocused={isSelected}
                flexible={flexible}
                domBlocks={domBlocks}
              />
            )}
            {isFocused && (
              <div className="flex items-center justify-center h-full text-gray-600 text-xs">
                Expanded
              </div>
            )}
            {!isFocused && terminal.lastOutputTimestamp === 0 && (
              <div
                className="absolute inset-0 p-3 space-y-2 pointer-events-none"
                style={{ background: '#141416' }}
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
            {!isFocused && showScrollBtn && (
              <button
                className="absolute bottom-2 right-2 w-8 h-8 flex items-center justify-center
                           rounded bg-white/[0.08] hover:bg-white/[0.15] active:bg-white/[0.2]
                           text-gray-400 hover:text-white transition-colors z-50"
                onClick={handleScrollToBottom}
                title="Scroll to bottom"
              >
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                  <path
                    d="M6 2.5V9.5M3 7L6 10L9 7"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
            )}
          </div>

          {hasPanes && !hasMaximizedPane && (
            <SplitDivider
              axis="x"
              containerRef={bodyRef}
              onRatioChange={setDragRatio}
              onRatioCommit={(r) => {
                setCardSplit(terminalId, { terminal: r, panes: storedPanes ?? [] })
                setDragRatio(null)
              }}
              label="Resize terminal and panels"
              testId={`card-divider-${terminalId}`}
            />
          )}
          {hasPanes && (
            // The two grow factors must sum to 1, or the stored ratio is not
            // the fraction of the card it claims to be and the divider lags
            // behind the cursor for the whole drag.
            <div
              className="flex min-h-0 min-w-0"
              style={{
                flexGrow: hasMaximizedPane ? 1 : 1 - terminalRatio,
                flexShrink: 1,
                flexBasis: 0
              }}
            >
              <PaneColumn sessionId={terminalId} />
            </div>
          )}
        </div>

        {!isFocused && (
          <IntentBar
            terminalId={terminalId}
            compact
            indentPx={terminalTextIndentPx(terminal.session.agentType, domBlocks)}
          />
        )}

        <CardStatusBar terminalId={terminalId} dimmed={isChromeDimmed} />

        {!flexible && <RowResizeHandle />}
      </div>
    )
  })
)
