import * as headless from '@xterm/headless'

/**
 * What a terminal shows, read from a headless xterm: the client's own
 * emulator, which is where a restored screen is drawn. Comparing what xterm
 * shows after replaying a serialized screen, rather than the escape sequences
 * themselves, does not depend on two formatters choosing the same sequences.
 */

function interop<T>(mod: unknown): T {
  return (mod as { default?: T })?.default ?? (mod as T)
}
const { Terminal } = interop<typeof import('@xterm/headless')>(headless)
type Term = InstanceType<typeof Terminal>

export interface View {
  rows: string[]
  cursor: [number, number]
  /** One entry per cell that is not an unstyled blank: position, colours, attributes, text. */
  styles: string[]
  alternate: boolean
}

function write(term: Term, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve))
}

/**
 * `PALETTE_AS_256`: a palette colour is compared by its index, whether it was
 * set as a 16-colour or a 256-colour code, so `32` and `38;5;2` are equal.
 */
function colour(isDefault: boolean, isRgb: boolean, value: number): string {
  if (isDefault) return 'default'
  if (isRgb) return `rgb:${value}`
  return `palette:${value}`
}

function view(term: Term): View {
  const buf = term.buffer.active
  const rows: string[] = []
  const styles: string[] = []
  for (let y = 0; y < term.rows; y++) {
    const line = buf.getLine(y)
    // BLANKS_AS_SPACES: trailing blanks are compared as cells, below, not as text.
    rows.push((line?.translateToString(true) ?? '').trimEnd())
    for (let x = 0; x < term.cols; x++) {
      const cell = line?.getCell(x)
      if (!cell) continue
      const style = [
        `${x},${y}`,
        colour(cell.isFgDefault(), cell.isFgRGB(), cell.getFgColor()),
        colour(cell.isBgDefault(), cell.isBgRGB(), cell.getBgColor()),
        cell.isBold(),
        cell.isItalic(),
        cell.isUnderline(),
        cell.isInverse(),
        cell.isDim(),
        cell.isStrikethrough(),
        cell.isBlink(),
        cell.isInvisible(),
        cell.isOverline(),
        cell.getWidth()
      ].join(' ')
      // BLANKS_AS_SPACES: an unstyled space is the same as an empty cell. A blank
      // with a style (a background, say) is visible, so it is compared.
      const blank = cell.getChars() === '' || cell.getChars() === ' '
      if (blank && style === `${x},${y} default default 0 0 0 0 0 0 0 0 0 1`) continue
      styles.push(`${style} ${blank ? ' ' : cell.getChars()}`)
    }
  }
  return { rows, cursor: [buf.cursorX, buf.cursorY], styles, alternate: buf.type === 'alternate' }
}

/** Replay a serialized screen into a fresh xterm of that size and read what it shows. */
export async function replayView(serialized: string, cols: number, rows: number): Promise<View> {
  const term = new Terminal({ cols, rows, scrollback: 0, allowProposedApi: true })
  await write(term, serialized)
  const v = view(term)
  term.dispose()
  return v
}
