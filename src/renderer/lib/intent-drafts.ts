const STORAGE_KEY = 'vorn:intentDrafts'

/** Typed into a pane's composer and not yet sent. Kept the way editor drafts are. */
export interface IntentDraft {
  text: string
  savedAt: number
}

type Drafts = Record<string, IntentDraft>

function load(): Drafts {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Drafts = {}
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      const draft = value as Partial<IntentDraft>
      if (typeof draft?.text !== 'string' || !draft.text) continue
      out[id] = { text: draft.text, savedAt: typeof draft.savedAt === 'number' ? draft.savedAt : 0 }
    }
    return out
  } catch {
    return {}
  }
}

function save(drafts: Drafts): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(drafts))
  } catch {
    // Storage full: the text is still in the composer, only its survival is lost.
  }
}

export function readIntentDraft(sessionId: string): IntentDraft | null {
  return load()[sessionId] ?? null
}

export function writeIntentDraft(sessionId: string, text: string): void {
  if (!text) return forgetIntentDraft(sessionId)
  const drafts = load()
  drafts[sessionId] = { text, savedAt: Date.now() }
  save(drafts)
}

export function forgetIntentDraft(sessionId: string): void {
  const drafts = load()
  if (!(sessionId in drafts)) return
  delete drafts[sessionId]
  save(drafts)
}

/** Drop drafts for sessions that are gone, alongside their panes. */
export function pruneIntentDrafts(liveSessionIds: Set<string>): void {
  const drafts = load()
  const dead = Object.keys(drafts).filter((id) => !liveSessionIds.has(id))
  if (!dead.length) return
  for (const id of dead) delete drafts[id]
  save(drafts)
}
