/**
 * Viewers fit, they never clip.
 *
 * A session vornd holds has one size, chosen by vornd for whichever client
 * is using it, and every other client draws that whole grid: the font scales
 * down until the grid fits the pane, to a floor of {@link MIN_FONT_PX}, and
 * whatever still does not fit is panned to rather than cut off. The pane
 * never resizes the grid itself.
 *
 * The same arithmetic as `fit` in `packages/core/crates/size`, which the
 * size rule's tests check in every state they pass through.
 */

/** The smallest font a pane scales a grid down to before it pans instead. */
export const MIN_FONT_PX = 9

export interface GridSize {
  cols: number
  rows: number
}

export interface GridFit {
  /** The font to draw at. */
  font: number
  /** Columns and rows on screen at that font, at most the grid's. */
  shown: GridSize
  /** Columns and rows panned to rather than shown at once. */
  pan: GridSize
}

/**
 * How to draw `grid` in a pane that holds `room` cells at the user's font
 * `font`, scaling no lower than `minFont`.
 *
 * The cell is taken to scale with the font, which a monospace font does to
 * within a pixel; the pane measures again once the font is applied.
 */
export function fitGrid(
  grid: GridSize,
  room: GridSize,
  font: number,
  minFont: number = MIN_FONT_PX
): GridFit {
  if (grid.cols <= 0 || grid.rows <= 0 || room.cols <= 0 || room.rows <= 0 || font <= 0) {
    return { font, shown: { ...grid }, pan: { cols: 0, rows: 0 } }
  }
  const scale = Math.min(room.cols / grid.cols, room.rows / grid.rows, 1)
  const scaled = Math.max(font * scale, Math.min(minFont, font))
  // The pane holds `room` cells of the user's font, so this many of the
  // scaled one; the epsilon keeps an exact fit from rounding down.
  const holds = (cells: number, of: number): number =>
    Math.min(of, Math.max(0, Math.floor((cells * font) / scaled + 1e-3)))
  const shown = { cols: holds(room.cols, grid.cols), rows: holds(room.rows, grid.rows) }
  return {
    font: scaled,
    shown,
    pan: { cols: grid.cols - shown.cols, rows: grid.rows - shown.rows }
  }
}
