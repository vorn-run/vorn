import type { FileStamp } from '../../shared/types'

const STORAGE_KEY = 'vorn:drafts'

/**
 * An edit that was never saved, kept so a quit does not throw it away.
 *
 * The concept did not exist: the draft was a component's `useState`, so closing
 * the pane, reloading, or quitting lost it with nothing said. It is written down
 * here instead — per device, beside the panes, because a draft belongs to the
 * window somebody was typing in and not to the session.
 *
 * `base` is what the file was when the edit started. It is the whole reason this
 * is more than a string: a file that has moved on disk under an unsaved draft
 * cannot be saved over silently, and without a record of what the draft was
 * based on there is no way to know that it has.
 */
export interface EditorDraft {
  filePath: string
  text: string
  /** Null when the file could not be stamped at the time — treated as unknown, never as unchanged. */
  base: FileStamp | null
  savedAt: number
}

type Drafts = Record<string, EditorDraft>

function load(): Drafts {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Drafts = {}
    for (const [paneId, value] of Object.entries(parsed as Record<string, unknown>)) {
      const draft = value as Partial<EditorDraft>
      if (typeof draft?.filePath !== 'string' || !draft.filePath) continue
      if (typeof draft.text !== 'string') continue
      out[paneId] = {
        filePath: draft.filePath,
        text: draft.text,
        base: readStamp(draft.base),
        savedAt: typeof draft.savedAt === 'number' ? draft.savedAt : 0
      }
    }
    return out
  } catch {
    return {}
  }
}

function readStamp(value: unknown): FileStamp | null {
  const stamp = value as Partial<FileStamp> | null
  if (!stamp || typeof stamp !== 'object') return null
  if (typeof stamp.size !== 'number' || typeof stamp.mtimeMs !== 'number') return null
  return { size: stamp.size, mtimeMs: stamp.mtimeMs }
}

function save(drafts: Drafts): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(drafts))
  } catch {
    // A quota that is full, which for a draft means the edit is not kept. The
    // text is still in the editor; only its survival past this window is lost.
  }
}

/**
 * The draft this pane was holding, if it is still about the same file.
 *
 * Keyed by pane, not by path. Two panes open on one file are two editors, and
 * giving them one draft would have a keystroke in either appear in both. The
 * cost is that they can disagree, which is what the save-time check is for.
 */
export function readDraft(paneId: string, filePath: string): EditorDraft | null {
  const draft = load()[paneId]
  return draft && draft.filePath === filePath ? draft : null
}

export function writeDraft(paneId: string, draft: Omit<EditorDraft, 'savedAt'>): void {
  const drafts = load()
  drafts[paneId] = { ...draft, savedAt: Date.now() }
  save(drafts)
}

export function forgetDraft(paneId: string): void {
  const drafts = load()
  if (!(paneId in drafts)) return
  delete drafts[paneId]
  save(drafts)
}

/** Drop drafts for panes that are gone, alongside the panes themselves. */
export function pruneDrafts(livePaneIds: Set<string>): void {
  const drafts = load()
  const dead = Object.keys(drafts).filter((id) => !livePaneIds.has(id))
  if (!dead.length) return
  for (const id of dead) delete drafts[id]
  save(drafts)
}

/**
 * Whether the file has moved since the draft was based on it.
 *
 * No base means the guard never armed -- nothing stamped the file when the edit
 * started, so there is no version to have moved away from and the save behaves
 * as it did before any of this existed. Answering "moved" there would turn every
 * save into a conflict on a surface that cannot stamp at all.
 *
 * An unknown *current* stamp is the opposite case and does count as moved: the
 * file was stamped once and now cannot be, which is a change worth asking about.
 * Being asked about a change that did not happen costs one decision; the other
 * error overwrites somebody's work without asking.
 */
export function hasMoved(base: FileStamp | null, current: FileStamp | null): boolean {
  if (!base) return false
  if (!current) return true
  return base.size !== current.size || base.mtimeMs !== current.mtimeMs
}
