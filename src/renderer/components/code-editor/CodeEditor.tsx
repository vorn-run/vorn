import { useEffect, useMemo, useRef, useState, type JSX, type KeyboardEvent } from 'react'
import { PANE_SURFACE } from '../../lib/pane-surface'
import { computeMatches, renderLineWithMarks, type FindMatch } from './find'
import { getLang } from './shiki'
import { useHighlightedLines } from './useHighlightedLines'

/** How long typing has to stop before the text is worth colouring again. */
const HIGHLIGHT_SETTLE_MS = 150
/** Past this many lines colour costs more than it gives, so the text is drawn plain. */
const MAX_HIGHLIGHT_LINES = 2000
export const EDIT_LINE_HEIGHT = 21 // px, shared by the gutter, the drawn text and the textarea

export type CodeEditorFind = {
  query: string
  activeIndex: number
  onMatchesComputed: (count: number) => void
}

export type CodeEditorProps = {
  value: string
  onChange: (next: string) => void
  /** A Shiki language id. Wins over the one `fileName` implies. */
  language?: string
  /** Picks the language from the extension when `language` is not given. */
  fileName?: string
  ariaLabel: string
  readOnly?: boolean
  /** A 1-based line to mark as wrong, in the gutter and across the row. */
  errorLine?: number
  onSaveShortcut?: () => void
  onKeyDown?: (e: KeyboardEvent<HTMLTextAreaElement>) => void
  find?: CodeEditorFind
  /** Classes for the scroller. Without any it grows to fill a flex column. */
  className?: string
  maxHeight?: number | string
  minHeight?: number | string
}

/**
 * A transparent textarea over the same text drawn in colour; a line uses its
 * tokens only while they spell what was typed.
 *
 * The browser keeps doing what it is good at — the caret, selection, undo, IME
 * — and the coloured copy underneath only has to line up with it, which the
 * shared line height and padding guarantee.
 */
export function CodeEditor({
  value,
  onChange,
  language,
  fileName,
  ariaLabel,
  readOnly = false,
  errorLine,
  onSaveShortcut,
  onKeyDown,
  find,
  className,
  maxHeight,
  minHeight
}: CodeEditorProps): JSX.Element {
  const lang = language ?? (fileName ? getLang(fileName) : undefined)
  const findQuery = find?.query ?? ''
  const activeMatchIdx = find?.activeIndex ?? 0
  const onMatchesComputed = find?.onMatchesComputed

  const lines = useMemo(() => value.split('\n'), [value])
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), HIGHLIGHT_SETTLE_MS)
    return () => clearTimeout(timer)
  }, [value])
  const highlighted = useHighlightedLines(
    lines.length > MAX_HIGHLIGHT_LINES ? '' : settled,
    lang,
    true
  )

  // An error line past the end is still worth showing: the text ended early.
  const markedLine =
    errorLine !== undefined && errorLine >= 1 ? Math.min(errorLine, lines.length) : undefined

  const gutter = useMemo(() => {
    const numbers = Array.from({ length: lines.length }, (_, i) => i + 1)
    if (markedLine === undefined) return numbers.join('\n')
    const before = numbers.slice(0, markedLine - 1).join('\n')
    const after = numbers.slice(markedLine).join('\n')
    return (
      <>
        {before && `${before}\n`}
        <span className="text-danger" data-testid="editor-error-gutter">
          {markedLine}
        </span>
        {after && `\n${after}`}
      </>
    )
  }, [lines.length, markedLine])

  const matches = useMemo(() => computeMatches(lines, findQuery), [lines, findQuery])
  useEffect(() => {
    onMatchesComputed?.(matches.length)
  }, [matches.length, onMatchesComputed])

  const activeMatch = matches.length > 0 ? matches[activeMatchIdx % matches.length] : null
  const scrollerRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = scrollerRef.current
    if (!activeMatch || !el) return
    el.scrollTop = activeMatch.line * EDIT_LINE_HEIGHT - el.clientHeight / 2
    // eslint-disable-next-line react-hooks/exhaustive-deps -- scroll when the match moves, not on every keystroke
  }, [activeMatch?.line, activeMatch?.start, activeMatchIdx])

  const rendered = useMemo<JSX.Element[]>(() => {
    const byLine = new Map<number, FindMatch[]>()
    for (const m of matches) byLine.set(m.line, [...(byLine.get(m.line) ?? []), m])
    return lines.map((line, i) => {
      const marks = byLine.get(i)
      const tokens = highlighted?.[i]
      const inStep = tokens !== undefined && tokens.map((t) => t.content).join('') === line
      const isError = markedLine === i + 1
      return (
        <div
          key={i}
          style={{ height: EDIT_LINE_HEIGHT }}
          className={isError ? 'bg-danger/10' : undefined}
          data-error-line={isError ? 'true' : undefined}
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
  }, [lines, highlighted, matches, activeMatch, markedLine])

  const text = 'text-[13px] font-mono whitespace-pre py-1 pr-3'
  return (
    <div
      ref={scrollerRef}
      className={`overflow-auto ${className ?? 'flex-1'}`}
      style={{ maxHeight, minHeight }}
    >
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
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if (onSaveShortcut && (e.metaKey || e.ctrlKey) && e.key === 's') {
                e.preventDefault()
                // The window also listens for this chord, and would change the view.
                e.stopPropagation()
                onSaveShortcut()
                return
              }
              onKeyDown?.(e)
            }}
            readOnly={readOnly}
            spellCheck={false}
            wrap="off"
            aria-label={ariaLabel}
            aria-invalid={markedLine !== undefined ? true : undefined}
            className={`${text} absolute inset-0 w-full h-full bg-transparent text-transparent outline-none resize-none overflow-hidden`}
            style={{ lineHeight: `${EDIT_LINE_HEIGHT}px`, caretColor: 'var(--color-ink)' }}
          />
        </div>
      </div>
    </div>
  )
}
