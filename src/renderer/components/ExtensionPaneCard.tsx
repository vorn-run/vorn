import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { PaneCard, PaneControls } from './PaneCard'
import { ExtensionPaneIcon } from './ExtensionPaneIcon'
import { TerminalPane } from './TerminalPane'
import { extensionPaneId } from '../lib/pane-id'
import { PANE_SURFACE } from '../lib/pane-surface'

interface Props {
  /** Session that owns this pane. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * The pane an extension opened beside a session's terminal.
 *
 * Two shapes behind one frame, because to the person they are the same thing —
 * something an extension put on the card, closed the same way:
 *
 * - A page the pack ships, framed from the host's page origin. That origin is
 *   not the app's, so the page reaches nothing this window holds; it is allowed
 *   its own scripts and its own bridge and nothing else. The grant proving it is
 *   in the path, which is why the frame is built from the URL the host returned
 *   rather than assembled here.
 * - A program the host is already running, drawn as the terminal it is. Its pty
 *   is deliberately not a session — no window is told it exists — so it is drawn
 *   from its id alone, with no store entry to add or prune.
 */
export const ExtensionPaneCard = memo(
  forwardRef<HTMLDivElement, Props>(function ExtensionPaneCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { pane, close, domBlocks } = useAppStore(
      useShallow((s) => ({
        pane: s.extensionPanes.get(sessionId) ?? null,
        close: s.closeExtensionPane,
        domBlocks: s.config?.defaults.domBlockRendering ?? true
      }))
    )

    if (!pane) return null

    const paneId = extensionPaneId(sessionId)
    const { open, title, extensionName, icon } = pane

    return (
      <PaneCard
        ref={ref}
        paneId={paneId}
        title={title}
        onClose={() => close(sessionId)}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // Its own bar, so the extension's name can sit beside the pane's title:
        // two panes called "Report" from different packs are otherwise the same
        // pane as far as anything on screen says.
        headerless
      >
        <div
          className={`flex items-center gap-1.5 px-2 py-1 shrink-0 ${
            onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''
          }`}
          onPointerDown={onDragStart ? (e) => onDragStart(paneId, e) : undefined}
          data-testid={`extension-pane-header-${sessionId}`}
        >
          <ExtensionPaneIcon icon={icon} extensionId={open.extensionId} size={12} />
          <span className="text-[12px] text-gray-300 font-medium shrink-0">{title}</span>
          <span className="text-ink-ghost shrink-0">·</span>
          <span className="text-[11px] text-ink-secondary truncate">{extensionName}</span>
          <span className="flex-1" />
          <PaneControls paneId={paneId} title={title} onClose={() => close(sessionId)} />
        </div>

        {open.url ? (
          <div className="flex-1 min-h-0 relative" style={{ background: PANE_SURFACE }}>
            {/* Sandboxed, and same-origin only with itself: the page server is a
                separate origin, so this grants the page its own storage and
                nothing of the app's. Top-level navigation and popups are not
                granted — a pane that could replace the window is not a pane. */}
            <iframe
              title={`${extensionName}: ${title}`}
              src={open.url}
              sandbox="allow-scripts allow-same-origin allow-forms"
              referrerPolicy="no-referrer"
              data-testid={`extension-pane-frame-${sessionId}`}
              className="absolute inset-0 w-full h-full border-0"
            />
          </div>
        ) : (
          // Framed as a terminal, not as a pane: the sunken surface and the same
          // half-step of padding a shell gets in the terminals panel. No
          // composer — this program is the extension's, and typing at it is
          // typing at the program, not prompting an agent.
          <div
            className="flex-1 min-h-0 relative pt-0.5"
            style={{ background: 'var(--color-surface-sunken)' }}
          >
            {open.terminalId && (
              <TerminalPane
                key={open.terminalId}
                terminalId={open.terminalId}
                agentType="shell"
                isFocused={false}
                flexible={flexible}
                domBlocks={domBlocks}
              />
            )}
          </div>
        )}
      </PaneCard>
    )
  })
)
