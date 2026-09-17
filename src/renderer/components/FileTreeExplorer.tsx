import { useState, useEffect, useCallback, useMemo, useRef, type JSX, type ReactNode } from 'react'
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
// Shiki syntax highlighting
// ---------------------------------------------------------------------------
const EXT_TO_LANG: Record<string, string> = {
  ts: 'typescript',
  tsx: 'tsx',
  mts: 'typescript',
  cts: 'typescript',
  js: 'javascript',
  jsx: 'jsx',
  mjs: 'javascript',
  cjs: 'javascript',
  json: 'json',
  jsonc: 'jsonc',
  json5: 'json5',
  html: 'html',
  htm: 'html',
  vue: 'vue',
  svelte: 'svelte',
  css: 'css',
  scss: 'scss',
  sass: 'sass',
  less: 'less',
  md: 'markdown',
  mdx: 'mdx',
  py: 'python',
  pyi: 'python',
  rs: 'rust',
  go: 'go',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
  rb: 'ruby',
  php: 'php',
  lua: 'lua',
  zig: 'zig',
  c: 'c',
  h: 'c',
  cpp: 'cpp',
  cc: 'cpp',
  hpp: 'cpp',
  cxx: 'cpp',
  cs: 'csharp',
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  fish: 'fish',
  sql: 'sql',
  graphql: 'graphql',
  gql: 'graphql',
  yml: 'yaml',
  yaml: 'yaml',
  toml: 'toml',
  ini: 'ini',
  xml: 'xml',
  svg: 'xml',
  dockerfile: 'dockerfile',
  makefile: 'makefile',
  r: 'r',
  dart: 'dart',
  ex: 'elixir',
  exs: 'elixir',
  prisma: 'prisma',
  tf: 'hcl',
  ps1: 'powershell',
  bat: 'batch'
}

const FILENAME_TO_LANG: Record<string, string> = {
  dockerfile: 'dockerfile',
  makefile: 'makefile',
  '.gitignore': 'gitignore',
  '.env': 'dotenv'
}

function getLang(name: string): string | undefined {
  const lower = name.toLowerCase()
  if (FILENAME_TO_LANG[lower]) return FILENAME_TO_LANG[lower]
  const ext = lower.includes('.') ? lower.split('.').pop()! : undefined
  return ext ? EXT_TO_LANG[ext] : undefined
}

type TokenLine = { content: string; color?: string }[]

type Highlighter = Awaited<ReturnType<typeof import('shiki').createHighlighter>>
let highlighterPromise: Promise<Highlighter> | null = null
const loadedLangs = new Set<string>()

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = import('shiki').then((m) =>
      m.createHighlighter({
        themes: ['vitesse-dark'],
        langs: [],
        engine: m.createJavaScriptRegexEngine()
      })
    )
  }
  return highlighterPromise
}

async function highlightCode(code: string, lang: string): Promise<TokenLine[]> {
  const hl = await getHighlighter()
  if (!loadedLangs.has(lang)) {
    try {
      await hl.loadLanguage(lang as Parameters<typeof hl.loadLanguage>[0])
      loadedLangs.add(lang)
    } catch {
      return []
    }
  }
  const result = hl.codeToTokens(code, {
    lang: lang as Parameters<typeof hl.codeToTokens>[1]['lang'],
    theme: 'vitesse-dark'
  })
  return result.tokens.map((line) => line.map((t) => ({ content: t.content, color: t.color })))
}

function useHighlightedLines(text: string, fileName: string): TokenLine[] | null {
  const [result, setResult] = useState<{ key: string; tokens: TokenLine[] } | null>(null)
  const lang = getLang(fileName)
  const key = `${fileName}\0${text.length}`

  useEffect(() => {
    if (!lang) return

    let stale = false
    highlightCode(text, lang)
      .then((tokens) => {
        if (stale) return
        setResult(tokens.length > 0 ? { key, tokens } : null)
      })
      .catch(() => {
        if (!stale) setResult(null)
      })

    return () => {
      stale = true
    }
  }, [text, lang, key])

  if (!lang || !result || result.key !== key) return null
  return result.tokens
}

// ---------------------------------------------------------------------------
// Find-in-file
// ---------------------------------------------------------------------------
type FindMatch = { line: number; start: number; end: number }

function computeMatches(lines: string[], query: string): FindMatch[] {
  if (!query) return []
  const lc = query.toLowerCase()
  const out: FindMatch[] = []
  for (let i = 0; i < lines.length; i++) {
    const lower = lines[i].toLowerCase()
    let from = 0
    while (from <= lower.length - lc.length) {
      const idx = lower.indexOf(lc, from)
      if (idx < 0) break
      out.push({ line: i, start: idx, end: idx + lc.length })
      from = idx + lc.length
    }
  }
  return out
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

// Render a line of plain text with `<mark>` overlays at the given match ranges.
function renderLineWithMarks(
  line: string,
  marks: { start: number; end: number; active: boolean }[]
): JSX.Element[] {
  if (marks.length === 0) return [<span key="t">{line || ' '}</span>]
  const out: JSX.Element[] = []
  let cursor = 0
  marks.forEach((m, i) => {
    if (m.start > cursor) out.push(<span key={`p${i}`}>{line.slice(cursor, m.start)}</span>)
    out.push(
      <span
        key={`m${i}`}
        className={
          m.active
            ? 'bg-amber-300/70 text-black rounded-[1px]'
            : 'bg-amber-300/25 text-gray-100 rounded-[1px]'
        }
      >
        {line.slice(m.start, m.end)}
      </span>
    )
    cursor = m.end
  })
  if (cursor < line.length) out.push(<span key="tail">{line.slice(cursor)}</span>)
  return out
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
  const highlighted = useHighlightedLines(visibleText, fileName)

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
const EDIT_LINE_HEIGHT = 21 // px, shared by the gutter, the drawn text and the textarea

/** A transparent textarea over the same text drawn in colour; a line uses its tokens only while they spell what was typed. */
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
  const lines = useMemo(() => draft.split('\n'), [draft])
  const highlighted = useHighlightedLines(lines.length > MAX_PREVIEW_LINES ? '' : draft, fileName)
  const gutter = useMemo(
    () => Array.from({ length: lines.length }, (_, i) => i + 1).join('\n'),
    [lines.length]
  )

  const matches = useMemo(() => computeMatches(lines, findQuery), [lines, findQuery])
  useEffect(() => {
    onMatchesComputed(matches.length)
  }, [matches.length, onMatchesComputed])

  const activeMatch = matches.length > 0 ? matches[activeMatchIdx % matches.length] : null
  const rowRefs = useRef(new Map<number, HTMLDivElement>())
  useEffect(() => {
    if (!activeMatch) return
    rowRefs.current.get(activeMatch.line)?.scrollIntoView?.({ block: 'center' })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- scroll when the match moves, not on every keystroke
  }, [activeMatch?.line, activeMatch?.start, activeMatchIdx])

  const rendered = useMemo<JSX.Element[]>(() => {
    const byLine = new Map<number, FindMatch[]>()
    for (const m of matches) byLine.set(m.line, [...(byLine.get(m.line) ?? []), m])
    return lines.map((line, i) => {
      const marks = byLine.get(i)
      const tokens = highlighted?.[i]
      const inStep = tokens !== undefined && tokens.map((t) => t.content).join('') === line
      return (
        <div
          key={i}
          ref={(el) => {
            if (el) rowRefs.current.set(i, el)
            else rowRefs.current.delete(i)
          }}
          style={{ height: EDIT_LINE_HEIGHT }}
        >
          {marks
            ? renderLineWithMarks(
                line,
                marks.map((m) => ({ start: m.start, end: m.end, active: m === activeMatch }))
              )
            : inStep && tokens.length > 0
              ? tokens.map((t, j) => (
                  <span key={j} style={t.color ? { color: t.color } : undefined}>
                    {t.content}
                  </span>
                ))
              : line || ' '}
        </div>
      )
    })
  }, [lines, highlighted, matches, activeMatch])

  const text = 'text-[13px] font-mono whitespace-pre py-1 pr-3'
  return (
    <div className="flex-1 overflow-auto">
      <div className="flex w-max min-w-full min-h-full">
        <pre
          className="sticky left-0 z-10 select-none text-right pr-3 pl-2 py-1 text-[12px] font-mono text-gray-600 shrink-0"
          style={{ lineHeight: `${EDIT_LINE_HEIGHT}px`, background: PANE_SURFACE }}
          aria-hidden="true"
        >
          {gutter}
        </pre>
        <div className="relative flex-1">
          <pre
            className={`${text} text-gray-300 pointer-events-none`}
            style={{ lineHeight: `${EDIT_LINE_HEIGHT}px` }}
            aria-hidden="true"
            data-testid="editor-highlight"
          >
            {rendered}
          </pre>
          <textarea
            value={draft}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault()
                // The window also listens for this chord, and would change the view.
                e.stopPropagation()
                onSaveShortcut()
              }
            }}
            spellCheck={false}
            wrap="off"
            data-file-editor="true"
            aria-label={`Edit ${fileName}`}
            className={`${text} absolute inset-0 w-full h-full bg-transparent text-transparent outline-none resize-none overflow-hidden`}
            style={{ lineHeight: `${EDIT_LINE_HEIGHT}px`, caretColor: 'var(--color-ink)' }}
          />
        </div>
      </div>
    </div>
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
  onHeaderDoubleClick,
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
  onHeaderDoubleClick?: () => void
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
  const canFind = content !== null && !loading && readOnly !== 'binary'

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
        onDoubleClick={onHeaderDoubleClick}
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
export function FileEditorPane({
  cwd,
  filePath,
  remoteHostId,
  dirtyRef: externalDirtyRef,
  draftKey,
  controls,
  onHeaderPointerDown,
  onHeaderDoubleClick,
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
  onHeaderDoubleClick?: () => void
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
      onHeaderDoubleClick={onHeaderDoubleClick}
      headerTestId={headerTestId}
      headerClassName={headerClassName}
    />
  )
}
