import type { CommandBlock } from './command-blocks'

/**
 * Command spine geometry.
 *
 * The spine is a narrow gutter beside the terminal: one mark per command,
 * aligned to the rows that command occupies on screen. Structure lives here
 * rather than in the terminal output, which costs horizontal space instead
 * of vertical — the scarce dimension in a grid card.
 *
 * Everything in this module is pure so it can be tested without xterm.
 */

export interface BufferMetrics {
  /** Total buffer rows, i.e. baseY + rows. */
  length: number
  /** First visible row. */
  viewportY: number
  /** First row of the viewport when scrolled to the bottom. */
  baseY: number
  /** Visible row count. */
  rows: number
  /**
   * Absolute row the cursor sits on — the end of everything written so far.
   * Bounds the last block: `length` pads out to a full viewport, so measuring
   * against it would stretch that block through rows holding nothing.
   */
  cursorLine: number
  /** True while a full-screen application owns the screen. */
  isAlternate: boolean
}

export type SpineStatus = 'ok' | 'fail' | 'running'

export interface SpineMark {
  key: string
  /** Buffer line to scroll to when the mark is activated. */
  line: number
  /** Last buffer row belonging to this block, for highlighting it. */
  endLine: number
  /** Vertical offset in pixels from the top of the spine. */
  y: number
  /**
   * Pixel height of the block this mark stands for, measured to wherever the
   * next one begins. A bar spanning the block reads as its extent; a dot
   * would only say "something happened here".
   */
  height: number
  /** Commands folded into this mark; > 1 when neighbours were clustered. */
  count: number
  status: SpineStatus
  command: string | null
  exitCode: number
  durationMs: number
  /** Succeeded quickly and quietly — drawn inset rather than full width. */
  routine: boolean
}

/**
 * Gutter width and the gap between it and the terminal text. Single source of
 * truth: the component styles itself from these, and anything aligning to the
 * terminal's text column derives its inset from them.
 */
export const SPINE_WIDTH_PX = 8
export const SPINE_GAP_PX = 8

/** Marks closer together than this are collapsed into one. */
export const MIN_GAP_PX = 4

/** Shortest a block bar may be drawn, so a one-line command stays visible. */
export const MIN_MARK_PX = 6

/** Breathing room between adjacent block bars. */
const MARK_GAP_PX = 2

const ROUTINE_MAX_MS = 2000
const ROUTINE_MAX_LINES = 5

/** Worst-first, so a cluster reports the most serious thing inside it. */
const STATUS_RANK: Record<SpineStatus, number> = { fail: 2, running: 1, ok: 0 }

/**
 * A command that succeeded quickly with little to say. These are the bulk of
 * a session — `cd`, `ls`, `git status` — and the spine demotes them so the
 * commands worth noticing stay findable.
 */
export function isRoutine(
  block: Pick<CommandBlock, 'exitCode' | 'durationMs' | 'outputLines'>
): boolean {
  return (
    block.exitCode === 0 &&
    block.durationMs <= ROUTINE_MAX_MS &&
    block.outputLines <= ROUTINE_MAX_LINES
  )
}

export interface RunningMark {
  command: string | null
  since: number
  line: number
}

/**
 * Map blocks onto pixel offsets within a spine of `heightPx`.
 *
 * The spine is aligned to the viewport, not to the whole session: a mark sits
 * beside the rows it stands for, so the gutter and the output read as the
 * same thing. Compressing the entire session into the gutter instead would
 * put marks next to unrelated text, and make hovering one highlight rows
 * somewhere else entirely.
 *
 * Blocks scrolled out of view are dropped; the ones straddling an edge are
 * clipped to it.
 *
 * Returns [] on the alternate buffer: markers reference the normal buffer
 * and would point at unrelated rows while a full-screen app is up.
 */
export function computeSpineMarks(
  blocks: CommandBlock[],
  metrics: BufferMetrics,
  heightPx: number,
  running?: RunningMark | null,
  now = 0
): SpineMark[] {
  if (metrics.isAlternate || heightPx <= 0 || metrics.rows <= 0) return []

  const rowPx = heightPx / metrics.rows
  const viewTop = metrics.viewportY
  const viewBottom = metrics.viewportY + metrics.rows

  // Ordered starts, so each block can be measured to the next one.
  const starts: Array<{ line: number; block?: CommandBlock; run?: RunningMark }> = []
  for (const block of blocks) {
    if (block.marker.isDisposed) continue
    starts.push({ line: block.marker.line, block })
  }
  if (running) starts.push({ line: running.line, run: running })
  starts.sort((a, b) => a.line - b.line)

  const placed: SpineMark[] = []
  for (let i = 0; i < starts.length; i++) {
    const start = starts[i]
    const next = starts[i + 1]
    // A block owns every row up to the one that starts the next block.
    const endLine = next ? Math.max(start.line, next.line - 1) : metrics.cursorLine
    if (endLine < viewTop || start.line >= viewBottom) continue

    const topRow = Math.max(start.line, viewTop)
    const bottomRow = Math.min(endLine, viewBottom - 1)
    const y = (topRow - viewTop) * rowPx
    const height = Math.max(MIN_MARK_PX, (bottomRow - topRow + 1) * rowPx - MARK_GAP_PX)

    if (start.run) {
      placed.push({
        key: `r-${start.line}`,
        line: start.line,
        endLine,
        y,
        height,
        count: 1,
        status: 'running',
        command: start.run.command,
        exitCode: 0,
        durationMs: Math.max(0, now - start.run.since),
        routine: false
      })
      continue
    }

    const block = start.block as CommandBlock
    placed.push({
      key: `b-${start.line}-${block.command ?? ''}`,
      line: start.line,
      endLine,
      y,
      height,
      count: 1,
      status: block.exitCode === 0 ? 'ok' : 'fail',
      command: block.command,
      exitCode: block.exitCode,
      durationMs: block.durationMs,
      routine: isRoutine(block)
    })
  }

  return clusterMarks(placed)
}

/**
 * Collapse marks nearer than MIN_GAP_PX. Without this a long session becomes
 * a solid bar in a short card. The cluster reports the worst status and keeps
 * that member's command, so a failure is never hidden behind its neighbours.
 */
function clusterMarks(sorted: SpineMark[]): SpineMark[] {
  const out: SpineMark[] = []
  for (const mark of sorted) {
    const last = out[out.length - 1]
    if (last && mark.y - last.y < MIN_GAP_PX) {
      last.count += 1
      // The merged bar has to cover both, or hovering it would highlight
      // less than it stands for.
      last.height = Math.max(last.height, mark.y + mark.height - last.y)
      last.endLine = Math.max(last.endLine, mark.endLine)
      if (STATUS_RANK[mark.status] > STATUS_RANK[last.status]) {
        last.status = mark.status
        last.line = mark.line
        last.command = mark.command
        last.exitCode = mark.exitCode
        last.durationMs = mark.durationMs
      }
      // A cluster is only routine when every member is.
      last.routine = last.routine && mark.routine
      continue
    }
    out.push({ ...mark })
  }
  return out
}
