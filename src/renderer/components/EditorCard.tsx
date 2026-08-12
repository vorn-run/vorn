import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { PaneCard } from './PaneCard'
import { FileEditorPane } from './FileTreeExplorer'
import { FileTypeIcon } from './file-icons'
import { editorPaneId } from '../lib/pane-id'
import { dirtyRefFor, confirmDiscard, clearDirty } from '../lib/editor-dirty'

interface Props {
  /** Session that owns this editor. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * A session's open file, as its own grid pane.
 *
 * Independent of that session's tree pane — it renders whatever path the store
 * holds, whether or not a tree is open, and can be maximized on its own.
 */
export const EditorCard = memo(
  forwardRef<HTMLDivElement, Props>(function EditorCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, filePath, closeEditorPane } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        filePath: s.editorPanes.get(sessionId)?.filePath ?? null,
        closeEditorPane: s.closeEditorPane
      }))
    )

    if (!terminal || !filePath) return null

    const cwd = terminal.session.worktreePath || terminal.session.projectPath
    const remoteHostId = terminal.session.remoteHostId
    const fileName = filePath.split(/[/\\]/).pop() ?? filePath

    // Closing discards the buffer, so confirm first. The editor's own state is
    // out of reach from here — it reports dirtiness through the shared ref.
    const handleClose = (): void => {
      if (!confirmDiscard(sessionId)) return
      clearDirty(sessionId)
      closeEditorPane(sessionId)
    }

    return (
      <PaneCard
        ref={ref}
        paneId={editorPaneId(sessionId)}
        // No subtitle: the editor's own path strip below already shows the
        // relative path, alongside the dirty dot and the find/edit toolbar.
        title={fileName}
        icon={<FileTypeIcon name={fileName} size={12} />}
        onClose={handleClose}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
      >
        <FileEditorPane
          key={filePath}
          cwd={cwd}
          filePath={filePath}
          remoteHostId={remoteHostId}
          dirtyRef={dirtyRefFor(sessionId)}
          onClose={handleClose}
        />
      </PaneCard>
    )
  })
)
