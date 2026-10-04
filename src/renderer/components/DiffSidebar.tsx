/* eslint-disable react-refresh/only-export-components */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { GitFileDiff } from '../../shared/types'
import { X, MessageSquare } from 'lucide-react'
import { FileTypeIcon } from './file-icons'

/**
 * The letter already says which. Four hues for four categories is the pattern
 * the rest of this pass removed — the diff body below keeps its green and red,
 * because that is the work itself rather than a label describing it.
 */
const STATUS_LETTER: Record<string, string> = {
  modified: 'M',
  added: 'A',
  deleted: 'D',
  renamed: 'R'
}

/** One tone for all four: the letter says which, the colour only says "file". */
const STATUS_LETTER_CLASS = 'text-ink-secondary'

export interface DiffComment {
  filePath: string
  lineIndex: number
  lineContent: string
  comment: string
}

export function DiffFileList({
  files,
  selectedFile,
  onSelectFile
}: {
  files: GitFileDiff[]
  selectedFile: string | null
  onSelectFile: (path: string) => void
}) {
  return (
    <div className="border-b border-white/[0.06] max-h-[200px] overflow-y-auto">
      {files.map((file) => {
        const letter = STATUS_LETTER[file.status] ?? STATUS_LETTER.modified
        const isSelected = selectedFile === file.filePath
        const fileName = file.filePath.split('/').pop() || file.filePath
        return (
          <button
            key={file.filePath}
            onClick={() => onSelectFile(file.filePath)}
            className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-[12px] transition-colors
                       ${isSelected ? 'bg-white/[0.08]' : 'hover:bg-white/[0.04]'}`}
          >
            <FileTypeIcon name={fileName} size={15} />
            <span className="flex-1 min-w-0 truncate text-gray-300 font-mono">{file.filePath}</span>
            <span className="shrink-0 flex items-center gap-1.5 text-[11px] font-mono">
              {file.insertions > 0 && <span className="text-diff-add">+{file.insertions}</span>}
              {file.deletions > 0 && <span className="text-diff-remove">-{file.deletions}</span>}
            </span>
            <span className={`shrink-0 text-[10px] font-bold ${STATUS_LETTER_CLASS}`}>
              {letter}
            </span>
          </button>
        )
      })}
    </div>
  )
}

function InlineCommentInput({
  onSubmit,
  onCancel
}: {
  onSubmit: (text: string) => void
  onCancel: () => void
}) {
  const [text, setText] = useState('')
  const inputRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      if (text.trim()) onSubmit(text.trim())
    } else if (e.key === 'Escape') {
      onCancel()
    }
  }

  return (
    <div className="mx-2 my-1 bg-white/[0.04] border border-white/[0.10] rounded-md p-2">
      <textarea
        ref={inputRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Add review comment..."
        rows={2}
        className="w-full px-2 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded text-xs
                   text-gray-200 placeholder-gray-600 focus:outline-none focus:border-white/[0.20]
                   resize-none font-mono"
      />
      <div className="flex items-center justify-between mt-1.5">
        <span className="text-[10px] text-gray-600">Cmd+Enter to submit</span>
        <div className="flex gap-1.5">
          <button
            onClick={onCancel}
            className="px-2 py-1 text-[10px] text-gray-500 hover:text-gray-300 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={() => text.trim() && onSubmit(text.trim())}
            disabled={!text.trim()}
            className="px-2 py-1 text-[10px] font-medium text-ink hover:text-white
                       disabled:opacity-30 transition-colors"
          >
            Comment
          </button>
        </div>
      </div>
    </div>
  )
}

function CommentBadge({ comment, onRemove }: { comment: DiffComment; onRemove: () => void }) {
  return (
    <div className="mx-2 my-0.5 bg-white/[0.03] border border-white/[0.08] rounded-md px-3 py-1.5 flex items-start gap-2">
      <MessageSquare size={11} className="text-ink-secondary mt-0.5 shrink-0" />
      <span className="text-xs text-ink-secondary flex-1">{comment.comment}</span>
      <button
        onClick={onRemove}
        className="text-gray-600 hover:text-danger p-0.5 shrink-0 transition-colors"
      >
        <X size={10} strokeWidth={2} />
      </button>
    </div>
  )
}

/** One line of a file's diff as it is drawn; `index` is its line in the raw diff, which comments are keyed by. */
export interface DiffRow {
  kind: 'meta' | 'hunk' | 'add' | 'del' | 'ctx'
  index: number
  text: string
  oldLine?: number
  newLine?: number
}

/** The rows a diff draws: header lines dropped, hunks numbered, blank metadata skipped. */
export function parseDiffRows(diff: string): DiffRow[] {
  const rows: DiffRow[] = []
  let oldLine = 0
  let newLine = 0
  let inHunk = false
  diff.split('\n').forEach((line, index) => {
    if (
      line.startsWith('diff --git') ||
      line.startsWith('index ') ||
      line.startsWith('--- ') ||
      line.startsWith('+++ ')
    ) {
      return
    }
    const hunk = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)/)
    if (hunk) {
      oldLine = parseInt(hunk[1], 10)
      newLine = parseInt(hunk[2], 10)
      inHunk = true
      rows.push({ kind: 'hunk', index, text: line })
      return
    }
    if (!inHunk) {
      // Binary diff or other metadata
      if (line.trim()) rows.push({ kind: 'meta', index, text: line })
      return
    }
    if (line.startsWith('+')) rows.push({ kind: 'add', index, text: line, newLine: newLine++ })
    else if (line.startsWith('-')) rows.push({ kind: 'del', index, text: line, oldLine: oldLine++ })
    else if (line.startsWith(' '))
      rows.push({ kind: 'ctx', index, text: line, oldLine: oldLine++, newLine: newLine++ })
  })
  return rows
}

/**
 * Rows drawn or skipped together. A diff of a few thousand lines is tens of
 * thousands of elements, and React building all of them is the cost of opening
 * the panel; so only blocks near the visible part of the list are drawn, and
 * the rest stand in as empty boxes of the same height.
 */
export const DIFF_BLOCK_ROWS = 100
/** How far beyond the visible part blocks are drawn, so scrolling never meets an empty box. */
const DIFF_DRAW_MARGIN_PX = 1200
/** A row is one line of 12px text at 1.6 leading; a hunk header adds 2px of padding above and below. */
const DIFF_ROW_PX = 19.2
const DIFF_HUNK_EXTRA_PX = 4

/** Marks the scrolling element blocks measure themselves against. */
const SCROLL_ROOT_ATTR = 'data-diff-scroll'

/** What one line's comments are, by their index in the full list, so a removal names the right one. */
type LineComments = Map<number, { comment: DiffComment; globalIdx: number }[]>

interface RowHandlers {
  onClickLine: (filePath: string, lineIndex: number, lineContent: string) => void
  onAddComment: (text: string) => void
  onCancelComment: () => void
  onRemoveComment: (index: number) => void
}

export function DiffContent({
  files,
  selectedFile,
  comments,
  commentingLine,
  onClickLine,
  onAddComment,
  onCancelComment,
  onRemoveComment
}: {
  files: GitFileDiff[]
  selectedFile: string | null
  comments: DiffComment[]
  commentingLine: { filePath: string; lineIndex: number } | null
  onClickLine: (filePath: string, lineIndex: number, lineContent: string) => void
  onAddComment: (text: string) => void
  onCancelComment: () => void
  onRemoveComment: (index: number) => void
}) {
  const fileRefs = useRef<Map<string, HTMLDivElement>>(new Map())

  useEffect(() => {
    if (selectedFile) {
      const el = fileRefs.current.get(selectedFile)
      if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [selectedFile])

  const byFile = useMemo(() => {
    const out = new Map<string, LineComments>()
    comments.forEach((comment, globalIdx) => {
      let lines = out.get(comment.filePath)
      if (!lines) out.set(comment.filePath, (lines = new Map()))
      const at = lines.get(comment.lineIndex) ?? []
      at.push({ comment, globalIdx })
      lines.set(comment.lineIndex, at)
    })
    return out
  }, [comments])

  const handlers: RowHandlers = { onClickLine, onAddComment, onCancelComment, onRemoveComment }

  return (
    <div {...{ [SCROLL_ROOT_ATTR]: '' }} className="flex-1 overflow-y-auto">
      {files.map((file) => (
        <DiffFile
          key={file.filePath}
          file={file}
          lineComments={byFile.get(file.filePath)}
          commentingIndex={
            commentingLine?.filePath === file.filePath ? commentingLine.lineIndex : null
          }
          handlers={handlers}
          fileRef={(el) => {
            if (el) fileRefs.current.set(file.filePath, el)
          }}
        />
      ))}
    </div>
  )
}

/** A short identity for a diff's text, so a changed diff remounts its blocks. */
function diffIdentity(diff: string): string {
  let hash = 0
  for (let i = 0; i < diff.length; i++) hash = (Math.imul(hash, 31) + diff.charCodeAt(i)) | 0
  return `${diff.length}-${(hash >>> 0).toString(36)}`
}

function DiffFile({
  file,
  lineComments,
  commentingIndex,
  handlers,
  fileRef
}: {
  file: GitFileDiff
  lineComments: LineComments | undefined
  commentingIndex: number | null
  handlers: RowHandlers
  fileRef: (el: HTMLDivElement | null) => void
}) {
  const letter = STATUS_LETTER[file.status] ?? STATUS_LETTER.modified
  const fileName = file.filePath.split('/').pop() || file.filePath
  const rows = useMemo(() => parseDiffRows(file.diff), [file.diff])
  const diffKey = useMemo(() => diffIdentity(file.diff), [file.diff])
  const blocks = useMemo(() => {
    const out: DiffRow[][] = []
    for (let i = 0; i < rows.length; i += DIFF_BLOCK_ROWS)
      out.push(rows.slice(i, i + DIFF_BLOCK_ROWS))
    return out
  }, [rows])
  let fileCommentCount = 0
  lineComments?.forEach((at) => (fileCommentCount += at.length))

  return (
    <div ref={fileRef}>
      {/* File header */}
      <div
        className="sticky top-0 z-10 flex items-center gap-2 px-3 py-1.5 text-[12px] font-mono
                            border-b border-white/[0.06]"
        style={{ background: 'var(--color-surface-overlay)' }}
      >
        <FileTypeIcon name={fileName} size={14} />
        <span className="text-gray-300 flex-1 min-w-0 truncate">{file.filePath}</span>
        <span className={`${STATUS_LETTER_CLASS} text-[10px] font-bold shrink-0`}>{letter}</span>
        {fileCommentCount > 0 && (
          <span className="text-[10px] text-ink-secondary bg-white/[0.06] px-1.5 py-0.5 rounded-full ml-auto">
            {fileCommentCount} comment{fileCommentCount !== 1 ? 's' : ''}
          </span>
        )}
      </div>

      {/* Diff lines */}
      <pre className="text-[12px] leading-[1.6] font-mono">
        {blocks.map((block, i) => (
          <DiffBlock
            // A new diff starts its blocks over: a kept placeholder height
            // would be the old block's.
            key={`${diffKey}:${i}`}
            rows={block}
            filePath={file.filePath}
            lineComments={lineComments}
            commentingIndex={commentingIndex}
            handlers={handlers}
          />
        ))}
      </pre>
    </div>
  )
}

/** The height a block takes before it has been drawn: rows have one height, hunk headers a little more. */
export function estimateBlockHeight(rows: readonly DiffRow[]): number {
  let px = rows.length * DIFF_ROW_PX
  for (const row of rows) if (row.kind === 'hunk') px += DIFF_HUNK_EXTRA_PX
  return px
}

function DiffBlock({
  rows,
  filePath,
  lineComments,
  commentingIndex,
  handlers
}: {
  rows: DiffRow[]
  filePath: string
  lineComments: LineComments | undefined
  commentingIndex: number | null
  handlers: RowHandlers
}) {
  const ref = useRef<HTMLDivElement>(null)
  // Drawn from the start where nothing can say what is visible, as in tests.
  const [near, setNear] = useState(() => typeof IntersectionObserver !== 'function')
  // Kept as it was last drawn, so the empty box that stands in for it is exactly as tall.
  const [drawnHeight, setDrawnHeight] = useState<number | null>(null)
  // A block holding a comment or the comment being written is always drawn, so
  // what someone is typing into is never unmounted under them.
  const pinned =
    commentingIndex !== null && rows.some((row) => row.index === commentingIndex)
      ? true
      : !!lineComments && rows.some((row) => lineComments.has(row.index))

  // Before the first paint, so a block on screen is drawn in the first frame
  // rather than one frame after an empty box.
  useLayoutEffect(() => {
    const el = ref.current
    const root = el?.closest(`[${SCROLL_ROOT_ATTR}]`)
    if (!el || !root || typeof IntersectionObserver !== 'function') return
    const box = el.getBoundingClientRect()
    const view = root.getBoundingClientRect()
    if (
      box.bottom >= view.top - DIFF_DRAW_MARGIN_PX &&
      box.top <= view.bottom + DIFF_DRAW_MARGIN_PX
    ) {
      setNear(true)
    }
  }, [])

  useEffect(() => {
    const el = ref.current
    if (!el || typeof IntersectionObserver !== 'function') return
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[entries.length - 1]
        if (!entry.isIntersecting && ref.current) {
          const drawn = ref.current.getBoundingClientRect().height
          if (drawn > 0) setDrawnHeight(drawn)
        }
        setNear(entry.isIntersecting)
      },
      { root: el.closest(`[${SCROLL_ROOT_ATTR}]`), rootMargin: `${DIFF_DRAW_MARGIN_PX}px 0px` }
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  if (!near && !pinned) {
    return <div ref={ref} style={{ height: drawnHeight ?? estimateBlockHeight(rows) }} />
  }
  return (
    <div ref={ref}>
      {rows.map((row) => (
        <DiffRowView
          key={row.index}
          row={row}
          filePath={filePath}
          comments={lineComments?.get(row.index)}
          commenting={commentingIndex === row.index}
          handlers={handlers}
        />
      ))}
    </div>
  )
}

function DiffRowView({
  row,
  filePath,
  comments,
  commenting,
  handlers
}: {
  row: DiffRow
  filePath: string
  comments: { comment: DiffComment; globalIdx: number }[] | undefined
  commenting: boolean
  handlers: RowHandlers
}) {
  const { onClickLine, onAddComment, onCancelComment, onRemoveComment } = handlers
  if (row.kind === 'hunk') {
    return (
      <div className="bg-white/[0.05] text-ink-secondary px-3 py-0.5 select-text">{row.text}</div>
    )
  }
  if (row.kind === 'meta') {
    return <div className="text-gray-500 px-3 select-text">{row.text}</div>
  }
  const commentIcon = (
    <span className="opacity-0 group-hover/line:opacity-100 pr-2 text-ink-secondary transition-opacity shrink-0">
      <MessageSquare size={11} strokeWidth={2} />
    </span>
  )
  let line: React.ReactNode
  if (row.kind === 'add') {
    line = (
      <div
        className="bg-green-500/10 flex select-text group/line cursor-pointer hover:bg-green-500/15"
        onClick={() => onClickLine(filePath, row.index, row.text)}
      >
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-gray-600 select-none">
          {' '}
        </span>
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-green-600 select-none">
          {row.newLine}
        </span>
        <span className="text-green-300 px-1 flex-1">{row.text.slice(1) || ' '}</span>
        {commentIcon}
      </div>
    )
  } else if (row.kind === 'del') {
    line = (
      <div
        className="bg-red-500/10 flex select-text group/line cursor-pointer hover:bg-red-500/15"
        onClick={() => onClickLine(filePath, row.index, row.text)}
      >
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-red-600 select-none">
          {row.oldLine}
        </span>
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-gray-600 select-none">
          {' '}
        </span>
        <span className="text-red-300 px-1 flex-1">{row.text.slice(1) || ' '}</span>
        {commentIcon}
      </div>
    )
  } else {
    line = (
      <div className="flex select-text">
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-gray-600 select-none">
          {row.oldLine}
        </span>
        <span className="w-[35px] shrink-0 text-right pr-2 text-[11px] text-gray-600 select-none">
          {row.newLine}
        </span>
        <span className="text-gray-400 px-1 flex-1">{row.text.slice(1) || ' '}</span>
      </div>
    )
  }
  if (!comments?.length && !commenting) return line
  return (
    <>
      {line}
      {comments?.map(({ comment, globalIdx }) => (
        <CommentBadge
          key={`comment-${row.index}-${globalIdx}`}
          comment={comment}
          onRemove={() => onRemoveComment(globalIdx)}
        />
      ))}
      {commenting && <InlineCommentInput onSubmit={onAddComment} onCancel={onCancelComment} />}
    </>
  )
}

export function formatReviewFeedback(comments: DiffComment[]): string {
  const grouped = new Map<string, DiffComment[]>()
  for (const c of comments) {
    if (!grouped.has(c.filePath)) grouped.set(c.filePath, [])
    grouped.get(c.filePath)!.push(c)
  }

  let feedback = 'Please address the following review comments:\n\n'
  for (const [file, fileComments] of grouped) {
    feedback += `**${file}:**\n`
    for (const c of fileComments) {
      const codeLine = c.lineContent.slice(1).trim()
      feedback += `- Line \`${codeLine}\`: ${c.comment}\n`
    }
    feedback += '\n'
  }
  return feedback
}
