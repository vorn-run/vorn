import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { PaneCard, PaneControls, PaneOwnerLabel, PromotedCardControls } from './PaneCard'
import { FileEditorPane } from './FileTreeExplorer'
import { editorPaneId } from '../lib/pane-id'
import { dirtyRefFor, confirmDiscard, clearDirty } from '../lib/editor-dirty'

interface Props {
  /** Session that owns this editor. */
  sessionId: string
  /**
   * Which entry in `editorPanes` to draw. Defaults to the session's own editor.
   *
   * A file popped out to a card of its own is another entry in the same map,
   * under a `card:` key — same component, same behaviour, different key. The
   * session's editor still holds exactly one file; the cards are how you get
   * two open at once.
   */
  paneKey?: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * An open file, as its own grid pane.
 *
 * Independent of its session's tree pane — it renders whatever path the store
 * holds, whether or not a tree is open, and can be maximized on its own.
 */
export const EditorCard = memo(
  forwardRef<HTMLDivElement, Props>(function EditorCard(
    { sessionId, paneKey, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const key = paneKey ?? sessionId
    const { terminal, filePath, promoteFile, closeEditorPane } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        filePath: s.editorPanes.get(key)?.filePath ?? null,
        promoteFile: s.promoteFile,
        closeEditorPane: s.closeEditorPane
      }))
    )

    if (!terminal || !filePath) return null

    const isCard = key !== sessionId
    const cwd = terminal.session.worktreePath || terminal.session.projectPath
    const remoteHostId = terminal.session.remoteHostId
    const fileName = filePath.split(/[/\\]/).pop() ?? filePath

    // Closing discards the buffer, so confirm first. The editor's own state is
    // out of reach from here — it reports dirtiness through the shared ref,
    // keyed by pane so two open files cannot answer for each other.
    const handleClose = (): void => {
      if (!confirmDiscard(key)) return
      clearDirty(key)
      closeEditorPane(key)
    }

    // Out of the session's editor and into a card of its own. The buffer does
    // not travel — the card mounts a fresh editor under its own id — so an
    // unsaved edit is discarded here exactly as it would be by closing.
    const handlePopOut = (): void => {
      if (!confirmDiscard(sessionId)) return
      clearDirty(sessionId)
      promoteFile(sessionId, filePath)
      closeEditorPane(sessionId)
    }

    const paneId = isCard ? key : editorPaneId(sessionId)
    const toggleMaximize = (): void => {
      const state = useAppStore.getState()
      state.setMaximizedPane(state.maximizedPaneId === paneId ? null : paneId)
    }

    return (
      <PaneCard
        ref={ref}
        paneId={paneId}
        title={fileName}
        onClose={handleClose}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // The path strip already names the file — with its icon, dirty dot and
        // toolbar. A title row above it printed the filename twice over, so
        // the controls join the strip instead.
        headerless
      >
        <FileEditorPane
          key={filePath}
          cwd={cwd}
          filePath={filePath}
          remoteHostId={remoteHostId}
          dirtyRef={dirtyRefFor(key)}
          onClose={handleClose}
          controls={
            isCard ? (
              <>
                <PaneOwnerLabel sessionId={sessionId} />
                <PromotedCardControls
                  cardId={key}
                  title={fileName}
                  onClose={handleClose}
                  className="shrink-0"
                />
              </>
            ) : (
              <PaneControls
                paneId={paneId}
                title={fileName}
                // Distinct from the tree's per-row control, which names the
                // file — a session with both open would otherwise carry two
                // buttons reading exactly alike.
                popOutLabel="this file"
                onPopOut={handlePopOut}
                onClose={handleClose}
                className="shrink-0"
              />
            )
          }
          headerClassName={
            onDragStart || flexible ? 'drag-handle cursor-grab active:cursor-grabbing' : ''
          }
          onHeaderPointerDown={onDragStart ? (e) => onDragStart(paneId, e) : undefined}
          onHeaderDoubleClick={toggleMaximize}
          headerTestId="editor-pane-header"
        />
      </PaneCard>
    )
  })
)
