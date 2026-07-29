/**
 * Read a finished command's rows out of the terminal buffer as styled runs.
 *
 * This is the piece that lets a block be rendered as real DOM instead of
 * being faked inside the character grid. xterm keeps owning the live command
 * and any full-screen application; once a command finishes, its rows are
 * extracted here and drawn as an ordinary element — which is what makes
 * padding, rounded corners, folding and per-block copy possible at all, and
 * what removes the need to mutate the user's shell to approximate them.
 *
 * Cells are merged into runs of identical styling, so a row becomes a handful
 * of spans rather than one per column.
 *
 * Everything here works against a minimal buffer surface rather than xterm
 * directly, so it can be tested without a terminal.
 */

/** Colour as the buffer reports it; mapping to a real value needs the theme. */
export type CellColor =
  | { kind: 'default' }
  | { kind: 'palette'; index: number }
  | { kind: 'rgb'; value: number }

export interface RunStyle {
  fg: CellColor
  bg: CellColor
  bold: boolean
  italic: boolean
  dim: boolean
  underline: boolean
  strikethrough: boolean
  /** Terminals render this by swapping fg and bg at draw time. */
  inverse: boolean
}

export interface StyledRun extends RunStyle {
  text: string
}

export interface BlockRow {
  runs: StyledRun[]
}

/** The subset of xterm's IBufferCell this needs. */
export interface CellLike {
  getChars(): string
  getWidth(): number
  getFgColor(): number
  getBgColor(): number
  isFgDefault(): boolean
  isBgDefault(): boolean
  isFgRGB(): boolean
  isBgRGB(): boolean
  isBold(): number
  isItalic(): number
  isDim(): number
  isUnderline(): number
  isStrikethrough(): number
  isInverse(): number
}

export interface LineLike {
  readonly length: number
  getCell(x: number, cell?: CellLike): CellLike | undefined
}

export interface BufferLike {
  getLine(y: number): LineLike | undefined
}

function fgOf(cell: CellLike): CellColor {
  if (cell.isFgDefault()) return { kind: 'default' }
  if (cell.isFgRGB()) return { kind: 'rgb', value: cell.getFgColor() }
  return { kind: 'palette', index: cell.getFgColor() }
}

function bgOf(cell: CellLike): CellColor {
  if (cell.isBgDefault()) return { kind: 'default' }
  if (cell.isBgRGB()) return { kind: 'rgb', value: cell.getBgColor() }
  return { kind: 'palette', index: cell.getBgColor() }
}

function styleOf(cell: CellLike): RunStyle {
  return {
    fg: fgOf(cell),
    bg: bgOf(cell),
    bold: cell.isBold() !== 0,
    italic: cell.isItalic() !== 0,
    dim: cell.isDim() !== 0,
    underline: cell.isUnderline() !== 0,
    strikethrough: cell.isStrikethrough() !== 0,
    inverse: cell.isInverse() !== 0
  }
}

function sameColor(a: CellColor, b: CellColor): boolean {
  if (a.kind !== b.kind) return false
  if (a.kind === 'palette' && b.kind === 'palette') return a.index === b.index
  if (a.kind === 'rgb' && b.kind === 'rgb') return a.value === b.value
  return true
}

export function sameStyle(a: RunStyle, b: RunStyle): boolean {
  return (
    a.bold === b.bold &&
    a.italic === b.italic &&
    a.dim === b.dim &&
    a.underline === b.underline &&
    a.strikethrough === b.strikethrough &&
    a.inverse === b.inverse &&
    sameColor(a.fg, b.fg) &&
    sameColor(a.bg, b.bg)
  )
}

/** Trailing blanks are padding in the grid, not content worth rendering. */
function trimTrailingBlank(runs: StyledRun[]): StyledRun[] {
  for (let i = runs.length - 1; i >= 0; i--) {
    const run = runs[i]
    // A run carrying a background is a visible band, not padding — leave it
    // exactly as the terminal drew it, whitespace included.
    const isPaint = run.bg.kind !== 'default' || run.inverse
    if (isPaint) break

    const trimmed = run.text.replace(/\s+$/, '')
    if (trimmed === '') {
      runs.pop()
      continue
    }
    runs[i] = { ...run, text: trimmed }
    break
  }
  return runs
}

export function extractRow(line: LineLike): BlockRow {
  const runs: StyledRun[] = []
  let current: StyledRun | null = null

  for (let x = 0; x < line.length; x++) {
    const cell = line.getCell(x)
    if (!cell) continue
    // Width 0 is the trailing half of a wide character; its glyph already
    // came from the preceding cell.
    if (cell.getWidth() === 0) continue

    const chars = cell.getChars()
    const text = chars === '' ? ' ' : chars
    const style = styleOf(cell)

    if (current && sameStyle(current, style)) {
      current.text += text
    } else {
      current = { ...style, text }
      runs.push(current)
    }
  }

  return { runs: trimTrailingBlank(runs) }
}

/**
 * Rows `startLine` through `endLine` inclusive, in absolute buffer
 * coordinates — the same coordinates the command tracker's markers use.
 */
export function extractBlock(buffer: BufferLike, startLine: number, endLine: number): BlockRow[] {
  const rows: BlockRow[] = []
  for (let y = startLine; y <= endLine; y++) {
    const line = buffer.getLine(y)
    if (!line) continue
    rows.push(extractRow(line))
  }
  return rows
}

/** Plain text of a block, for copy-to-clipboard. */
export function blockToText(rows: BlockRow[]): string {
  return rows.map((r) => r.runs.map((run) => run.text).join('')).join('\n')
}
