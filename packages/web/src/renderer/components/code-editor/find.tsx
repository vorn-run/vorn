import type { JSX } from 'react'

/** One hit of a find query: a line, and the characters on it that matched. */
export type FindMatch = { line: number; start: number; end: number }

/** Every case-insensitive, non-overlapping hit of `query`, in reading order. */
export function computeMatches(lines: string[], query: string): FindMatch[] {
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

// Render a line of plain text with `<mark>` overlays at the given match ranges.
export function renderLineWithMarks(
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
