import { extractBlock, type BlockRow, type BufferLike, type CellColor } from './block-render'

/**
 * Finished commands, captured out of the terminal buffer and kept as styled
 * rows for the renderer to draw as ordinary DOM.
 *
 * This is the other half of the block model: xterm keeps the live command and
 * any full-screen application, and everything already finished lives here,
 * where padding, rounded corners, folding and per-block copy are just CSS.
 */

export interface LoggedBlock {
  id: string
  command: string | null
  exitCode: number
  durationMs: number
  cwd: string | null
  rows: BlockRow[]
}

/** Keeps memory bounded; the buffer itself is capped at 2000 rows anyway. */
const MAX_BLOCKS = 100

/**
 * Terminals whose view is currently drawing the log.
 *
 * Lifting rows out of the buffer clears the terminal, so it must only happen
 * where something is rendering them — otherwise the output is captured into a
 * log nobody shows and the terminal is left blank.
 */
const mountedViews = new Set<string>()

export function registerBlockLogView(terminalId: string): () => void {
  mountedViews.add(terminalId)
  return () => {
    mountedViews.delete(terminalId)
  }
}

export function hasBlockLogView(terminalId: string): boolean {
  return mountedViews.has(terminalId)
}

const logs = new Map<string, LoggedBlock[]>()
const listeners = new Map<string, Set<() => void>>()
let nextId = 0

export function onBlockLogChange(terminalId: string, cb: () => void): () => void {
  let set = listeners.get(terminalId)
  if (!set) {
    set = new Set()
    listeners.set(terminalId, set)
  }
  set.add(cb)
  return () => {
    const current = listeners.get(terminalId)
    current?.delete(cb)
    if (current && current.size === 0) listeners.delete(terminalId)
  }
}

function emit(terminalId: string): void {
  const set = listeners.get(terminalId)
  if (!set) return
  for (const cb of set) cb()
}

/**
 * Shared so an empty log is reference-stable: the snapshot is compared by
 * identity, and a fresh [] every call would loop forever.
 */
const EMPTY: LoggedBlock[] = []

export function getBlockLog(terminalId: string): LoggedBlock[] {
  return logs.get(terminalId) ?? EMPTY
}

export function clearBlockLog(terminalId: string): void {
  logs.delete(terminalId)
  emit(terminalId)
}

function isBlankRow(row: BlockRow): boolean {
  return row.runs.every((r) => r.text.trim() === '' && r.bg.kind === 'default' && !r.inverse)
}

/** The shell prints blank rows for spacing; the DOM block supplies its own. */
function trimBlankEdges(rows: BlockRow[]): BlockRow[] {
  let start = 0
  let end = rows.length - 1
  while (start <= end && isBlankRow(rows[start])) start++
  while (end >= start && isBlankRow(rows[end])) end--
  return rows.slice(start, end + 1)
}

export interface CaptureInput {
  terminalId: string
  buffer: BufferLike
  /** Row the command was typed on. */
  startLine: number
  /** Last row of its output. */
  endLine: number
  command: string | null
  exitCode: number
  durationMs: number
  cwd: string | null
}

export function captureBlock(input: CaptureInput): LoggedBlock | null {
  const rows = trimBlankEdges(extractBlock(input.buffer, input.startLine, input.endLine))
  // A command that printed nothing and whose own line was consumed leaves no
  // rows worth a container.
  if (rows.length === 0 && !input.command) return null

  const block: LoggedBlock = {
    id: `blk-${nextId++}`,
    command: input.command,
    exitCode: input.exitCode,
    durationMs: input.durationMs,
    cwd: input.cwd,
    rows
  }
  // Replaced rather than mutated, so the snapshot's identity changes when the
  // contents do.
  const existing = logs.get(input.terminalId) ?? EMPTY
  const next = [...existing, block]
  logs.set(input.terminalId, next.slice(Math.max(0, next.length - MAX_BLOCKS)))
  emit(input.terminalId)
  return block
}

// --- colour resolution -------------------------------------------------

/**
 * xterm reports colours as a palette index or packed RGB. The 16 base slots
 * come from the terminal theme so blocks match the live terminal exactly;
 * 16-255 follow the standard 6x6x6 cube and greyscale ramp.
 */
const BASE_16 = [
  '#27272a',
  '#ef4444',
  '#22c55e',
  '#eab308',
  '#3b82f6',
  '#a855f7',
  '#06b6d4',
  '#d4d4d8',
  '#52525b',
  '#f87171',
  '#4ade80',
  '#facc15',
  '#60a5fa',
  '#c084fc',
  '#22d3ee',
  '#fafafa'
]

const CUBE = [0, 95, 135, 175, 215, 255]

function hex(n: number): string {
  return n.toString(16).padStart(2, '0')
}

export function paletteToCss(index: number): string {
  if (index < 16) return BASE_16[index] ?? '#d4d4d8'
  if (index < 232) {
    const i = index - 16
    const r = CUBE[Math.floor(i / 36) % 6]
    const g = CUBE[Math.floor(i / 6) % 6]
    const b = CUBE[i % 6]
    return `#${hex(r)}${hex(g)}${hex(b)}`
  }
  const level = 8 + (index - 232) * 10
  return `#${hex(level)}${hex(level)}${hex(level)}`
}

export function colorToCss(color: CellColor, fallback: string): string {
  if (color.kind === 'default') return fallback
  if (color.kind === 'rgb') {
    return `#${hex((color.value >> 16) & 0xff)}${hex((color.value >> 8) & 0xff)}${hex(color.value & 0xff)}`
  }
  return paletteToCss(color.index)
}
