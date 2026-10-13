import { memo, forwardRef, useMemo, useState } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { SquareArrowOutUpRight } from 'lucide-react'
import { useAppStore } from '../stores'
import { activePanelIndex } from '../stores/types'
import { PaneCard, PaneControls } from './PaneCard'
import { PaneTabStrip } from './PaneTabStrip'
import { TerminalPane } from './TerminalPane'
import { SessionComposer } from './card/SessionComposer'
import { terminalsPaneId } from '../lib/pane-id'
import { getDisplayName } from '../lib/terminal-display'
import { terminalTextIndentPx } from '../lib/terminal-indent'
import { addTerminalToPanel } from '../lib/session-utils'
import { closeTerminalSession, closeTerminalsPanel } from '../lib/terminal-close'

interface Props {
  /** Session that owns this panel. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * The shells a session holds beside its agent.
 *
 * Tabbed rather than stacked: a shell needs width to be readable and this
 * column is narrow, so three at once makes three unusable. Only the tab in
 * front renders a `TerminalPane` — the others keep running and keep their
 * scrollback, because the terminal registry owns the xterm and a slot is only
 * where it is currently drawn.
 *
 * Every shell in here is a full session with its own pid. That is what makes
 * the ↗ on each tab a one-line action: it removes the claim, and the terminal
 * is immediately a grid cell, a tab, a sidebar row and a focus target, with no
 * card machinery involved at all.
 */
export const TerminalsCard = memo(
  forwardRef<HTMLDivElement, Props>(function TerminalsCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    // Everything selected here is either a primitive or a reference the store
    // replaces only when it genuinely changes. Building the tab list inside the
    // selector instead would hand `useShallow` a fresh array every call, so no
    // two snapshots would ever compare equal and the component would re-render
    // without end. Derive it in a memo below, not in the selector.
    const { owner, pane, terminals, setActive, extract, domBlocks } = useAppStore(
      useShallow((s) => ({
        owner: s.terminals.get(sessionId),
        pane: s.terminalsPanes.get(sessionId) ?? null,
        terminals: s.terminals,
        setActive: s.setActivePanelTerminal,
        extract: s.extractPanelTerminal,
        domBlocks: s.config?.defaults.domBlockRendering ?? true
      }))
    )

    // Which shell should hold the keyboard. It cannot be `focusedTerminalId`:
    // a claimed shell is deliberately kept out of the focusable set, so that
    // comparison is structurally always false and nothing ever put the caret in
    // here — clicking a tab left you typing at the agent instead.
    const [focusTarget, setFocusTarget] = useState<string | null>(null)

    const held = pane?.terminals
    const names = useMemo(
      () =>
        (held ?? []).map((id) => {
          const t = terminals.get(id)
          return { id, name: t ? getDisplayName(t.session) : id, agentType: t?.session.agentType }
        }),
      [held, terminals]
    )

    if (!owner || !pane || names.length === 0) return null

    const activeIndex = activePanelIndex(pane)
    const active = names[activeIndex]
    const paneId = terminalsPaneId(sessionId)
    const close = (): void => void closeTerminalsPanel(sessionId)

    return (
      <PaneCard
        ref={ref}
        paneId={paneId}
        title="Terminals"
        onClose={close}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // The tab strip is this pane's title bar, as the browser's is.
        headerless
      >
        <PaneTabStrip
          ariaLabel="Terminals"
          testId={`terminals-pane-header-${sessionId}`}
          draggable={Boolean(onDragStart || flexible)}
          onPointerDown={onDragStart ? (e) => onDragStart(paneId, e) : undefined}
          // A press on a tab must not reach the card, which would hand the keyboard back to the agent.
          isolateTabPointer
          tabs={names.map((t) => ({ id: t.id, name: t.name }))}
          // The clamped index, the same one the body below is drawn from.
          activeId={active.id}
          onSelect={(id) => {
            setActive(
              sessionId,
              names.findIndex((t) => t.id === id)
            )
            setFocusTarget(id)
          }}
          onClose={(id) => void closeTerminalSession(id)}
          tabActions={(tab) => (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                extract(sessionId, tab.id)
              }}
              aria-label={`Open ${tab.name} as its own terminal`}
              title="Open as its own terminal"
              className="shrink-0 p-0.5 rounded text-gray-600 hover:text-white
                         hover:bg-white/[0.08] transition-colors"
            >
              <SquareArrowOutUpRight size={10} strokeWidth={2.5} />
            </button>
          )}
          onAdd={() => {
            void addTerminalToPanel(sessionId).then((id) => {
              if (id) setFocusTarget(id)
            })
          }}
          addLabel="New terminal in this session"
          trailing={
            <PaneControls paneId={paneId} title="Terminals" onClose={close} className="shrink-0" />
          }
        />

        {/* Only the tab in front is drawn. The rest keep running — the registry
            holds the xterm and its scrollback, and a slot is only where a
            terminal is currently shown. Rendering them all would also have two
            slots claiming one terminal, which the registry resolves by
            last-writer-wins: one of them would simply go blank.

            Framed as a terminal, not as a pane: the sunken surface, the same
            half-step of top padding, and the composer docked beneath. A shell
            in here is the same kind of thing as a shell in the grid, and the
            pane grey is for frames around someone else's content — which is
            what the tab strip above is, and what the terminal below is not. */}
        <div
          className="flex-1 min-h-0 relative pt-0.5"
          style={{ background: 'var(--color-surface-sunken)' }}
        >
          <TerminalPane
            key={active.id}
            terminalId={active.id}
            agentType={active.agentType}
            isFocused={focusTarget === active.id}
            flexible={flexible}
            domBlocks={domBlocks}
          />
        </div>

        <SessionComposer
          terminalId={active.id}
          compact
          indentPx={terminalTextIndentPx(active.agentType, domBlocks)}
        />
      </PaneCard>
    )
  })
)
