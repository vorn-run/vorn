import { useRef, useState, type ReactNode } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { paneIdFor, type PaneChildKind } from '../lib/pane-id'
import { usePromotedCardsFor } from '../hooks/usePromotedCards'
import { useCardsDrawnAsCells } from '../hooks/useCardsDrawnAsCells'
import { FilesCard } from './FilesCard'
import { PromotedPaneCard } from './PromotedPaneCard'
import { EditorCard } from './EditorCard'
import { BrowserCard } from './BrowserCard'
import { DeviceCard } from './DeviceCard'
import { SplitDivider } from './SplitDivider'
import { splitPaneWeights, resizePaneWeights } from '../lib/split-ratio'

/**
 * The stack of panes a session owns, rendered inside that session's frame.
 *
 * Panes are not grid cells: a session's tree, editor and browser live in a
 * column beside its terminal, so the space a card gets is divided between the
 * things that belong to it rather than spread across unrelated grid cells.
 *
 * The column also takes in the session's popped-out cards wherever the layout
 * gives them no cell of their own — focused mode, hover preview, mobile. Those
 * show a single session, so without this a popped-out file simply vanished when
 * you left the grid, taking the control that brings it back with it.
 *
 * While one of the panes is maximized it takes the whole column and its siblings
 * hide — the owner check keeps another session's maximized pane from taking over
 * this one.
 */

/** One row of the column: a pane the session owns, or a card it popped out. */
interface ColumnEntry {
  /** Pane id, which is also the React key and what maximize is matched on. */
  id: string
  kind: PaneChildKind
  /** Set when this is a popped-out card, and is then the key into its map. */
  cardKey?: string
}

export function PaneColumn({ sessionId }: { sessionId: string }): ReactNode {
  const { hasFiles, hasEditor, hasBrowser, hasDevice, maximizedPaneId, split, setCardSplit } =
    useAppStore(
      useShallow((s) => ({
        hasFiles: s.filesPanes.has(sessionId),
        hasEditor: s.editorPanes.has(sessionId),
        hasBrowser: s.browserPanes.has(sessionId),
        hasDevice: s.devicePanes.has(sessionId),
        maximizedPaneId: s.maximizedPaneId,
        split: s.cardSplits[sessionId],
        setCardSplit: s.setCardSplit
      }))
    )
  const cardsHaveCells = useCardsDrawnAsCells()
  const cards = usePromotedCardsFor(sessionId)
  const containerRef = useRef<HTMLDivElement | null>(null)
  // The live drag drives local state; the store is written once, on pointerup.
  const [dragWeights, setDragWeights] = useState<number[] | null>(null)

  const ownKinds = [
    hasFiles ? ('files' as const) : null,
    hasEditor ? ('editor' as const) : null,
    hasBrowser ? ('browser' as const) : null,
    hasDevice ? ('device' as const) : null
  ].filter((k): k is PaneChildKind => k !== null)

  const entries: ColumnEntry[] = [
    ...ownKinds.map((kind) => ({ id: paneIdFor(kind, sessionId), kind })),
    // Cards last, after everything the session already had — they arrived last,
    // and inserting them above would shuffle the panes someone had arranged.
    ...(cardsHaveCells
      ? []
      : cards.map((card) => ({ id: card.id, kind: card.kind, cardKey: card.id })))
  ]

  if (entries.length === 0) return null

  // Matched on pane id rather than kind: a session can now hold several editors
  // in one column — its own, plus a card per popped-out file — and only the id
  // tells them apart.
  const maximizedIndex = entries.findIndex((e) => e.id === maximizedPaneId)
  const hasMaximized = maximizedIndex !== -1

  const weights = dragWeights ?? splitPaneWeights(split?.panes, entries.length)

  const commit = (next: number[]): void => {
    setCardSplit(sessionId, { terminal: split?.terminal ?? 0.5, panes: next })
    setDragWeights(null)
  }

  const render = (entry: ColumnEntry): ReactNode => {
    // One dispatcher for cards, shared with the grid and the tab strip. Three
    // places drawing a card is three chances for them to disagree about what a
    // card even is.
    if (entry.cardKey) return <PromotedPaneCard cardId={entry.cardKey} />
    if (entry.kind === 'files') return <FilesCard sessionId={sessionId} />
    if (entry.kind === 'editor') return <EditorCard sessionId={sessionId} />
    if (entry.kind === 'browser') return <BrowserCard sessionId={sessionId} />
    return <DeviceCard sessionId={sessionId} />
  }

  return (
    // No grow factor of its own: the frame around it (card body or tab rail)
    // owns the sizing, and a `flex-1` here would fight the terminal's stored
    // ratio — two grow factors that no longer sum to 1. No padding either: the
    // panes take the card's full height, and the step down in surface separates
    // them, where insetting framed the panel and cost height.
    <div ref={containerRef} className="relative flex flex-col min-h-0 min-w-0 w-full gap-px">
      {entries.map((entry, i) => {
        const hidden = hasMaximized && i !== maximizedIndex
        if (hidden) {
          // Hidden, not unmounted. Unmounting destroys the browser pane's
          // <webview> guest, which loses the page, its scroll position and any
          // half-filled form the moment a sibling is maximized — and detaches
          // the session agent's CDP handle, leaving it told "no pane open"
          // while the store still says one exists. This is the same rule the
          // browser's own tabs already follow one level down.
          //
          // Taken out of flow rather than `display: none`, so it keeps a real
          // size: a webview collapsed to zero does not reliably come back.
          return (
            <div
              key={entry.id}
              aria-hidden
              data-testid={`pane-hidden-${sessionId}-${entry.kind}`}
              className="absolute inset-0 pointer-events-none invisible"
            >
              {render(entry)}
            </div>
          )
        }
        return (
          <div key={entry.id} className="contents">
            {i > 0 && !hasMaximized && (
              <SplitDivider
                axis="y"
                containerRef={containerRef}
                onRatioChange={(r) => setDragWeights(resizePaneWeights(weights, i - 1, r))}
                onRatioCommit={(r) => commit(resizePaneWeights(weights, i - 1, r))}
                label={`Resize ${entry.kind} panel`}
                testId={`pane-divider-${sessionId}-${entry.kind}`}
              />
            )}
            <div
              className="min-h-0 min-w-0"
              style={{ flexGrow: hasMaximized ? 1 : weights[i], flexShrink: 1, flexBasis: 0 }}
            >
              {render(entry)}
            </div>
          </div>
        )
      })}
    </div>
  )
}
