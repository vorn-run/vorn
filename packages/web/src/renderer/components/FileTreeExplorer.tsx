import {
  memo,
  useState,
  useEffect,
  useCallback,
  useMemo,
  useRef,
  type JSX,
  type ReactNode
} from 'react'
import type { FileStamp } from '../../shared/types'
import { forgetDraft, hasMoved, readDraft, writeDraft } from '../lib/editor-drafts'

/** How long typing has to stop before the draft is worth a synchronous write. */
const DRAFT_SETTLE_MS = 400
import type { FileEntry } from '../../shared/types'
import { ChevronRight, Loader2, X, Search, Save, SquareArrowOutUpRight, Undo2 } from 'lucide-react'
import { isTruncatedRead } from '@vornrun/shared/string-utils'
import { FileTypeIcon } from './file-icons'
import { PANE_SURFACE } from '../lib/pane-surface'
import { ICON_BUTTON, ICON_BUTTON_SIZE } from '../lib/icon-button'
import { Tooltip } from './Tooltip'
import { CodeEditor } from './code-editor/CodeEditor'
import { computeMatches, renderLineWithMarks, type FindMatch } from './code-editor/find'
import { getLang } from './code-editor/shiki'
import { useHighlightedLines } from './code-editor/useHighlightedLines'

const MAX_PREVIEW_LINES = 2000
const ROW_HEIGHT = 22 // px per tree row
const INDENT_WIDTH = 16 // px per depth level
const BASE_LEFT = 8 // px left gutter

// ---------------------------------------------------------------------------
// Filter helpers
// ---------------------------------------------------------------------------
function computeFilterSets(
  rootEntries: FileEntry[],
  dirCache: Map<string, FileEntry[]>,
  filter: string
): { matched: Set<string>; expand: Set<string> } {
  const matched = new Set<string>()
  const expand = new Set<string>()
  if (!filter) return { matched, expand }
  const lc = filter.toLowerCase()

  function visit(entry: FileEntry): boolean {
    const selfMatch = entry.name.toLowerCase().includes(lc)
    if (entry.isDirectory) {
      let descendantMatch = false
      const children = dirCache.get(entry.path)
      if (children) {
        for (const child of children) {
          if (visit(child)) descendantMatch = true
        }
      }
      if (descendantMatch) expand.add(entry.path)
      if (selfMatch || descendantMatch) {
        matched.add(entry.path)
        return true
      }
      return false
    } else {
      if (selfMatch) matched.add(entry.path)
      return selfMatch
    }
  }

  for (const e of rootEntries) visit(e)
  return { matched, expand }
}

// ---------------------------------------------------------------------------
// Tree node
// ---------------------------------------------------------------------------
function TreeNode({
  entry,
  depth,
  dirCache,
  loadDir,
  selectedFile,
  onSelectFile,
  onPinFile,
  onPopOutFile,
  filter,
  matched,
  forceExpand
}: {
  entry: FileEntry
  depth: number
  dirCache: Map<string, FileEntry[]>
  loadDir: (path: string) => Promise<void>
  selectedFile: string | null
  onSelectFile: (path: string) => void
  onPinFile?: (path: string) => void
  onPopOutFile?: (path: string) => void
  filter: string
  matched: Set<string>
  forceExpand: Set<string>
}) {
  const [expanded, setExpanded] = useState(false)
  const [loading, setLoading] = useState(false)

  const filterActive = filter.length > 0
  if (filterActive && !matched.has(entry.path)) return null

  const handleToggle = async (): Promise<void> => {
    if (!entry.isDirectory) return
    if (!expanded && !dirCache.has(entry.path)) {
      setLoading(true)
      await loadDir(entry.path)
      setLoading(false)
    }
    setExpanded(!expanded)
  }

  const effectivelyExpanded = expanded || (filterActive && forceExpand.has(entry.path))
  const children = dirCache.get(entry.path)
  const isSelected = !entry.isDirectory && selectedFile === entry.path

  // Indent guides: one vertical line per depth level
  const guides: JSX.Element[] = []
  for (let i = 0; i < depth; i++) {
    guides.push(
      <span
        key={i}
        className="absolute top-0 bottom-0 border-l border-white/[0.06] pointer-events-none"
        aria-hidden="true"
        style={{ left: `${BASE_LEFT + 7 + i * INDENT_WIDTH}px` }}
      />
    )
  }

  if (entry.isDirectory) {
    return (
      <div>
        <button
          onClick={handleToggle}
          className="group relative w-full flex items-center gap-[5px] pr-2 text-left text-[13.5px] transition-colors hover:bg-white/[0.05]"
          style={{ height: ROW_HEIGHT, paddingLeft: `${BASE_LEFT + depth * INDENT_WIDTH}px` }}
        >
          {guides}
          {loading ? (
            <Loader2
              size={14}
              className="text-gray-600 animate-spin shrink-0"
              style={{ width: 14, height: 14 }}
            />
          ) : (
            <ChevronRight
              size={14}
              strokeWidth={2}
              className={`text-gray-500 shrink-0 transition-transform duration-100 ${effectivelyExpanded ? 'rotate-90' : ''}`}
              style={{ width: 14, height: 14 }}
            />
          )}
          <span className="truncate text-gray-300 leading-none">{entry.name}</span>
        </button>
        {effectivelyExpanded && children && (
          <div>
            {children.map((child) => (
              <TreeNode
                key={child.path}
                entry={child}
                depth={depth + 1}
                dirCache={dirCache}
                loadDir={loadDir}
                selectedFile={selectedFile}
                onSelectFile={onSelectFile}
                onPinFile={onPinFile}
                onPopOutFile={onPopOutFile}
                filter={filter}
                matched={matched}
                forceExpand={forceExpand}
              />
            ))}
            {children.length === 0 && (
              <div
                className="relative text-[11px] text-gray-600 italic leading-none flex items-center"
                style={{
                  height: ROW_HEIGHT,
                  paddingLeft: `${BASE_LEFT + (depth + 1) * INDENT_WIDTH + 16}px`
                }}
              >
                {[
                  ...guides,
                  <span
                    key={depth}
                    className="absolute top-0 bottom-0 border-l border-white/[0.06] pointer-events-none"
                    aria-hidden="true"
                    style={{ left: `${BASE_LEFT + 7 + depth * INDENT_WIDTH}px` }}
                  />
                ]}
                empty
              </div>
            )}
          </div>
        )}
      </div>
    )
  }

  // A div rather than a button: the row carries a button of its own, and a
  // button inside a button is invalid markup that browsers resolve by dropping
  // the inner one — which would leave pop-out unclickable.
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onSelectFile(entry.path)}
      onDoubleClick={onPinFile ? () => onPinFile(entry.path) : undefined}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onSelectFile(entry.path)
        }
      }}
      className={`group relative w-full flex items-center gap-[5px] pr-1 text-left text-[13.5px]
        cursor-default select-none transition-colors
        ${isSelected ? 'bg-white/[0.10] text-gray-100' : 'hover:bg-white/[0.05] text-gray-400'}`}
      style={{ height: ROW_HEIGHT, paddingLeft: `${BASE_LEFT + depth * INDENT_WIDTH + 16}px` }}
    >
      {guides}
      <FileTypeIcon name={entry.name} size={16} />
      <span
        className={`truncate leading-none flex-1 ${isSelected ? 'text-gray-200' : 'text-gray-400 group-hover:text-gray-300'}`}
      >
        {entry.name}
      </span>
      {onPopOutFile && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            onPopOutFile(entry.path)
          }}
          aria-label={`Open ${entry.name} as its own card`}
          title="Open as its own card"
          // Hover-revealed, unlike the pane's controls: at rest it would be a column of arrows.
          className="shrink-0 p-0.5 rounded text-ink-ghost hover:text-white hover:bg-white/[0.08]
                     opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity"
        >
          <SquareArrowOutUpRight size={11} strokeWidth={2} />
        </button>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Line row
// ---------------------------------------------------------------------------
function LineRow({
  lineNum,
  children,
  rowRef
}: {
  lineNum: number
  children: React.ReactNode
  rowRef?: (el: HTMLDivElement | null) => void
}) {
  return (
    <div ref={rowRef} className="flex select-text hover:bg-white/[0.02]">
      <span className="w-[44px] shrink-0 text-right pr-3 text-[12px] text-gray-600 select-none">
        {lineNum}
      </span>
      {children}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Read view
// ---------------------------------------------------------------------------
function ReadView({
  filePath,
  content,
  findQuery,
  activeMatchIdx,
  onMatchesComputed
}: {
  filePath: string
  content: string
  findQuery: string
  activeMatchIdx: number
  onMatchesComputed: (count: number) => void
}) {
  const allLines = useMemo(() => content.split('\n'), [content])
  const fileName = filePath.split(/[/\\]/).pop() || filePath
  const capped = allLines.length > MAX_PREVIEW_LINES
  const visibleLines = useMemo(
    () => (capped ? allLines.slice(0, MAX_PREVIEW_LINES) : allLines),
    [allLines, capped]
  )
  const visibleText = useMemo(() => visibleLines.join('\n'), [visibleLines])
  const highlighted = useHighlightedLines(visibleText, getLang(fileName))

  const matches = useMemo(() => computeMatches(visibleLines, findQuery), [visibleLines, findQuery])
  const matchesByLine = useMemo(() => {
    const m = new Map<number, FindMatch[]>()
    matches.forEach((mm) => {
      const arr = m.get(mm.line) ?? []
      arr.push(mm)
      m.set(mm.line, arr)
    })
    return m
  }, [matches])

  useEffect(() => {
    onMatchesComputed(matches.length)
  }, [matches.length, onMatchesComputed])

  const rowRefs = useRef(new Map<number, HTMLDivElement | null>())

  useEffect(() => {
    if (matches.length === 0) return
    const m = matches[activeMatchIdx % matches.length]
    if (!m) return
    const el = rowRefs.current.get(m.line)
    el?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [activeMatchIdx, matches])

  const findActive = findQuery.length > 0
  const activeMatch =
    findActive && matches.length > 0 ? matches[activeMatchIdx % matches.length] : null

  const renderedLines = useMemo<JSX.Element[]>(() => {
    if (findActive) {
      return visibleLines.map((line, i) => {
        const lm = matchesByLine.get(i) ?? []
        const marks = lm.map((m) => ({
          start: m.start,
          end: m.end,
          active: !!activeMatch && activeMatch.line === i && activeMatch.start === m.start
        }))
        return (
          <LineRow
            key={i}
            lineNum={i + 1}
            rowRef={(el) => {
              if (el) rowRefs.current.set(i, el)
              else rowRefs.current.delete(i)
            }}
          >
            <span className="text-gray-300 px-1 flex-1 whitespace-pre">
              {renderLineWithMarks(line, marks)}
            </span>
          </LineRow>
        )
      })
    }

    if (highlighted) {
      return highlighted.map((tokens, i) => (
        <LineRow key={i} lineNum={i + 1}>
          <span className="px-1 flex-1 whitespace-pre">
            {tokens.map((t, j) => (
              <span key={j} style={t.color ? { color: t.color } : undefined}>
                {t.content}
              </span>
            ))}
            {tokens.length === 0 && ' '}
          </span>
        </LineRow>
      ))
    }

    return visibleLines.map((line, i) => (
      <LineRow key={i} lineNum={i + 1}>
        <span className="text-gray-400 px-1 flex-1 whitespace-pre">{line || ' '}</span>
      </LineRow>
    ))
  }, [findActive, visibleLines, highlighted, matchesByLine, activeMatch])

  return (
    <div className="flex-1 overflow-y-auto">
      <pre className="text-[13px] leading-[1.65] font-mono">
        {renderedLines}
        {capped && (
          <div className="px-3 py-2 text-[11px] text-gray-600 italic">
            Showing first {MAX_PREVIEW_LINES} of {allLines.length} lines
          </div>
        )}
      </pre>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Edit view
// ---------------------------------------------------------------------------
/** The file's text in the shared code editor, labelled and coloured by its name. */
function EditView({
  draft,
  fileName,
  onChange,
  onSaveShortcut,
  findQuery,
  activeMatchIdx,
  onMatchesComputed
}: {
  draft: string
  fileName: string
  onChange: (next: string) => void
  onSaveShortcut: () => void
  findQuery: string
  activeMatchIdx: number
  onMatchesComputed: (count: number) => void
}) {
  return (
    <CodeEditor
      value={draft}
      onChange={onChange}
      fileName={fileName}
      ariaLabel={`Edit ${fileName}`}
      onSaveShortcut={onSaveShortcut}
      find={{ query: findQuery, activeIndex: activeMatchIdx, onMatchesComputed }}
    />
  )
}

// ---------------------------------------------------------------------------
// File panel
// ---------------------------------------------------------------------------
/** Why a file is shown but cannot be typed in, or null when it can. */
type ReadOnlyReason = 'binary' | 'unreadable' | 'truncated' | null

function FilePanel({
  cwd,
  filePath,
  content,
  loading,
  readOnly,
  onContentSaved,
  remoteHostId,
  dirtyRef,
  draftKey,
  controls,
  onHeaderPointerDown,
  headerTestId,
  headerClassName = ''
}: {
  cwd: string
  filePath: string
  content: string | null
  loading: boolean
  readOnly: ReadOnlyReason
  onContentSaved: (next: string) => void
  remoteHostId?: string
  dirtyRef: React.MutableRefObject<boolean>
  /** Where an unsaved edit is kept, so a quit does not throw it away. Keyed by tab or card, not by path. */
  draftKey?: string
  /** Pane chrome seated in the path strip, for a host with no title bar of its own. */
  controls?: ReactNode
  onHeaderPointerDown?: (e: React.PointerEvent) => void
  headerTestId?: string
  headerClassName?: string
}) {
  const [draft, setDraft] = useState('')
  /** True once the buffer holds this file, or its restored draft. */
  const [ready, setReady] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  /** What the file was when this buffer was taken from it. State, so a late stamp reaches the stored draft. */
  const [base, setBase] = useState<FileStamp | null>(null)
  /** The file on screen right now, for answers that arrive after it changed. */
  const pathRef = useRef(filePath)
  /** Set when the file moved under the draft. Cleared by whichever way out is taken. */
  const [conflict, setConflict] = useState(false)

  const [findOpen, setFindOpen] = useState(false)
  const [findQuery, setFindQuery] = useState('')
  const [findCount, setFindCount] = useState(0)
  const [findIdx, setFindIdx] = useState(0)
  const findInputRef = useRef<HTMLInputElement | null>(null)

  const editable = readOnly === null && content !== null && !loading

  // A failed stamp leaves the guard unarmed; one for another file is dropped.
  const stampBase = useCallback((): void => {
    const stamped = filePath
    window.api
      .fileStamp?.(filePath, remoteHostId)
      .then((stamp) => {
        if (pathRef.current === stamped) setBase(stamp ?? null)
      })
      .catch(() => {})
  }, [filePath, remoteHostId])

  // Reset transient state when file changes
  useEffect(() => {
    pathRef.current = filePath
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: clear per-file edit/find state when the file changes
    setReady(false)
    setDraft('')
    setSaveError(null)
    setConflict(false)
    setBase(null)
    setFindOpen(false)
    setFindQuery('')
    setFindIdx(0)
  }, [filePath])

  // Take the file into the buffer once it has loaded, or the draft left over it.
  useEffect(() => {
    if (ready || !editable || content === null) return
    const kept = draftKey ? readDraft(draftKey, filePath) : null
    /* eslint-disable react-hooks/set-state-in-effect -- intentional: the file and its draft are external state, read in once loaded */
    setReady(true)
    if (!kept || kept.text === content) {
      // A draft that agrees with the file has already landed; drop the record.
      if (kept && draftKey) forgetDraft(draftKey)
      setDraft(content)
      stampBase()
      return
    }
    setBase(kept.base)
    setDraft(kept.text)
    /* eslint-enable react-hooks/set-state-in-effect */
    // The file can have moved while the app was closed, so ask now rather than at save.
    let stale = false
    window.api
      .fileStamp?.(filePath, remoteHostId)
      .then((current) => {
        if (!stale) setConflict(hasMoved(kept.base, current ?? null))
      })
      .catch(() => {
        if (!stale) setConflict(false)
      })
    return () => {
      stale = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `ready` gates this to once per load
  }, [draftKey, filePath, remoteHostId, editable, content])

  useEffect(() => {
    if (findOpen) findInputRef.current?.focus()
  }, [findOpen])

  const fileName = filePath.split(/[/\\]/).pop() || filePath
  const relPath = useMemo(() => {
    if (cwd && filePath.startsWith(cwd)) {
      const rel = filePath.slice(cwd.length).replace(/^[\\/]+/, '')
      return rel || fileName
    }
    return filePath
  }, [filePath, cwd, fileName])

  const dirty = ready && editable && draft !== content
  const canFind = content !== null && !loading

  useEffect(() => {
    dirtyRef.current = dirty
    return () => {
      dirtyRef.current = false
    }
  }, [dirty, dirtyRef])

  const handleDiscard = (): void => {
    if (content === null) return
    if (dirty && !window.confirm('Discard unsaved changes?')) return
    if (draftKey) forgetDraft(draftKey)
    setDraft(content)
    setSaveError(null)
    setConflict(false)
  }

  // Keep the edit past the window. Delayed: this fires per keystroke and the write is synchronous.
  useEffect(() => {
    if (!draftKey || !ready || content === null) return
    if (draft === content) {
      forgetDraft(draftKey)
      return
    }
    const timer = setTimeout(
      () => writeDraft(draftKey, { filePath, text: draft, base }),
      DRAFT_SETTLE_MS
    )
    return () => clearTimeout(timer)
    // `base` included on purpose: a stamp that lands after the first write has to reach the record.
  }, [draftKey, ready, draft, content, filePath, base])

  /** Write the buffer to disk. `force` skips the moved-file check, and only the conflict banner passes it. */
  const handleSave = useCallback(
    async (force = false): Promise<void> => {
      if (!dirty) return
      setSaving(true)
      setSaveError(null)
      try {
        if (!force) {
          const current = (await window.api.fileStamp?.(filePath, remoteHostId)) ?? null
          if (hasMoved(base, current)) {
            setConflict(true)
            return
          }
        }
        const res = await window.api.writeFileContent(filePath, draft, remoteHostId)
        if (!res.success) {
          setSaveError(res.error || 'Failed to save')
          return
        }
        if (draftKey) forgetDraft(draftKey)
        onContentSaved(draft)
        setConflict(false)
        setBase(null)
        stampBase()
      } catch (err) {
        setSaveError(err instanceof Error ? err.message : String(err))
      } finally {
        setSaving(false)
      }
    },
    [filePath, dirty, draft, remoteHostId, onContentSaved, draftKey, base, stampBase]
  )

  /** Throw the draft away and take what is on disk. */
  const handleTakeDisk = useCallback(async (): Promise<void> => {
    const next = await window.api.readFileContent(filePath, undefined, remoteHostId)
    if (next === null) {
      setSaveError('The file could not be read.')
      return
    }
    if (draftKey) forgetDraft(draftKey)
    onContentSaved(next)
    setDraft(next)
    setConflict(false)
    setBase(null)
    stampBase()
  }, [filePath, remoteHostId, onContentSaved, draftKey, stampBase])

  const handleToggleFind = (): void => {
    if (!canFind) return
    setFindOpen((v) => !v)
  }

  const cycleMatch = (delta: number): void => {
    if (findCount === 0) return
    setFindIdx((i) => (i + delta + findCount) % findCount)
  }

  const onMatchesComputed = useCallback((count: number) => {
    setFindCount(count)
    setFindIdx((i) => (count === 0 ? 0 : Math.min(i, count - 1)))
  }, [])

  return (
    <div className="flex-1 flex flex-col min-h-0">
      {/* Path strip + toolbar — the title bar too, for a host with none of its own. */}
      <div
        className={`flex items-center gap-1 pl-2 pr-1 py-0.5 text-[11px] font-mono shrink-0 ${headerClassName}`}
        style={{ background: PANE_SURFACE }}
        onPointerDown={onHeaderPointerDown}
        data-testid={headerTestId}
      >
        <FileTypeIcon name={fileName} size={12} />
        <span className="text-gray-400 flex-1 min-w-0 truncate ml-1" title={filePath} dir="rtl">
          {relPath}
        </span>
        {dirty && (
          <span
            className="w-[6px] h-[6px] rounded-full bg-amber-400 shrink-0 mx-1"
            title="Unsaved changes"
          />
        )}
        <ToolbarBtn
          icon={Search}
          label="Find in file"
          active={findOpen}
          disabled={!canFind}
          onClick={handleToggleFind}
        />
        {dirty && (
          <>
            <ToolbarBtn
              icon={Save}
              label={saving ? 'Saving…' : 'Save'}
              shortcut="⌘S"
              disabled={saving}
              onClick={() => void handleSave()}
            />
            <ToolbarBtn icon={Undo2} label="Discard changes" onClick={handleDiscard} />
          </>
        )}
        {controls}
      </div>

      {/* Find bar */}
      {findOpen && canFind && (
        <div
          className="flex items-center gap-2 px-3 py-1 text-[11px] shrink-0"
          style={{ background: PANE_SURFACE }}
        >
          <Search size={12} className="text-gray-500 shrink-0" />
          <input
            ref={findInputRef}
            value={findQuery}
            onChange={(e) => {
              setFindQuery(e.target.value)
              setFindIdx(0)
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                cycleMatch(e.shiftKey ? -1 : 1)
              } else if (e.key === 'Escape') {
                e.preventDefault()
                setFindOpen(false)
                setFindQuery('')
              }
            }}
            placeholder="Find in file"
            className="flex-1 bg-transparent text-gray-200 outline-none text-[13px] font-mono"
          />
          <span className="text-gray-500 shrink-0 tabular-nums">
            {findCount === 0 ? '0/0' : `${findIdx + 1}/${findCount}`}
          </span>
          <button
            onClick={() => cycleMatch(-1)}
            disabled={findCount === 0}
            className="text-gray-500 hover:text-white px-1 disabled:opacity-40"
            title="Previous (Shift+Enter)"
          >
            ↑
          </button>
          <button
            onClick={() => cycleMatch(1)}
            disabled={findCount === 0}
            className="text-gray-500 hover:text-white px-1 disabled:opacity-40"
            title="Next (Enter)"
          >
            ↓
          </button>
          <button
            onClick={() => {
              setFindOpen(false)
              setFindQuery('')
            }}
            className="text-gray-500 hover:text-white p-0.5"
            title="Close (Esc)"
          >
            <X size={12} />
          </button>
        </div>
      )}

      {/* The file moved under the draft. Three ways out, none of them taken for
          the person: saving over an agent's work and losing an afternoon of
          your own are both worse than being asked. */}
      {conflict && (
        <div className="px-3 py-1.5 text-[11px] text-bronzo bg-bronzo/10 border-t border-white/[0.06] shrink-0 flex items-center gap-3 flex-wrap">
          <span className="flex-1 min-w-0">
            This file changed on disk while your edit was open.
          </span>
          <button
            type="button"
            className="text-gray-300 hover:text-gray-100 underline underline-offset-2"
            onClick={() => void handleSave(true)}
            disabled={saving}
          >
            Save mine anyway
          </button>
          <button
            type="button"
            className="text-gray-300 hover:text-gray-100 underline underline-offset-2"
            onClick={() => void handleTakeDisk()}
          >
            Discard mine
          </button>
          <button
            type="button"
            className="text-gray-300 hover:text-gray-100 underline underline-offset-2"
            onClick={() => setConflict(false)}
          >
            Keep editing
          </button>
        </div>
      )}

      {readOnly === 'truncated' && (
        <div className="px-3 py-1 text-[11px] text-ink-faint border-t border-white/[0.06] shrink-0">
          Too large to edit here. Showing the first part, read-only.
        </div>
      )}

      {/* Body */}
      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <Loader2 size={16} className="text-gray-500 animate-spin" />
        </div>
      ) : readOnly === 'binary' || readOnly === 'unreadable' ? (
        <div className="flex-1 flex items-center justify-center text-gray-600 text-[13px]">
          {readOnly === 'binary'
            ? 'Binary file — preview unavailable'
            : 'This file could not be read'}
        </div>
      ) : content === null ? null : editable && ready ? (
        <EditView
          draft={draft}
          fileName={fileName}
          onChange={setDraft}
          onSaveShortcut={() => void handleSave()}
          findQuery={findOpen ? findQuery : ''}
          activeMatchIdx={findIdx}
          onMatchesComputed={onMatchesComputed}
        />
      ) : editable ? null : (
        <ReadView
          filePath={filePath}
          content={content}
          findQuery={findOpen ? findQuery : ''}
          activeMatchIdx={findIdx}
          onMatchesComputed={onMatchesComputed}
        />
      )}

      {saveError && (
        <div className="px-3 py-1 text-[11px] text-danger bg-danger/10 border-t border-white/[0.06] shrink-0">
          {saveError}
        </div>
      )}
    </div>
  )
}

function ToolbarBtn({
  icon: Icon,
  label,
  shortcut,
  active,
  disabled,
  onClick
}: {
  icon: typeof Search
  label: string
  shortcut?: string
  active?: boolean
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <Tooltip label={label} shortcut={shortcut}>
      <button
        onClick={onClick}
        disabled={disabled}
        aria-label={label}
        className={`${ICON_BUTTON} shrink-0 ${active ? 'bg-white/[0.10]' : ''} ${
          disabled ? 'opacity-40 cursor-not-allowed' : ''
        }`}
      >
        <Icon size={ICON_BUTTON_SIZE} strokeWidth={2} />
      </button>
    </Tooltip>
  )
}

// ---------------------------------------------------------------------------
// Files panel (filter + tree)
// ---------------------------------------------------------------------------
function FilesPanel({
  rootEntries,
  dirCache,
  loadDir,
  selectedFile,
  onSelectFile,
  onPinFile,
  onPopOutFile,
  headerTestId
}: {
  rootEntries: FileEntry[]
  dirCache: Map<string, FileEntry[]>
  loadDir: (path: string) => Promise<void>
  selectedFile: string | null
  onSelectFile: (path: string) => void
  onPinFile?: (path: string) => void
  /** Open a file as a card of its own. Absent where there is no grid to put a card in. */
  onPopOutFile?: (path: string) => void
  headerTestId?: string
}) {
  const [filter, setFilter] = useState('')
  const { matched, expand } = useMemo(
    () => computeFilterSets(rootEntries, dirCache, filter),
    [rootEntries, dirCache, filter]
  )

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-1 px-1.5 py-1.5 shrink-0" data-testid={headerTestId}>
        {/* A search field has to read as somewhere you can type before anything
            is in it; at 4% over a near-black pane it was very nearly the pane. */}
        <div className="flex items-center gap-1.5 flex-1 min-w-0 px-2 py-1 rounded bg-white/[0.09] focus-within:bg-white/[0.13] transition-colors">
          <Search size={13} className="text-gray-600 shrink-0" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setFilter('')
            }}
            placeholder="Filter files…"
            className="flex-1 min-w-0 bg-transparent text-gray-200 outline-none text-[13px] placeholder:text-gray-600"
          />
          {filter && (
            <button
              onClick={() => setFilter('')}
              className="text-gray-600 hover:text-white p-0.5"
              aria-label="Clear filter"
            >
              <X size={11} />
            </button>
          )}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto py-0.5">
        {rootEntries.map((entry) => (
          <TreeNode
            key={entry.path}
            entry={entry}
            depth={0}
            dirCache={dirCache}
            loadDir={loadDir}
            selectedFile={selectedFile}
            onSelectFile={onSelectFile}
            onPinFile={onPinFile}
            onPopOutFile={onPopOutFile}
            filter={filter}
            matched={matched}
            forceExpand={expand}
          />
        ))}
        {filter && matched.size === 0 && (
          <div className="px-3 py-2 text-[11px] text-gray-600 italic">No matching files loaded</div>
        )}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// The two halves of the Files pane. Each owns its own loading.
// ---------------------------------------------------------------------------

/** The file tree. A click reports `onSelectFile`; a double click, `onPinFile`. */
export function FileTreePane({
  cwd,
  remoteHostId,
  selectedFile,
  onSelectFile,
  onPinFile,
  onPopOutFile,
  headerTestId
}: {
  cwd: string
  remoteHostId?: string
  selectedFile: string | null
  onSelectFile: (path: string) => void
  onPinFile?: (path: string) => void
  onPopOutFile?: (path: string) => void
  headerTestId?: string
}): JSX.Element {
  const [rootEntries, setRootEntries] = useState<FileEntry[] | null>(null)
  const [loading, setLoading] = useState(true)
  const [dirCache, setDirCache] = useState(() => new Map<string, FileEntry[]>())

  useEffect(() => {
    let stale = false
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: reset cache when cwd/host changes
    setDirCache(new Map())
    setLoading(true)
    window.api
      .listDir(cwd, remoteHostId)
      .then((entries) => {
        if (stale) return
        setRootEntries(entries)
        setLoading(false)
      })
      .catch(() => {
        if (!stale) setLoading(false)
      })
    return () => {
      stale = true
    }
  }, [cwd, remoteHostId])

  const loadDir = useCallback(
    async (dirPath: string) => {
      const entries = await window.api.listDir(dirPath, remoteHostId)
      setDirCache((prev) => {
        if (prev.has(dirPath)) return prev
        const next = new Map(prev)
        next.set(dirPath, entries)
        return next
      })
    },
    [remoteHostId]
  )

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 size={20} className="text-gray-500 animate-spin" />
      </div>
    )
  }

  if (!rootEntries || rootEntries.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-gray-500 text-sm">
        Empty directory
      </div>
    )
  }

  return (
    <FilesPanel
      rootEntries={rootEntries}
      dirCache={dirCache}
      loadDir={loadDir}
      selectedFile={selectedFile}
      onSelectFile={onSelectFile}
      onPinFile={onPinFile}
      onPopOutFile={onPopOutFile}
      headerTestId={headerTestId}
    />
  )
}

/** One open file, owning the load of `filePath`. It opens ready to type in. */
function FileEditorPaneImpl({
  cwd,
  filePath,
  remoteHostId,
  dirtyRef: externalDirtyRef,
  draftKey,
  controls,
  onHeaderPointerDown,
  headerTestId,
  headerClassName
}: {
  cwd: string
  filePath: string
  remoteHostId?: string
  /** Where an unsaved edit is kept; see `FilePanel`. */
  draftKey?: string
  /** Pane chrome seated in the path strip; see `FilePanel`. */
  controls?: ReactNode
  onHeaderPointerDown?: (e: React.PointerEvent) => void
  headerTestId?: string
  headerClassName?: string
  /** Set while the buffer has unsaved edits, for whoever is about to close it. */
  dirtyRef?: React.MutableRefObject<boolean>
}): JSX.Element {
  const [content, setContent] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [readOnly, setReadOnly] = useState<ReadOnlyReason>(null)
  const localDirtyRef = useRef(false)
  const dirtyRef = externalDirtyRef ?? localDirtyRef

  useEffect(() => {
    let stale = false
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: reset to a loading state when the file changes
    setLoading(true)
    setContent(null)
    setReadOnly(null)
    const load = async (): Promise<void> => {
      const next = await window.api.readFileContent(filePath, undefined, remoteHostId)
      if (stale) return
      if (next !== null) {
        setReadOnly(isTruncatedRead(next) ? 'truncated' : null)
        setContent(next)
        return
      }
      // A null read is a binary file or a failed one; only a stat can tell them apart.
      const stamp = await window.api.fileStamp?.(filePath, remoteHostId)?.catch(() => null)
      if (!stale) setReadOnly(stamp === null ? 'unreadable' : 'binary')
    }
    load()
      .catch(() => {
        if (!stale) setReadOnly('unreadable')
      })
      .finally(() => {
        if (!stale) setLoading(false)
      })
    return () => {
      stale = true
    }
  }, [filePath, remoteHostId])

  const handleContentSaved = useCallback((next: string) => {
    // A re-read from disk can come back capped even when the first read did not.
    if (isTruncatedRead(next)) setReadOnly('truncated')
    setContent(next)
  }, [])

  return (
    <FilePanel
      cwd={cwd}
      filePath={filePath}
      content={content}
      loading={loading}
      readOnly={readOnly}
      onContentSaved={handleContentSaved}
      remoteHostId={remoteHostId}
      dirtyRef={dirtyRef}
      draftKey={draftKey}
      controls={controls}
      onHeaderPointerDown={onHeaderPointerDown}
      headerTestId={headerTestId}
      headerClassName={headerClassName}
    />
  )
}

/** Memoised: a Files pane keeps every open tab mounted, and its own re-renders should not reach them. */
export const FileEditorPane = memo(FileEditorPaneImpl)
