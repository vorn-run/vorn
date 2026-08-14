import { useRef, useState, type ReactNode } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { paneIdFor, type PaneChildKind } from '../lib/pane-id'
import { FilesCard } from './FilesCard'
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
 * Popped-out cards are deliberately not here. A card is a thing in its own
 * right — its own cell, its own tab, its own focus stage — and drawing it back
 * inside its owner was what made it read as merely displaced from the session
 * rather than independent of it.
 *
 * While one of the panes is maximized it takes the whole column and its siblings
 * hide — the owner check keeps another session's maximized pane from taking over
 * this one.
 */

/** One row of the column, keyed by the pane id maximize is matched on. */
interface ColumnEntry {
  id: string
  kind: PaneChildKind
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
  const containerRef = useRef<HTMLDivElement | null>(null)
  // The live drag drives local state; the store is written once, on pointerup.
  const [dragWeights, setDragWeights] = useState<number[] | null>(null)

  const ownKinds = [
    hasFiles ? ('files' as const) : null,
    hasEditor ? ('editor' as const) : null,
    hasBrowser ? ('browser' as const) : null,
    hasDevice ? ('device' as const) : null
  ].filter((k): k is PaneChildKind => k !== null)

  const entries: ColumnEntry[] = ownKinds.map((kind) => ({
    id: paneIdFor(kind, sessionId),
    kind
  }))

  if (entries.length === 0) return null

  // Matched on pane id rather than kind, so a stale or foreign id is simply not
  // found rather than matching by kind and blanking the wrong session's column.
  const maximizedIndex = entries.findIndex((e) => e.id === maximizedPaneId)
  const hasMaximized = maximizedIndex !== -1

  const weights = dragWeights ?? splitPaneWeights(split?.panes, entries.length)

  const commit = (next: number[]): void => {
    setCardSplit(sessionId, { terminal: split?.terminal ?? 0.5, panes: next })
    setDragWeights(null)
  }

  const render = (entry: ColumnEntry): ReactNode => {
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
