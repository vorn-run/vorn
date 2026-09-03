import type { CommandBlock } from './command-blocks'

const STORAGE_KEY = 'vorn:scrollAnchors'

/**
 * Where a session was left reading, as a command rather than a row number.
 *
 * A row number does not survive the trip. The screen a pane comes back to is
 * replayed from the server and trimmed to a scrollback limit, so the same output
 * sits at a different row -- restoring row 812 lands somewhere arbitrary, and
 * looks like the app scrolled to a random place rather than failing.
 *
 * A command survives it, because the replay carries the same commands in the
 * same order. Counted from the newest, since that is the end the replay keeps
 * when it has to drop something, and checked against the command text so a
 * count that has gone stale is caught rather than trusted.
 */
export interface ScrollAnchor {
  /** 0 is the newest block. */
  fromEnd: number
  command: string | null
}

/** What `chooseAnchor` needs to know about the viewport. */
export interface AnchorMetrics {
  viewportY: number
  baseY: number
  isAlternate: boolean
}

type Anchors = Record<string, ScrollAnchor>

function load(): Anchors {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const out: Anchors = {}
    for (const [id, value] of Object.entries(parsed as Record<string, unknown>)) {
      const anchor = value as Partial<ScrollAnchor>
      if (typeof anchor?.fromEnd !== 'number' || !Number.isInteger(anchor.fromEnd)) continue
      if (anchor.fromEnd < 0) continue
      out[id] = {
        fromEnd: anchor.fromEnd,
        command: typeof anchor.command === 'string' ? anchor.command : null
      }
    }
    return out
  } catch {
    return {}
  }
}

function save(anchors: Anchors): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(anchors))
  } catch {
    /* ignore */
  }
}

export function readScrollAnchor(terminalId: string): ScrollAnchor | null {
  return load()[terminalId] ?? null
}

export function writeScrollAnchor(terminalId: string, anchor: ScrollAnchor | null): void {
  const anchors = load()
  if (anchor) anchors[terminalId] = anchor
  else if (!(terminalId in anchors)) return
  else delete anchors[terminalId]
  save(anchors)
}

/**
 * The block the top of the viewport is sitting in, or null at the bottom.
 *
 * Null is the answer for a pane that was left following the output, which is
 * most of them: there is nothing to anchor, and the bottom is where a terminal
 * opens anyway. Storing an anchor for it would scroll the pane up to the start
 * of the last command on every launch.
 */
export function chooseAnchor(blocks: CommandBlock[], metrics: AnchorMetrics): ScrollAnchor | null {
  if (metrics.isAlternate) return null
  if (metrics.viewportY >= metrics.baseY) return null
  for (let i = blocks.length - 1; i >= 0; i--) {
    const marker = blocks[i].marker
    if (!marker.isDisposed && marker.line <= metrics.viewportY) {
      return { fromEnd: blocks.length - 1 - i, command: blocks[i].command }
    }
  }
  return null
}

/**
 * The row an anchor names in this replay, or null when it names nothing here.
 *
 * Null covers the ordinary cases as well as the broken one: a session with no
 * shell integration has no blocks at all, and a count that has gone stale points
 * at a different command. Both mean the bottom, which is where the pane already
 * is -- so the caller does nothing rather than guessing at a row.
 */
export function resolveAnchor(blocks: CommandBlock[], anchor: ScrollAnchor | null): number | null {
  if (!anchor) return null
  const block = blocks[blocks.length - 1 - anchor.fromEnd]
  if (!block || block.command !== anchor.command || block.marker.isDisposed) return null
  return block.marker.line
}

/** Drop anchors for sessions that are gone, alongside the rest of the view state. */
export function pruneScrollAnchors(liveSessionIds: Set<string>): void {
  const anchors = load()
  const dead = Object.keys(anchors).filter((id) => !liveSessionIds.has(id))
  if (!dead.length) return
  for (const id of dead) delete anchors[id]
  save(anchors)
}
