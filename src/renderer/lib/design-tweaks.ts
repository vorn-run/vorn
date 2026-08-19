/**
 * Your adjustments to a design, kept beside the file rather than inside it.
 *
 * A design declares defaults so it travels — opened anywhere, it renders the way
 * its author meant. What you turn while looking at it is a different thing: it
 * is scratch state about one file on one machine, and writing it back into the
 * file would put it in the diff and let the agent's next write clobber it.
 *
 * Keeping it here is also what makes a repaint survivable. The pane reloads a
 * design whenever the file changes, and without somewhere to put these the loop
 * would reset every value the moment the agent touched anything — punishing you
 * for the agent's turn.
 *
 * localStorage rather than the database because these are local overrides, not
 * project facts. If they ever need to be agent-writable or to travel in host
 * mode, the target is the `defaults` KV table reached through `config:load` —
 * and note that loading there is an explicit allowlist, so a key missing from it
 * round-trips to nothing and its feature is silently inert.
 */

const STORAGE_KEY = 'vorn:designTweaks'

/** Values a control can hold, matching the four declared tweak types. */
type TweakValue = string | number | boolean

/** Overrides for one file, keyed by tweak name. */
export type TweakOverrides = Record<string, TweakValue>

/**
 * Bound on how many files we remember.
 *
 * Keyed by absolute path, this grows every time a design is opened and never
 * shrinks on its own — a file deleted or renamed leaves an entry nothing will
 * ever read again. The renderer cannot stat a path to check, so age is the only
 * signal available: the least recently touched entries go first.
 */
const MAX_FILES = 200

interface Stored {
  /** Last touched, so eviction has something to sort by. */
  at: number
  values: TweakOverrides
}

function isTweakValue(v: unknown): v is TweakValue {
  return (
    typeof v === 'boolean' || typeof v === 'string' || (typeof v === 'number' && Number.isFinite(v))
  )
}

/**
 * Read the whole map, dropping anything that no longer makes sense.
 *
 * Sanitised on read rather than trusted: this is a file a person can edit, and a
 * corrupt entry would otherwise reach a control as a value of the wrong type —
 * which the design then does arithmetic on.
 */
function loadAll(): Record<string, Stored> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as Record<string, unknown>
    const out: Record<string, Stored> = {}
    for (const [path, entry] of Object.entries(parsed)) {
      if (!entry || typeof entry !== 'object') continue
      const e = entry as { at?: unknown; values?: unknown }
      if (!e.values || typeof e.values !== 'object') continue
      const values: TweakOverrides = {}
      for (const [key, value] of Object.entries(e.values as Record<string, unknown>)) {
        if (isTweakValue(value)) values[key] = value
      }
      if (Object.keys(values).length === 0) continue
      out[path] = { at: typeof e.at === 'number' && Number.isFinite(e.at) ? e.at : 0, values }
    }
    return out
  } catch {
    return {}
  }
}

function saveAll(all: Record<string, Stored>): void {
  try {
    const entries = Object.entries(all)
    // Newest first, then cut. Sorting only when over budget keeps the common
    // write cheap.
    const kept =
      entries.length <= MAX_FILES
        ? entries
        : entries.sort(([, a], [, b]) => b.at - a.at).slice(0, MAX_FILES)
    localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(kept)))
  } catch {
    /* ignore */
  }
}

/** What you last set for this file, or nothing. */
export function loadTweaks(path: string): TweakOverrides {
  return loadAll()[path]?.values ?? {}
}

/**
 * Record one adjustment.
 *
 * Read-modify-write on every change, which is what the other localStorage
 * helpers do — the map is small and a control is turned at human speed. The
 * caller debounces a dragged slider; this does not try to.
 */
export function saveTweak(path: string, key: string, value: unknown): void {
  if (!path || !isTweakValue(value)) return
  const all = loadAll()
  const existing = all[path]?.values ?? {}
  all[path] = { at: Date.now(), values: { ...existing, [key]: value } }
  saveAll(all)
}

/**
 * The values a design should open with.
 *
 * Three layers, widest to narrowest: what the design declared, then whatever
 * the page itself holds, then what you set. Yours has to come last — a freshly
 * loaded page reports its own defaults, and letting those win would undo the
 * adjustment on every repaint, which is the thing this module exists to stop.
 *
 * All three resolved here rather than patched afterwards by the caller: split
 * across two places, the ordering is something a reader has to reassemble, and
 * it was already got wrong once that way.
 *
 * Only names the manifest still declares survive. A design that dropped a tweak
 * has no control for it and no code reading it, so carrying the old value
 * forward would keep it alive in storage forever with nothing to spend it on.
 */
export function mergeTweaks(
  declared: Record<string, { default: TweakValue; options?: string[] }> | undefined,
  stored: TweakOverrides,
  live?: Record<string, unknown>
): TweakOverrides {
  if (!declared) return {}
  const out: TweakOverrides = {}
  for (const [key, decl] of Object.entries(declared)) {
    const usable = (v: unknown): v is TweakValue => {
      // Types must agree. A design that changed `plan` from a number to a
      // select would otherwise be handed the old number as its selected option.
      if (v === undefined || typeof v !== typeof decl.default) return false
      // A non-finite number is the wrong kind of number: it would render as
      // "NaN" in the control and be done arithmetic with by the design.
      if (typeof v === 'number' && !Number.isFinite(v)) return false
      // And a value has to still be on offer. If the design dropped an option
      // you had selected, keeping it would push a value into the page the
      // control cannot display — the bar showing one thing while the design
      // renders another, with no event to reconcile them.
      return !decl.options || (typeof v === 'string' && decl.options.includes(v))
    }
    const fromPage = live ? live[key] : undefined
    out[key] = usable(stored[key]) ? stored[key] : usable(fromPage) ? fromPage : decl.default
  }
  return out
}

/** Only for tests, which must not inherit state across cases. */
export function resetTweaks(): void {
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    /* ignore */
  }
}
