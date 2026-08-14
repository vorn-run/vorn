import { forwardRef, memo } from 'react'
import { parsePaneId } from '../lib/pane-id'
import { FilesCard } from './FilesCard'
import { EditorCard } from './EditorCard'
import { BrowserCard } from './BrowserCard'
import { DeviceCard } from './DeviceCard'

/**
 * A promoted pane, drawn as its own cell in the grid.
 *
 * This is a dispatcher and nothing more. The pane renders exactly the component
 * it renders inside a session card — same props, same state, same `PaneCard`
 * chrome — because promotion changes where a pane sits, not what it is. What
 * differs (the identity header, the card frame) is `PaneCard`'s business, and it
 * reads `promotedPanes` for itself rather than being told.
 *
 * Pane ids share an id space with sessions, so ordering, drag, resize and
 * minimize already address a promoted pane without knowing it is one. The pane's
 * record never moves out of the collection that owns it, which is why closing
 * the pane, or its session, still tears down the way it always did.
 */

interface Props {
  paneId: string
  isDragTarget?: boolean
  onDragStart?: (id: string, e: React.PointerEvent) => void
  flexible?: boolean
}

export const PromotedPaneCard = memo(
  forwardRef<HTMLDivElement, Props>(function PromotedPaneCard(
    { paneId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { kind, sessionId } = parsePaneId(paneId)
    const props = { sessionId, isDragTarget, onDragStart, flexible }

    switch (kind) {
      case 'files':
        return <FilesCard ref={ref} {...props} />
      case 'editor':
        return <EditorCard ref={ref} {...props} />
      case 'browser':
        return <BrowserCard ref={ref} {...props} />
      case 'device':
        return <DeviceCard ref={ref} {...props} />
      default:
        return null
    }
  })
)
