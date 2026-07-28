/**
 * Local history of inputs submitted through the intent bar.
 *
 * Entries are kept in localStorage (renderer-local, per machine). Shell
 * commands and agent prompts are tracked separately so a prompt typed into
 * a Claude session is never suggested inside a plain shell.
 */

export type HistoryKind = 'shell' | 'agent'

export interface CommandHistoryEntry {
  text: string
  kind: HistoryKind
  timestamp: number
  projectPath?: string
}

const STORAGE_KEY = 'vorn:commandHistory'
const MAX_ENTRIES = 1000
const MAX_RESULTS = 8

let cache: CommandHistoryEntry[] | null = null

function load(): CommandHistoryEntry[] {
  if (cache) return cache
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    const parsed = raw ? (JSON.parse(raw) as CommandHistoryEntry[]) : []
    cache = Array.isArray(parsed) ? parsed : []
  } catch {
    cache = []
  }
  return cache
}

function save(entries: CommandHistoryEntry[]): void {
  cache = entries
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries))
  } catch {
    /* quota or unavailable storage — history stays in-memory */
  }
}

/** Test helper / hot-reload safety: drop the in-memory cache. */
export function resetCommandHistoryCache(): void {
  cache = null
}

/**
 * Record a submitted input. Most recent first. Re-submitting an identical
 * text moves it to the front instead of duplicating it.
 */
export function recordCommand(text: string, kind: HistoryKind, projectPath?: string): void {
  const trimmed = text.trim()
  if (!trimmed) return
  const entries = load().filter((e) => !(e.text === trimmed && e.kind === kind))
  entries.unshift({ text: trimmed, kind, timestamp: Date.now(), projectPath })
  if (entries.length > MAX_ENTRIES) entries.length = MAX_ENTRIES
  save(entries)
}

/** True when `query` is a subsequence of `text` (cheap fuzzy match). */
function isSubsequence(query: string, text: string): boolean {
  let qi = 0
  for (let ti = 0; ti < text.length && qi < query.length; ti++) {
    if (text[ti] === query[qi]) qi++
  }
  return qi === query.length
}

/**
 * Rank history entries against the query. Prefix matches beat substring
 * matches beat subsequence matches; same-project entries and recent entries
 * rank higher within a tier. Empty query returns most recent entries.
 */
export function searchHistory(
  query: string,
  kind: HistoryKind,
  projectPath?: string
): CommandHistoryEntry[] {
  const entries = load().filter((e) => e.kind === kind)
  const q = query.trim().toLowerCase()

  if (!q) {
    return entries
      .slice()
      .sort((a, b) => {
        const ap = a.projectPath === projectPath ? 1 : 0
        const bp = b.projectPath === projectPath ? 1 : 0
        return bp - ap || b.timestamp - a.timestamp
      })
      .slice(0, MAX_RESULTS)
  }

  const scored: { entry: CommandHistoryEntry; score: number }[] = []
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i]
    const text = entry.text.toLowerCase()
    if (text === q) continue // exact match suggests nothing new
    let score: number
    if (text.startsWith(q)) score = 3000
    else if (text.includes(q)) score = 2000
    else if (isSubsequence(q, text)) score = 1000
    else continue
    if (entry.projectPath === projectPath) score += 500
    score -= i // entries are most-recent-first; older ranks lower
    scored.push({ entry, score })
  }
  scored.sort((a, b) => b.score - a.score)
  return scored.slice(0, MAX_RESULTS).map((s) => s.entry)
}

/**
 * Fish-style ghost suggestion: the most recent entry that extends the
 * current input. Same-project entries win over other projects.
 */
export function ghostSuggestion(
  input: string,
  kind: HistoryKind,
  projectPath?: string
): string | null {
  if (!input || input.includes('\n')) return null
  const entries = load().filter((e) => e.kind === kind)
  let fallback: string | null = null
  for (const entry of entries) {
    if (entry.text.length <= input.length || !entry.text.startsWith(input)) continue
    if (entry.projectPath === projectPath) return entry.text
    if (!fallback) fallback = entry.text
  }
  return fallback
}
