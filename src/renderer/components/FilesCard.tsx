import { memo, forwardRef } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { useAppStore } from '../stores'
import { PaneCard } from './PaneCard'
import { FileTreePane } from './FileTreeExplorer'
import { filesPaneId } from '../lib/pane-id'
import { confirmDiscard } from '../lib/editor-dirty'

interface Props {
  /** Session that owns this tree. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

/**
 * A session's file tree, as its own grid pane.
 *
 * Independent of that session's editor pane: closing this leaves an open file
 * open. Selecting a file here routes through the store (`openEditorPane`) rather
 * than a prop, so the two panes have no parent/child relationship.
 */
export const FilesCard = memo(
  forwardRef<HTMLDivElement, Props>(function FilesCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, selectedFile, openEditorPane, promoteFile, closeFilesPane } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        selectedFile: s.editorPanes.get(sessionId)?.filePath ?? null,
        openEditorPane: s.openEditorPane,
        promoteFile: s.promoteFile,
        closeFilesPane: s.closeFilesPane
      }))
    )

    if (!terminal) return null

    const cwd = terminal.session.worktreePath || terminal.session.projectPath
    const remoteHostId = terminal.session.remoteHostId

    const paneId = filesPaneId(sessionId)
    const handleClose = (): void => closeFilesPane(sessionId)

    return (
      <PaneCard
        ref={ref}
        paneId={paneId}
        title="Files"
        onClose={handleClose}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
      >
        {/* The pane keeps its own title row. Folding the controls into the
            filter made that field the title bar: full width, buttons reading as
            part of the input. Drag and double-click-to-maximize live on that
            title row now, so the filter row carries neither. */}
        <FileTreePane
          key={cwd}
          cwd={cwd}
          remoteHostId={remoteHostId}
          selectedFile={selectedFile}
          headerTestId="files-pane-header"
          onSelectFile={(path) => {
            // Swapping the editor's file discards its buffer — confirm first.
            if (path !== selectedFile && !confirmDiscard(sessionId)) return
            openEditorPane(sessionId, path)
          }}
          // A card of its own displaces nothing, so unlike selecting a file
          // there is no buffer at risk and nothing to confirm.
          onPopOutFile={(path) => promoteFile(sessionId, path)}
        />
      </PaneCard>
    )
  })
)
