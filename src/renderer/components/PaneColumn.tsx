import { useRef, useState, type ReactNode } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { parsePaneId } from '../lib/pane-id'
import { FilesCard } from './FilesCard'
import { EditorCard } from './EditorCard'
import { BrowserCard } from './BrowserCard'
import { SplitDivider } from './SplitDivider'
import { splitPaneWeights, resizePaneWeights } from '../lib/split-ratio'

/**
 * The stack of panes a session owns, rendered inside that session's frame.
 *
 * Panes are no longer grid cells: a session's tree, editor and browser live in
 * a column beside its terminal, so the space a card gets is divided between the
 * things that belong to it rather than spread across unrelated grid cells.
 *
 * While one of the session's panes is maximized it takes the whole column and
 * its siblings hide — the owner check keeps another session's maximized pane
 * from taking over this one.
 */
export function PaneColumn({ sessionId }: { sessionId: string }): ReactNode {
  const { hasFiles, hasEditor, hasBrowser, maximizedPaneId, split, setCardSplit } = useAppStore(
    useShallow((s) => ({
      hasFiles: s.filesPanes.has(sessionId),
      hasEditor: s.editorPanes.has(sessionId),
      hasBrowser: s.browserPanes.has(sessionId),
      maximizedPaneId: s.maximizedPaneId,
      split: s.cardSplits[sessionId],
      setCardSplit: s.setCardSplit
    }))
  )
  const containerRef = useRef<HTMLDivElement | null>(null)
  // The live drag drives local state; the store is written once, on pointerup.
  const [dragWeights, setDragWeights] = useState<number[] | null>(null)

  const kinds = [
    hasFiles ? ('files' as const) : null,
    hasEditor ? ('editor' as const) : null,
    hasBrowser ? ('browser' as const) : null
  ].filter((k): k is 'files' | 'editor' | 'browser' => k !== null)

  if (kinds.length === 0) return null

  const maximized = maximizedPaneId ? parsePaneId(maximizedPaneId) : null
  const maximizedKind =
    maximized && maximized.sessionId === sessionId && maximized.kind !== 'terminal'
      ? maximized.kind
      : null
  const hasMaximized = maximizedKind !== null && kinds.includes(maximizedKind)

  const weights = dragWeights ?? splitPaneWeights(split?.panes, kinds.length)

  const commit = (next: number[]): void => {
    setCardSplit(sessionId, { terminal: split?.terminal ?? 0.5, panes: next })
    setDragWeights(null)
  }

  const render = (kind: 'files' | 'editor' | 'browser'): ReactNode => {
    if (kind === 'files') return <FilesCard sessionId={sessionId} />
    if (kind === 'editor') return <EditorCard sessionId={sessionId} />
    return <BrowserCard sessionId={sessionId} />
  }

  return (
    // No grow factor of its own: the frame around it (card body or tab rail)
    // owns the sizing, and a `flex-1` here would fight the terminal's stored
    // ratio — two grow factors that no longer sum to 1.
    <div ref={containerRef} className="flex flex-col min-h-0 min-w-0 w-full gap-px">
      {kinds.map((kind, i) => {
        if (hasMaximized && kind !== maximizedKind) return null
        return (
          <div key={kind} className="contents">
            {i > 0 && !hasMaximized && (
              <SplitDivider
                axis="y"
                containerRef={containerRef}
                onRatioChange={(r) => setDragWeights(resizePaneWeights(weights, i - 1, r))}
                onRatioCommit={(r) => commit(resizePaneWeights(weights, i - 1, r))}
                label={`Resize ${kind} panel`}
                testId={`pane-divider-${sessionId}-${kind}`}
              />
            )}
            <div
              className="min-h-0 min-w-0"
              style={{ flexGrow: hasMaximized ? 1 : weights[i], flexShrink: 1, flexBasis: 0 }}
            >
              {render(kind)}
            </div>
          </div>
        )
      })}
    </div>
  )
}
