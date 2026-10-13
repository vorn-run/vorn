import type { ArtifactArtboard } from '../../shared/types'

/** The smallest zoom a guest takes; Chromium clamps a page's zoom factor here. */
export const MIN_ZOOM = 0.25
export const MAX_ZOOM = 1
/** Space around and between artboards, in screen pixels. */
export const CANVAS_PAD = 24
/** Room above each artboard for its label. */
export const LABEL_HEIGHT = 20

export interface PlacedArtboard extends ArtifactArtboard {
  left: number
  top: number
}

/** Artboards side by side at a zoom, each with its label above it. */
export function placeArtboards(
  boards: ArtifactArtboard[],
  zoom: number
): { placed: PlacedArtboard[]; width: number; height: number } {
  let left = CANVAS_PAD
  let tallest = 0
  const placed = boards.map((b) => {
    const at = { ...b, left, top: CANVAS_PAD + LABEL_HEIGHT }
    left += Math.round(b.width * zoom) + CANVAS_PAD
    tallest = Math.max(tallest, Math.round(b.height * zoom))
    return at
  })
  return { placed, width: left, height: CANVAS_PAD * 2 + LABEL_HEIGHT + tallest }
}

export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(zoom * 100) / 100))
}

/** The zoom that fits every artboard in the area, no smaller than a guest can go. */
export function fitZoom(
  boards: ArtifactArtboard[],
  area: { width: number; height: number }
): number {
  if (boards.length === 0 || area.width <= 0 || area.height <= 0) return MAX_ZOOM
  const widths = boards.reduce((sum, b) => sum + b.width, 0)
  const tallest = Math.max(...boards.map((b) => b.height))
  const byWidth = (area.width - CANVAS_PAD * (boards.length + 1)) / widths
  const byHeight = (area.height - CANVAS_PAD * 2 - LABEL_HEIGHT) / tallest
  // Floored, so rounding never leaves the last artboard a pixel past the edge.
  return clampZoom(Math.floor(Math.min(byWidth, byHeight) * 100) / 100)
}

/** The same page, told which artboard it is drawing through its hash. */
export function artboardUrl(url: string, id: string): string {
  const hash = url.indexOf('#')
  return `${hash === -1 ? url : url.slice(0, hash)}#artboard=${encodeURIComponent(id)}`
}
