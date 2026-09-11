import type { LibraryPick } from './library-pick'

const RECENT_KEY = 'vorn:recentSteps'
const OPEN_KEY = 'vorn:stepLibraryOpen'
/** How many picked actions the library offers again at its top. */
const KEPT_RECENT = 5

/** An action someone picked, the kind of pick worth offering again. */
export type RecentPick = Extract<LibraryPick, { kind: 'connectorAction' | 'catalogAction' }>

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

function isRecentPick(value: unknown): value is RecentPick {
  const pick = value as Record<string, unknown> | null
  if (!pick || typeof pick.action !== 'string' || typeof pick.actionLabel !== 'string') return false
  return (
    (pick.kind === 'connectorAction' && typeof pick.connectionId === 'string') ||
    (pick.kind === 'catalogAction' && typeof pick.connectorId === 'string')
  )
}

/** The row an action is listed under, the same in Recent as in its group. */
export function actionKey(pick: RecentPick): string {
  return pick.kind === 'connectorAction'
    ? `action:${pick.connectionId}:${pick.action}`
    : `catalog:${pick.connectorId}:${pick.action}`
}

/** The actions picked most recently, newest first. */
export function readRecentPicks(): RecentPick[] {
  const stored = readJson(RECENT_KEY)
  return Array.isArray(stored) ? stored.filter(isRecentPick).slice(0, KEPT_RECENT) : []
}

export function recordRecentPick(pick: RecentPick): void {
  const key = actionKey(pick)
  const rest = readRecentPicks().filter((p) => actionKey(p) !== key)
  writeJson(RECENT_KEY, [pick, ...rest].slice(0, KEPT_RECENT))
}

/** The groups left open, so the library opens the way it was left. */
export function readOpenGroups(): Set<string> {
  const stored = readJson(OPEN_KEY)
  return new Set(Array.isArray(stored) ? stored.filter((k) => typeof k === 'string') : [])
}

export function writeOpenGroups(open: ReadonlySet<string>): void {
  writeJson(OPEN_KEY, [...open])
}
