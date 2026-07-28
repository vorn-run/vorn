import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { TerminalPane, terminalTextIndentPx } from './TerminalPane'
import { CardHeader } from './card/CardHeader'
import { CardStatusBar } from './card/CardStatusBar'
import { IntentBar } from './IntentBar'
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
    const { showScrollBtn, handleScrollToBottom } = useTerminalScrollButton(terminalId)

    if (!terminal) return null

    const isFocused = focusedId === terminalId
    const isSelected = selectedId === terminalId
    // Selection is signalled by dimming the chrome (header + status bar) on
    // every *other* card so the active one stands out by relative brightness,
    // without painting a border that fights with the terminal contents.
    const isChromeDimmed = selectedId !== null && !isSelected && !isFocused

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

        {/* Terminal */}
        <div className="relative flex-1 min-h-0 pt-0.5" style={{ background: '#141416' }}>
          {!isFocused && (
            <TerminalPane
              terminalId={terminalId}
              agentType={terminal.session.agentType}
              isFocused={isSelected}
              flexible={flexible}
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

        {!isFocused && (
          <IntentBar
            terminalId={terminalId}
            compact
            indentPx={terminalTextIndentPx(terminal.session.agentType)}
          />
        )}

        <CardStatusBar terminalId={terminalId} dimmed={isChromeDimmed} />

        {!flexible && <RowResizeHandle />}
      </div>
    )
  })
)
