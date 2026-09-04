import { useState, useEffect, useCallback, useMemo, useRef, type JSX, type ReactNode } from 'react'
import type { FileStamp } from '../../shared/types'
import { forgetDraft, hasMoved, readDraft, writeDraft } from '../lib/editor-drafts'

/** How long typing has to stop before the draft is worth a synchronous write. */
const DRAFT_SETTLE_MS = 400
import type { FileEntry } from '../../shared/types'
import { ChevronRight, Loader2, X, Search, Pencil, Save, SquareArrowOutUpRight } from 'lucide-react'
import { FileTypeIcon } from './file-icons'
import { PANE_SURFACE } from '../lib/pane-surface'
import { SplitDivider } from './SplitDivider'
import { clampSplitRatio, DEFAULT_SPLIT_RATIO } from '../lib/split-ratio'

const MAX_PREVIEW_LINES = 2000
const ROW_HEIGHT = 22 // px — matches VS Code's tree item height
const INDENT_WIDTH = 16 // px per depth level
const BASE_LEFT = 8 // px left gutter
const SPLIT_RATIO_KEY = 'vorn:files-split-ratio'

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
          // Hover-revealed here, unlike the pane's own controls. A tree is
          // hundreds of rows deep: a control drawn at rest on every one of them
          // is a column of arrows down the whole panel, and the thing being
          // read — the filenames — has to compete with it. The editor pane
          // carries a control that is always there, for the file you have open.
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
function EditView({
  draft,
  onChange,
  onSaveShortcut
}: {
  draft: string
  onChange: (next: string) => void
  onSaveShortcut: () => void
}) {
  const lineCount = useMemo(() => draft.split('\n').length, [draft])
  const gutter = useMemo(
    () => Array.from({ length: lineCount }, (_, i) => i + 1).join('\n'),
    [lineCount]
  )

  return (
    <div className="flex-1 overflow-auto flex">
      <pre
        className="select-none text-right pr-3 pl-2 py-1 text-[12px] leading-[1.65] font-mono text-gray-600 shrink-0"
        aria-hidden="true"
      >
        {gutter}
      </pre>
      <textarea
        value={draft}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === 's') {
            e.preventDefault()
            onSaveShortcut()
          }
        }}
        spellCheck={false}
        className="flex-1 bg-transparent text-gray-200 text-[13px] leading-[1.65] font-mono outline-none resize-none whitespace-pre py-1 pr-3"
        style={{ minHeight: '100%' }}
      />
    </div>
  )
}

// ---------------------------------------------------------------------------
// File panel
// ---------------------------------------------------------------------------
function FilePanel({
  cwd,
  filePath,
  content,
  loading,
  isBinary,
  onClose,
  onContentSaved,
  remoteHostId,
  dirtyRef,
  draftKey,
  showHeader = true,
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
  isBinary: boolean
  onClose: () => void
  onContentSaved: (next: string) => void
  remoteHostId?: string
  dirtyRef: React.MutableRefObject<boolean>
  /**
   * Where an unsaved edit is kept, so a quit does not throw it away.
   *
   * Keyed the way dirtiness already is — by pane, not by path — because two
   * panes open on one file are two editors, and one draft between them would
   * put a keystroke in either into both.
   *
   * Absent for a panel with no pane of its own to be keyed by, which keeps
   * today's behaviour of losing the edit.
   */
  draftKey?: string
  /** False when hosted in a pane card that draws its own header. */
  showHeader?: boolean
  /**
   * Pane chrome (maximize / close) seated in the path strip. A hosting card
   * that passes this drops its own header row: the path strip already names
   * the file, and two stacked bars read as chrome on chrome.
   */
  controls?: ReactNode
  onHeaderPointerDown?: (e: React.PointerEvent) => void
  onHeaderDoubleClick?: () => void
  headerTestId?: string
  headerClassName?: string
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  /**
   * What the file was when this edit started; the save compares against it.
   *
   * State rather than a ref, because it decides what gets written down. The
   * stamp arrives over a round trip -- seconds of it, for a file on a remote
   * host -- and a ref changing does not re-run the effect that persists the
   * draft. So a draft saved before the stamp landed kept `base: null` for good,
   * and a relaunch restored it with the guard unarmed: the next save would go
   * over a file that had changed underneath, without asking. That is the one
   * outcome all of this exists to prevent.
   */
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

  // Reset transient state when file changes
  useEffect(() => {
    pathRef.current = filePath
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: clear per-file edit/find state when the file changes
    setEditing(false)
    setDraft('')
    setSaveError(null)
    setConflict(false)
    setBase(null)
    setFindOpen(false)
    setFindQuery('')
    setFindIdx(0)
  }, [filePath])

  /**
   * Pick an edit back up where it was left.
   *
   * After the file has loaded, so the draft is compared against something. The
   * conflict is decided here rather than at save time for the restored case:
   * the file can have moved while the app was closed, and finding that out only
   * once somebody presses save means they have been editing against a screen
   * that was already wrong.
   */
  useEffect(() => {
    if (!draftKey || loading || content === null) return
    const draft = readDraft(draftKey, filePath)
    if (!draft) return
    // Saved elsewhere in the meantime, and the text now agrees with the file.
    // Nothing to restore, and leaving the record would reopen the editor on
    // every launch for an edit that has already landed.
    if (draft.text === content) {
      forgetDraft(draftKey)
      return
    }
    /* eslint-disable react-hooks/set-state-in-effect -- intentional: the draft is external state, read in once the file it belongs to has loaded */
    setBase(draft.base)
    setDraft(draft.text)
    setEditing(true)
    /* eslint-enable react-hooks/set-state-in-effect */
    // Guarded on the way back: this resolves after a round trip to the server,
    // by which time the pane can have been given another file or closed, and
    // the answer would then be about neither. A stamp that cannot be taken
    // leaves the guard unarmed rather than declaring a conflict.
    let stale = false
    window.api
      .fileStamp?.(filePath, remoteHostId)
      .then((current) => {
        if (!stale) setConflict(hasMoved(draft.base, current ?? null))
      })
      .catch(() => {
        if (!stale) setConflict(false)
      })
    return () => {
      stale = true
    }
  }, [draftKey, filePath, remoteHostId, loading, content])

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

  const dirty = editing && content !== null && draft !== content
  const canEdit = !isBinary && content !== null && !loading
  const canFind = canEdit && !editing

  useEffect(() => {
    dirtyRef.current = dirty
    return () => {
      dirtyRef.current = false
    }
  }, [dirty, dirtyRef])

  const handleStartEdit = (): void => {
    if (!canEdit || content === null) return
    setDraft(content)
    setEditing(true)
    setSaveError(null)
    setConflict(false)
    // What this edit is based on, asked for now rather than at save time: by
    // then the file may already have moved, and stamping it there would record
    // somebody else's version as the one being edited.
    setBase(null)
    // Against the path this edit started on, because the answer can arrive after
    // the pane has moved to another file -- and recording that file's stamp as
    // this edit's base is worse than having none, since the guard would then be
    // armed with the wrong version. A failure leaves it unarmed, which is the
    // documented meaning of no base.
    const editing = filePath
    window.api
      .fileStamp?.(filePath, remoteHostId)
      .then((stamp) => {
        if (pathRef.current === editing) setBase(stamp ?? null)
      })
      .catch(() => {})
  }

  const handleCancelEdit = (): void => {
    if (dirty && !window.confirm('Discard unsaved changes?')) return
    if (draftKey) forgetDraft(draftKey)
    setEditing(false)
    setDraft('')
    setSaveError(null)
    setConflict(false)
    setBase(null)
  }

  /**
   * Keep the edit, so closing the window is not the same as discarding it.
   *
   * Written on a delay: this fires per keystroke, and `localStorage.setItem` is
   * synchronous and on the same thread as the typing.
   */
  useEffect(() => {
    if (!draftKey || !editing || content === null) return
    if (draft === content) {
      forgetDraft(draftKey)
      return
    }
    const timer = setTimeout(
      () => writeDraft(draftKey, { filePath, text: draft, base }),
      DRAFT_SETTLE_MS
    )
    return () => clearTimeout(timer)
    // `base` included on purpose: a stamp that lands after the first write has
    // to reach the record, or the draft outlives the window with no base.
  }, [draftKey, editing, draft, content, filePath, base])

  /**
   * Write the draft to disk.
   *
   * `force` skips the check, and is only ever passed by the person answering the
   * conflict. The write itself is last-writer-wins, so without asking first this
   * silently discards whatever the file gained while the draft was open --
   * which, now that a draft outlives the window, can be a day's work by an
   * agent that was running the whole time.
   */
  const handleSave = useCallback(
    async (force = false): Promise<void> => {
      if (!editing) return
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
        setEditing(false)
        setConflict(false)
        setBase(null)
      } catch (err) {
        setSaveError(err instanceof Error ? err.message : String(err))
      } finally {
        setSaving(false)
      }
    },
    [filePath, editing, draft, remoteHostId, onContentSaved, draftKey, base]
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
    setEditing(false)
    setBase(null)
  }, [filePath, remoteHostId, onContentSaved, draftKey])

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
      {showHeader && <PanelHeader title="File" onClose={onClose} />}

      {/* Path strip + toolbar — doubles as the pane's title bar when the
          hosting card goes headerless. */}
      <div
        className={`flex items-center gap-2 px-2 py-1 text-[11px] font-mono shrink-0 ${headerClassName}`}
        style={{ background: PANE_SURFACE }}
        onPointerDown={onHeaderPointerDown}
        onDoubleClick={onHeaderDoubleClick}
        data-testid={headerTestId}
      >
        <FileTypeIcon name={fileName} size={12} />
        <span className="text-gray-400 flex-1 min-w-0 truncate" title={filePath} dir="rtl">
          {relPath}
        </span>
        {dirty && (
          <span
            className="w-[6px] h-[6px] rounded-full bg-amber-400 shrink-0"
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
        {editing ? (
          <>
            <ToolbarBtn
              icon={Save}
              label={saving ? 'Saving…' : 'Save (⌘S)'}
              disabled={!dirty || saving}
              onClick={() => void handleSave()}
            />
            <ToolbarBtn icon={X} label="Cancel edit" onClick={handleCancelEdit} />
          </>
        ) : (
          <ToolbarBtn icon={Pencil} label="Edit" disabled={!canEdit} onClick={handleStartEdit} />
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

      {/* Body */}
      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <Loader2 size={16} className="text-gray-500 animate-spin" />
        </div>
      ) : isBinary ? (
        <div className="flex-1 flex items-center justify-center text-gray-600 text-[13px]">
          Binary file — preview unavailable
        </div>
      ) : editing ? (
        <EditView draft={draft} onChange={setDraft} onSaveShortcut={() => void handleSave()} />
      ) : content !== null ? (
        <ReadView
          filePath={filePath}
          content={content}
          findQuery={findOpen ? findQuery : ''}
          activeMatchIdx={findIdx}
          onMatchesComputed={onMatchesComputed}
        />
      ) : null}

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
  active,
  disabled,
  onClick
}: {
  icon: typeof Search
  label: string
  active?: boolean
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className={`p-1 rounded transition-colors shrink-0 ${
        disabled
          ? 'text-gray-700 cursor-not-allowed'
          : active
            ? 'text-gray-100 bg-white/[0.08]'
            : 'text-gray-500 hover:text-gray-200 hover:bg-white/[0.05]'
      }`}
    >
      <Icon size={12} strokeWidth={2} />
    </button>
  )
}

function PanelHeader({ title, onClose }: { title: string; onClose?: () => void }) {
  return (
    <div
      className="flex items-center px-3 py-1.5 shrink-0 text-[12px]"
      style={{ background: PANE_SURFACE }}
    >
      <span className="flex-1 text-gray-300 font-medium">{title}</span>
      {onClose && (
        <button
          onClick={onClose}
          className="text-gray-600 hover:text-white p-0.5 rounded transition-colors"
          aria-label={`Close ${title}`}
        >
          <X size={12} strokeWidth={2} />
        </button>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Files panel (header + filter + tree)
// ---------------------------------------------------------------------------
function FilesPanel({
  rootEntries,
  dirCache,
  loadDir,
  selectedFile,
  onSelectFile,
  onPopOutFile,
  showHeader = true,
  headerTestId
}: {
  rootEntries: FileEntry[]
  dirCache: Map<string, FileEntry[]>
  loadDir: (path: string) => Promise<void>
  selectedFile: string | null
  onSelectFile: (path: string) => void
  /**
   * Open a file as a card of its own instead of in the session's editor.
   * Absent where there is no grid to put a card in, which is what hides the
   * per-row control rather than leaving it there doing nothing.
   */
  onPopOutFile?: (path: string) => void
  /** False when hosted in a pane card that draws its own header. */
  showHeader?: boolean
  headerTestId?: string
}) {
  const [filter, setFilter] = useState('')
  const { matched, expand } = useMemo(
    () => computeFilterSets(rootEntries, dirCache, filter),
    [rootEntries, dirCache, filter]
  )

  return (
    <div className="flex flex-col min-h-0 h-full">
      {showHeader && <PanelHeader title="Files" />}
      <div className="flex items-center gap-1 px-1.5 py-1.5 shrink-0" data-testid={headerTestId}>
        {/* A search field has to read as somewhere you can type before anything
            is in it; at 4% over a near-black pane it was very nearly the pane. */}
        <div className="flex items-center gap-1.5 flex-1 min-w-0 px-2 py-1 rounded-md bg-white/[0.09] focus-within:bg-white/[0.13] transition-colors">
          <Search size={13} className="text-gray-600 shrink-0" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') setFilter('')
            }}
            placeholder="Filter files…"
            className="flex-1 bg-transparent text-gray-200 outline-none text-[13px] placeholder:text-gray-600"
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
// Top-level orchestrator
// ---------------------------------------------------------------------------
export function FileTreeExplorer({ cwd, remoteHostId }: { cwd: string; remoteHostId?: string }) {
  const [rootEntries, setRootEntries] = useState<FileEntry[] | null>(null)
  const [loading, setLoading] = useState(true)
  const [dirCache, setDirCache] = useState(() => new Map<string, FileEntry[]>())
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [fileContent, setFileContent] = useState<string | null>(null)
  const [fileLoading, setFileLoading] = useState(false)
  const [isBinary, setIsBinary] = useState(false)
  const activeRequestRef = useRef<string | null>(null)
  const dirtyRef = useRef(false)

  const containerRef = useRef<HTMLDivElement | null>(null)
  const [splitRatio, setSplitRatio] = useState<number>(() => {
    if (typeof localStorage === 'undefined') return DEFAULT_SPLIT_RATIO
    const stored = localStorage.getItem(SPLIT_RATIO_KEY)
    const n = stored ? Number(stored) : NaN
    if (!Number.isFinite(n)) return DEFAULT_SPLIT_RATIO
    return clampSplitRatio(n)
  })

  const persistRatio = useCallback((next: number): void => {
    try {
      localStorage.setItem(SPLIT_RATIO_KEY, String(next))
    } catch {
      /* ignore quota errors */
    }
  }, [])

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

  const handleSelectFile = useCallback(
    async (filePath: string) => {
      if (filePath === activeRequestRef.current) return
      if (dirtyRef.current && !window.confirm('Discard unsaved changes?')) return
      activeRequestRef.current = filePath
      setSelectedFile(filePath)
      setFileContent(null)
      setIsBinary(false)
      setFileLoading(true)
      const content = await window.api.readFileContent(filePath, undefined, remoteHostId)
      if (activeRequestRef.current !== filePath) return
      if (content === null) {
        setIsBinary(true)
        setFileContent(null)
      } else {
        setIsBinary(false)
        setFileContent(content)
      }
      setFileLoading(false)
    },
    [remoteHostId]
  )

  const handleClosePreview = useCallback(() => {
    if (dirtyRef.current && !window.confirm('Discard unsaved changes?')) return
    activeRequestRef.current = null
    setSelectedFile(null)
    setFileContent(null)
    setIsBinary(false)
  }, [])

  const handleContentSaved = useCallback((next: string) => {
    setFileContent(next)
  }, [])

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

  const showFilePanel = selectedFile !== null

  return (
    <div ref={containerRef} className="flex-1 flex flex-col min-h-0">
      <div
        className="flex flex-col min-h-0"
        style={
          showFilePanel
            ? { flex: `${splitRatio} 1 0`, minHeight: 0 }
            : { flex: '1 1 0', minHeight: 0 }
        }
      >
        <FilesPanel
          rootEntries={rootEntries}
          dirCache={dirCache}
          loadDir={loadDir}
          selectedFile={selectedFile}
          onSelectFile={handleSelectFile}
        />
      </div>

      {showFilePanel && (
        <>
          <SplitDivider
            axis="y"
            label="Resize files / file panels"
            containerRef={containerRef}
            onRatioChange={setSplitRatio}
            onRatioCommit={persistRatio}
          />
          <div
            className="flex flex-col min-h-0"
            style={{ flex: `${1 - splitRatio} 1 0`, minHeight: 0 }}
          >
            <FilePanel
              cwd={cwd}
              filePath={selectedFile}
              content={fileContent}
              loading={fileLoading}
              isBinary={isBinary}
              onClose={handleClosePreview}
              onContentSaved={handleContentSaved}
              remoteHostId={remoteHostId}
              dirtyRef={dirtyRef}
            />
          </div>
        </>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Standalone panes
//
// `FileTreeExplorer` above stacks the tree and the editor in one component with
// an internal divider. When each lives in its own grid pane, the grid provides
// the split instead, so these two exports own their state independently — a
// session can have either open, maximized, or closed without the other.
// ---------------------------------------------------------------------------

/** The file tree on its own. Selecting a file is reported via `onSelectFile`. */
export function FileTreePane({
  cwd,
  remoteHostId,
  selectedFile,
  onSelectFile,
  onPopOutFile,
  headerTestId
}: {
  cwd: string
  remoteHostId?: string
  selectedFile: string | null
  onSelectFile: (path: string) => void
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
      onPopOutFile={onPopOutFile}
      showHeader={false}
      headerTestId={headerTestId}
    />
  )
}

/**
 * The file editor on its own, owning the load of `filePath`. Independent of the
 * tree: it renders whatever path it is given, whether or not a tree is open.
 */
export function FileEditorPane({
  cwd,
  filePath,
  remoteHostId,
  onClose,
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
  onClose?: () => void
  /** Where an unsaved edit is kept; see `FilePanel`. */
  draftKey?: string
  /** Pane chrome seated in the path strip; see `FilePanel`. */
  controls?: ReactNode
  onHeaderPointerDown?: (e: React.PointerEvent) => void
  onHeaderDoubleClick?: () => void
  headerTestId?: string
  headerClassName?: string
  /**
   * Set while the buffer has unsaved edits. The hosting pane reads it to
   * confirm before swapping files or closing — in the split-pane layout those
   * actions are driven from the tree and the card header, not from here.
   */
  dirtyRef?: React.MutableRefObject<boolean>
}): JSX.Element {
  const [content, setContent] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [isBinary, setIsBinary] = useState(false)
  const activeRequestRef = useRef<string | null>(null)
  const localDirtyRef = useRef(false)
  const dirtyRef = externalDirtyRef ?? localDirtyRef

  useEffect(() => {
    let stale = false
    activeRequestRef.current = filePath
    // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: reset to a loading state when the file changes
    setLoading(true)
    setContent(null)
    setIsBinary(false)
    window.api
      .readFileContent(filePath, undefined, remoteHostId)
      .then((next) => {
        if (stale || activeRequestRef.current !== filePath) return
        if (next === null) {
          setIsBinary(true)
          setContent(null)
        } else {
          setIsBinary(false)
          setContent(next)
        }
        setLoading(false)
      })
      .catch(() => {
        if (!stale) setLoading(false)
      })
    return () => {
      stale = true
    }
  }, [filePath, remoteHostId])

  const handleContentSaved = useCallback((next: string) => {
    setContent(next)
  }, [])

  return (
    <FilePanel
      cwd={cwd}
      filePath={filePath}
      content={content}
      loading={loading}
      isBinary={isBinary}
      onClose={onClose ?? (() => {})}
      onContentSaved={handleContentSaved}
      remoteHostId={remoteHostId}
      dirtyRef={dirtyRef}
      draftKey={draftKey}
      showHeader={false}
      controls={controls}
      onHeaderPointerDown={onHeaderPointerDown}
      onHeaderDoubleClick={onHeaderDoubleClick}
      headerTestId={headerTestId}
      headerClassName={headerClassName}
    />
  )
}
