export interface DocEdit {
  before: string
  after: string
}

/** A doc's paragraphs: the blocks between blank lines. */
export function paragraphs(md: string): string[] {
  return md
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean)
}

/** A paragraph's words, so the editor rewriting `*` as `-` or rewrapping a line is not an edit. */
function key(paragraph: string): string {
  return paragraph
    .replace(/[*_`#>~\\-]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}

// Untouched paragraphs keep the source's Markdown; each run of changed ones is one edit.
export function mergeDocEdit(original: string, edited: string): { body: string; edits: DocEdit[] } {
  const a = paragraphs(original)
  const b = paragraphs(edited)
  const ka = a.map(key)
  const kb = b.map(key)
  // Longest common subsequence over paragraph keys, filled from the end.
  const lcs = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0))
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i][j] = ka[i] === kb[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1])
    }
  }

  const out: string[] = []
  const edits: DocEdit[] = []
  let removed: string[] = []
  let added: string[] = []
  const flush = (): void => {
    if (removed.length || added.length) {
      edits.push({ before: removed.join('\n\n'), after: added.join('\n\n') })
    }
    removed = []
    added = []
  }
  let i = 0
  let j = 0
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && ka[i] === kb[j]) {
      flush()
      out.push(a[i])
      i++
      j++
    } else if (j < b.length && (i === a.length || lcs[i][j + 1] >= lcs[i + 1][j])) {
      added.push(b[j])
      out.push(b[j])
      j++
    } else {
      removed.push(a[i])
      i++
    }
  }
  flush()
  return { body: out.join('\n\n') + '\n', edits }
}
