import { forwardRef, memo } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { EditorCard } from './EditorCard'
import { BrowserCard } from './BrowserCard'

/**
 * A file or browser tab popped out of a session's card, drawn as its own cell.
 *
 * A dispatcher and nothing more. The card renders the same component the pane
 * renders inside a session — same props but a different key into the same
 * collection — because popping out changes where a thing is drawn, not what it
 * is. What differs (the card frame, the owner label, minimize / return) is
 * `PaneCard`'s business, and it works that out from the key too.
 *
 * Which collection holds the key is what says whether this is a file or a page.
 * There is no third list to keep in step: closing the pane closes the card,
 * because the pane entry *is* the card.
 */

interface Props {
  cardId: string
  isDragTarget?: boolean
  onDragStart?: (id: string, e: React.PointerEvent) => void
  flexible?: boolean
}

export const PromotedPaneCard = memo(
  forwardRef<HTMLDivElement, Props>(function PromotedPaneCard(
    { cardId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { editor, browser } = useAppStore(
      useShallow((s) => ({
        editor: s.editorPanes.get(cardId),
        browser: s.browserPanes.get(cardId)
      }))
    )
    const rest = { paneKey: cardId, isDragTarget, onDragStart, flexible }

    // No `isPromotedPane` check: a `card:`-prefixed key can only ever name a
    // card, since a session's own pane is keyed by its session id.
    if (editor) return <EditorCard ref={ref} sessionId={editor.sessionId} {...rest} />
    if (browser) return <BrowserCard ref={ref} sessionId={browser.sessionId} {...rest} />
    // The pane went away without the grid having re-derived its cells yet.
    return null
  })
)
