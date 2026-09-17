import { memo, forwardRef, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useShallow } from 'zustand/react/shallow'
import { ListTree } from 'lucide-react'
import { useAppStore } from '../stores'
import { PaneCard, PaneControls } from './PaneCard'
import { PaneTabStrip, type PaneTab } from './PaneTabStrip'
import { FileTreePane, FileEditorPane } from './FileTreeExplorer'
import { FileTypeIcon } from './file-icons'
import { SplitDivider } from './SplitDivider'
import { Tooltip } from './Tooltip'
import { filesPaneId, fileTabKey } from '../lib/pane-id'
import { ICON_BUTTON, ICON_BUTTON_SIZE } from '../lib/icon-button'
import { clampSplitRatio } from '../lib/split-ratio'
import {
  confirmDiscard,
  confirmDiscardAll,
  dirtyRefFor,
  isEditorDirty,
  useDirtyVersion
} from '../lib/editor-dirty'

const SPLIT_RATIO_KEY = 'vorn:files-split-ratio'
const DEFAULT_TREE_RATIO = 0.38
/** Below this width the tree and the file take turns rather than sharing the pane. */
const SIDE_BY_SIDE_MIN_PX = 480

interface Props {
  /** Session that owns this pane. */
  sessionId: string
  isDragTarget?: boolean
  onDragStart?: (paneId: string, e: React.PointerEvent) => void
  flexible?: boolean
}

function loadTreeRatio(): number {
  try {
    const n = Number(localStorage.getItem(SPLIT_RATIO_KEY))
    return Number.isFinite(n) && n > 0 ? clampSplitRatio(n) : DEFAULT_TREE_RATIO
  } catch {
    return DEFAULT_TREE_RATIO
  }
}

function baseName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path
}

function parentName(path: string): string {
  return path.split(/[/\\]/).slice(-2, -1)[0] ?? ''
}

/** A session's files: its tree, and the files open from it as tabs. */
export const FilesCard = memo(
  forwardRef<HTMLDivElement, Props>(function FilesCard(
    { sessionId, isDragTarget, onDragStart, flexible },
    ref
  ) {
    const { terminal, pane } = useAppStore(
      useShallow((s) => ({
        terminal: s.terminals.get(sessionId),
        pane: s.filesPanes.get(sessionId)
      }))
    )
    const bodyRef = useRef<HTMLDivElement>(null)
    const [treeRatio, setTreeRatio] = useState(loadTreeRatio)
    const [narrow, setNarrow] = useState(false)
    const dirtyVersion = useDirtyVersion()

    useEffect(() => {
      const el = bodyRef.current
      if (!el || typeof ResizeObserver === 'undefined') return
      const observer = new ResizeObserver(([entry]) =>
        setNarrow(entry.contentRect.width > 0 && entry.contentRect.width < SIDE_BY_SIDE_MIN_PX)
      )
      observer.observe(el)
      return () => observer.disconnect()
    }, [])

    // A preview tab that has been typed in is kept, or the next click would replace it.
    useEffect(() => {
      const preview = pane?.preview
      if (preview && isEditorDirty(fileTabKey(sessionId, preview)))
        useAppStore.getState().pinFileTab(sessionId, preview)
    }, [dirtyVersion, pane?.preview, sessionId])

    const persistRatio = useCallback((next: number): void => {
      try {
        localStorage.setItem(SPLIT_RATIO_KEY, String(next))
      } catch {
        /* ignore quota errors */
      }
    }, [])

    const paths = pane?.tabs
    const tabs = useMemo<PaneTab[]>(() => {
      const names = (paths ?? []).map(baseName)
      return (paths ?? []).map((path, i) => {
        const shared = names.indexOf(names[i]) !== names.lastIndexOf(names[i])
        return {
          id: path,
          name: names[i],
          title: path,
          label: shared ? (
            <>
              {names[i]} <span className="text-ink-faint">{parentName(path)}</span>
            </>
          ) : undefined,
          icon: <FileTypeIcon name={names[i]} size={12} />,
          italic: pane?.preview === path,
          badge: isEditorDirty(fileTabKey(sessionId, path)) ? (
            <span
              className="w-[6px] h-[6px] rounded-full bg-amber-400"
              title="Unsaved changes"
              data-testid="tab-unsaved"
            />
          ) : undefined
        }
      })
      // eslint-disable-next-line react-hooks/exhaustive-deps -- `dirtyVersion` is what says a badge changed
    }, [paths, pane?.preview, sessionId, dirtyVersion])

    if (!terminal || !pane) return null

    const store = useAppStore.getState()
    const cwd = terminal.session.worktreePath || terminal.session.projectPath
    const remoteHostId = terminal.session.remoteHostId
    const paneId = filesPaneId(sessionId)
    const hasTabs = pane.tabs.length > 0
    const treeShown = !hasTabs || pane.treeVisible
    const fileShown = hasTabs && !(narrow && pane.treeVisible)

    const handleClose = (): void => {
      if (!confirmDiscardAll(pane.tabs.map((path) => fileTabKey(sessionId, path)))) return
      store.closeFilesPane(sessionId)
    }

    const handleCloseTab = (path: string): void => {
      if (confirmDiscard(fileTabKey(sessionId, path))) store.closeFileTab(sessionId, path)
    }

    // The buffer does not travel: the card mounts a fresh editor under its own id.
    const handlePopOutTab = (path: string): void => {
      if (!confirmDiscard(fileTabKey(sessionId, path))) return
      store.promoteFile(sessionId, path)
      store.closeFileTab(sessionId, path)
    }

    const handleOpen = (path: string, pin: boolean): void => {
      store.openFileTab(sessionId, path, { pin })
      if (narrow) store.setFileTreeVisible(sessionId, false)
    }

    const handleSelectTab = (path: string): void => {
      store.setActiveFileTab(sessionId, path)
      if (narrow) store.setFileTreeVisible(sessionId, false)
    }

    const toggleMaximize = (): void => {
      const state = useAppStore.getState()
      state.setMaximizedPane(state.maximizedPaneId === paneId ? null : paneId)
    }

    return (
      <PaneCard
        ref={ref}
        paneId={paneId}
        title="Files"
        onClose={handleClose}
        isDragTarget={isDragTarget}
        onDragStart={onDragStart}
        flexible={flexible}
        // The tab strip is this pane's title bar.
        headerless
      >
        <PaneTabStrip
          ariaLabel="Open files"
          testId="files-pane-header"
          draggable={Boolean(onDragStart || flexible)}
          onPointerDown={onDragStart ? (e) => onDragStart(paneId, e) : undefined}
          onDoubleClick={toggleMaximize}
          tabs={tabs}
          activeId={pane.active}
          emptyTitle="Files"
          onSelect={handleSelectTab}
          onDoubleClickTab={(path) => store.pinFileTab(sessionId, path)}
          onClose={handleCloseTab}
          leading={
            <Tooltip label={treeShown ? 'Hide file tree' : 'Show file tree'}>
              <button
                type="button"
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => store.toggleFileTree(sessionId)}
                disabled={!hasTabs}
                aria-label={treeShown ? 'Hide file tree' : 'Show file tree'}
                aria-pressed={treeShown}
                className={`${ICON_BUTTON} shrink-0 ${treeShown ? 'bg-white/[0.10]' : ''}`}
              >
                <ListTree size={ICON_BUTTON_SIZE} />
              </button>
            </Tooltip>
          }
          menuFooter={(close) => (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                store.closeSavedFileTabs(
                  sessionId,
                  pane.tabs.filter((path) => isEditorDirty(fileTabKey(sessionId, path)))
                )
                close()
              }}
              className="w-full px-2 h-[26px] rounded text-left text-[12px] text-ink-secondary hover:bg-white/[0.05]"
            >
              Close saved tabs
            </button>
          )}
          trailing={
            <PaneControls
              paneId={paneId}
              title="Files"
              popOutLabel="this file"
              onPopOut={pane.active ? () => handlePopOutTab(pane.active as string) : undefined}
              onClose={handleClose}
              className="shrink-0"
            />
          }
        />

        <div ref={bodyRef} className="flex-1 min-h-0 flex pt-0.5">
          <div
            className={treeShown ? 'min-w-0 min-h-0 flex flex-col' : 'hidden'}
            style={{ flex: fileShown ? `${treeRatio} 1 0` : '1 1 0' }}
            data-testid="files-tree-column"
          >
            <FileTreePane
              key={cwd}
              cwd={cwd}
              remoteHostId={remoteHostId}
              selectedFile={pane.active}
              onSelectFile={(path) => handleOpen(path, false)}
              onPinFile={(path) => handleOpen(path, true)}
              // A card of its own displaces nothing, so there is nothing to confirm.
              onPopOutFile={(path) => store.promoteFile(sessionId, path)}
            />
          </div>

          {treeShown && fileShown && (
            <SplitDivider
              axis="x"
              label="Resize file tree"
              containerRef={bodyRef}
              onRatioChange={setTreeRatio}
              onRatioCommit={persistRatio}
              testId={`files-divider-${sessionId}`}
            />
          )}

          {/* Every open file stays mounted, so a tab switch keeps its buffer, undo and scroll. */}
          {pane.tabs.map((path) => {
            const key = fileTabKey(sessionId, path)
            const front = fileShown && path === pane.active
            return (
              <div
                key={path}
                role="tabpanel"
                aria-label={baseName(path)}
                className={front ? 'min-w-0 min-h-0 flex flex-col' : 'hidden'}
                style={{ flex: treeShown ? `${1 - treeRatio} 1 0` : '1 1 0' }}
              >
                <FileEditorPane
                  cwd={cwd}
                  filePath={path}
                  remoteHostId={remoteHostId}
                  dirtyRef={dirtyRefFor(key)}
                  draftKey={key}
                />
              </div>
            )
          })}
        </div>
      </PaneCard>
    )
  })
)
