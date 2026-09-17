import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { PaneCard, PaneOwnerLabel, PromotedCardControls } from './PaneCard'
import { FileEditorPane } from './FileTreeExplorer'
import { dirtyRefFor } from '../lib/editor-dirty'

interface Props {
  /** Session the file was popped out of. */
  sessionId: string
  /** The card's id, and its key in `editorPanes`. */
  paneKey: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/** A file popped out of its session's Files pane, as a card of its own. */
export const EditorCard = memo(
  forwardRef<HTMLDivElement, Props>(function EditorCard(
    { sessionId, paneKey, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, filePath, closeCard } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        filePath: s.editorPanes.get(paneKey)?.filePath ?? null,
        closeCard: s.closeCard
      }))
    )

    if (!terminal || !filePath) return null

    const cwd = terminal.session.worktreePath || terminal.session.projectPath
    const fileName = filePath.split(/[/\\]/).pop() ?? filePath
    // Closing discards the buffer, so the store asks first.
    const handleClose = (): void => closeCard(paneKey)

    return (
      <PaneCard
        ref={ref}
        paneId={paneKey}
        title={fileName}
        onClose={handleClose}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // The path strip already names the file, so the controls join it.
        headerless
      >
        <FileEditorPane
          key={filePath}
          cwd={cwd}
          filePath={filePath}
          remoteHostId={terminal.session.remoteHostId}
          dirtyRef={dirtyRefFor(paneKey)}
          draftKey={paneKey}
          controls={
            <>
              <PaneOwnerLabel sessionId={sessionId} />
              <PromotedCardControls
                cardId={paneKey}
                title={fileName}
                onClose={handleClose}
                className="shrink-0"
              />
            </>
          }
          headerClassName={
            onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''
          }
          onHeaderPointerDown={onDragStart ? (e) => onDragStart(paneKey, e) : undefined}
          headerTestId="editor-pane-header"
        />
      </PaneCard>
    )
  })
)
