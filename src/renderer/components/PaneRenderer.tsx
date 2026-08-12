import { forwardRef } from 'react'
import { AgentCard } from './AgentCard'
import { FilesCard } from './FilesCard'
import { EditorCard } from './EditorCard'
import { parsePaneId } from '../lib/pane-id'

interface Props {
  /** Pane id: a terminal id, or `files:<sessionId>` / `editor:<sessionId>`. */
  paneId: string
  index?: number
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * Renders whichever card a pane id names.
 *
 * Every grid render path goes through here so the kind branch lives in exactly
 * one place; the paths themselves stay a flat map over opaque id strings.
 */
export const PaneRenderer = forwardRef<HTMLDivElement, Props>(function PaneRenderer(
  { paneId, index, isDragTarget, onDragStart, flexible },
  ref
) {
  const { kind, sessionId } = parsePaneId(paneId)

  if (kind === 'files') {
    return (
      <FilesCard
        ref={ref}
        sessionId={sessionId}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
      />
    )
  }

  if (kind === 'editor') {
    return (
      <EditorCard
        ref={ref}
        sessionId={sessionId}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
      />
    )
  }

  return (
    <AgentCard
      ref={ref}
      terminalId={paneId}
      index={index}
      isDragTarget={isDragTarget}
      onDragStart={onDragStart}
      flexible={flexible}
    />
  )
})
