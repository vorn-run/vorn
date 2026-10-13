const RECENT_KEY = 'vorn:recentSteps'
const OPEN_KEY = 'vorn:stepLibraryOpen'
/** How many picked actions the library offers again at its top. */
const KEPT_RECENT = 5

/** An action someone picked, known by its connector so it outlives the row it was picked from. */
export interface RecentAction {
  connectorId: string
  action: string
  /** The connection it went through, preferred while that connection is still there. */
  connectionId?: string
}

function readJson(key: string): unknown {
  try {
    return JSON.parse(localStorage.getItem(key) ?? 'null')
  } catch {
    return null
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* a full or blocked store only costs what the library remembers */
  }
}

function isRecentAction(value: unknown): value is RecentAction {
  const picked = value as Record<string, unknown> | null
  return (
    !!picked &&
    typeof picked.connectorId === 'string' &&
    typeof picked.action === 'string' &&
    (picked.connectionId === undefined || typeof picked.connectionId === 'string')
  )
}

/** The actions picked most recently, newest first. */
export function readRecentActions(): RecentAction[] {
  const stored = readJson(RECENT_KEY)
  return Array.isArray(stored) ? stored.filter(isRecentAction) : []
}

export function recordRecentAction(picked: RecentAction): void {
  const rest = readRecentActions().filter(
    (a) => a.connectorId !== picked.connectorId || a.action !== picked.action
  )
  writeJson(RECENT_KEY, [picked, ...rest].slice(0, KEPT_RECENT))
}

/** The groups left open, so the library opens the way it was left. */
export function readOpenGroups(): Set<string> {
  const stored = readJson(OPEN_KEY)
  return new Set(Array.isArray(stored) ? stored.filter((k) => typeof k === 'string') : [])
}

export function writeOpenGroups(open: ReadonlySet<string>): void {
  writeJson(OPEN_KEY, [...open])
}
